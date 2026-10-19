//! Catalog-wide commands that act on a set of videos chosen in the list.

use std::path::Path;

use tauri::State;

use crate::commands::dedup_cmds::{delete_thumbnail_files, thumbnails_root_from_app_dir};
use crate::db::Db;
use crate::events::{CatalogNotifier, TauriCatalogNotifier};

/// Removes `video_ids` from the catalog in one transaction (rows, tags, path
/// collisions) and then deletes their cached thumbnails. Returns how many
/// videos were removed; ids with no row are ignored.
///
/// **Never touches the source video files.** The only files removed are the
/// thumbnail cache entries under `thumbnails_root`.
///
/// The DB is changed first and the thumbnails only after it succeeded: if the
/// transaction fails, nothing was removed and the thumbnails must still be
/// there for the rows that remain. The reverse order would leave surviving
/// rows without thumbnails. Leftover thumbnail files after a successful
/// delete are harmless, so that step is best-effort.
///
/// `thumbnails_root` is `None` when the app data directory could not be
/// resolved; the thumbnail cleanup is then skipped.
pub fn remove_videos_from_catalog(
    db: &Db,
    thumbnails_root: Option<&Path>,
    video_ids: &[String],
) -> Result<u32, String> {
    // Looked up before the delete: once the rows are gone, `file_path` (which
    // decides the thumbnail subdirectory) is no longer recoverable.
    let targets: Vec<(&String, String)> = video_ids
        .iter()
        .filter_map(|id| {
            crate::db::queries::find_video_by_id(&db.read_pool, id)
                .ok()
                .flatten()
                .map(|row| (id, row.file_path))
        })
        .collect();

    let deleted = {
        let mut conn = db.writer.lock().unwrap();
        crate::db::queries::delete_videos_cascade(&mut conn, video_ids)
            .map_err(|e| e.to_string())?
    };

    if let Some(thumbnails_root) = thumbnails_root {
        let watch_folders = db
            .read_pool
            .get()
            .ok()
            .and_then(|conn| crate::db::queries::get_watch_folders(&conn).ok())
            .unwrap_or_default();
        for (id, file_path) in &targets {
            delete_thumbnail_files(thumbnails_root, &watch_folders, file_path, id);
        }
    }

    Ok(deleted as u32)
}

/// "Remove from catalog" for the selected videos. The source files stay on
/// disk, so a still-watched folder can register them again on the next scan.
// async: runs on the async runtime's thread pool rather than the main
// thread, since a large batch holds the writer lock for the whole delete.
#[tauri::command(async)]
pub fn unregister_videos(
    app: tauri::AppHandle,
    db: State<Db>,
    video_ids: Vec<String>,
) -> Result<u32, String> {
    let thumbnails_root = thumbnails_root_from_app_dir();
    let removed = remove_videos_from_catalog(&db, thumbnails_root.as_deref(), &video_ids)?;
    TauriCatalogNotifier::new(app).notify_changed();
    log::info!(
        "unregistered {removed} video(s) from the catalog ({} requested)",
        video_ids.len()
    );
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::init_temp_db;
    use std::path::PathBuf;

    struct Env {
        // Kept alive so the temp directory (DB, files) outlives the test.
        _dir: tempfile::TempDir,
        db: Db,
        folder: PathBuf,
        thumbs: PathBuf,
    }

    impl Env {
        fn new() -> Self {
            let (dir, db) = init_temp_db();
            let folder = dir.path().join("lib");
            std::fs::create_dir_all(&folder).unwrap();
            let thumbs = dir.path().join("thumbs");
            {
                let conn = db.writer.lock().unwrap();
                crate::db::queries::set_watch_folders(
                    &conn,
                    &[folder.to_string_lossy().into_owned()],
                )
                .unwrap();
            }
            Self {
                _dir: dir,
                db,
                folder,
                thumbs,
            }
        }

        /// A real file inside the registered folder plus its row.
        fn add_video(&self, id: &str) -> PathBuf {
            let path = self.folder.join(format!("{id}.mp4"));
            std::fs::write(&path, b"video bytes").unwrap();
            let conn = self.db.writer.lock().unwrap();
            conn.execute(
                "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status)
                 VALUES (?1, ?2, ?3, 11, 'h', 'online')",
                rusqlite::params![
                    id,
                    path.to_string_lossy(),
                    path.file_name().unwrap().to_string_lossy(),
                ],
            )
            .unwrap();
            path
        }

        fn add_tag(&self, id: &str) {
            let mut conn = self.db.writer.lock().unwrap();
            crate::db::queries::assign_tag_to_video(&mut conn, id, "tag").unwrap();
        }

        fn row_exists(&self, id: &str) -> bool {
            crate::db::queries::find_video_by_id(&self.db.read_pool, id)
                .unwrap()
                .is_some()
        }

        fn tag_count(&self, id: &str) -> i64 {
            let conn = self.db.read_pool.get().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM video_tags WHERE video_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
        }

        fn add_thumbs(&self, id: &str, path: &Path) -> Vec<PathBuf> {
            let dir = crate::thumbnail::paths::video_thumbnail_dir(
                &self.thumbs,
                &[self.folder.to_string_lossy().into_owned()],
                &path.to_string_lossy(),
            );
            std::fs::create_dir_all(&dir).unwrap();
            (0..crate::thumbnail::worker::THUMBNAILS_PER_VIDEO)
                .map(|i| {
                    let p = crate::thumbnail::paths::slot_path(&dir, id, i);
                    std::fs::write(&p, b"webp").unwrap();
                    p
                })
                .collect()
        }
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn removes_rows_tags_and_thumbnails_but_keeps_source_files() {
        let env = Env::new();
        let p1 = env.add_video("v1");
        let p2 = env.add_video("v2");
        let p3 = env.add_video("v3");
        for id in ["v1", "v2", "v3"] {
            env.add_tag(id);
        }
        let t1 = env.add_thumbs("v1", &p1);
        let t2 = env.add_thumbs("v2", &p2);
        let t3 = env.add_thumbs("v3", &p3);

        let removed =
            remove_videos_from_catalog(&env.db, Some(&env.thumbs), &ids(&["v1", "v2"])).unwrap();

        assert_eq!(removed, 2);
        assert!(!env.row_exists("v1") && !env.row_exists("v2"));
        assert_eq!(env.tag_count("v1") + env.tag_count("v2"), 0);
        assert!(t1.iter().chain(&t2).all(|p| !p.exists()));
        // Source files are untouched.
        assert!(p1.exists() && p2.exists() && p3.exists());
        assert_eq!(std::fs::read(&p1).unwrap(), b"video bytes");
        // The unselected video keeps its row, tag, and thumbnails.
        assert!(env.row_exists("v3"));
        assert_eq!(env.tag_count("v3"), 1);
        assert!(t3.iter().all(|p| p.exists()));
    }

    #[test]
    fn succeeds_without_thumbnails_or_thumbnails_root() {
        let env = Env::new();
        let p1 = env.add_video("v1");
        let p2 = env.add_video("v2");

        assert_eq!(
            remove_videos_from_catalog(&env.db, Some(&env.thumbs), &ids(&["v1"])).unwrap(),
            1
        );
        assert_eq!(
            remove_videos_from_catalog(&env.db, None, &ids(&["v2"])).unwrap(),
            1
        );
        assert!(!env.row_exists("v1") && !env.row_exists("v2"));
        assert!(p1.exists() && p2.exists());
    }

    #[test]
    fn unknown_ids_and_empty_input_are_ignored() {
        let env = Env::new();
        env.add_video("v1");

        assert_eq!(
            remove_videos_from_catalog(&env.db, Some(&env.thumbs), &[]).unwrap(),
            0
        );
        assert_eq!(
            remove_videos_from_catalog(&env.db, Some(&env.thumbs), &ids(&["nope", "v1"])).unwrap(),
            1
        );
        assert!(!env.row_exists("v1"));
    }

    #[test]
    fn db_failure_removes_nothing_including_thumbnails() {
        let env = Env::new();
        let p1 = env.add_video("v1");
        let p2 = env.add_video("v2");
        let t1 = env.add_thumbs("v1", &p1);
        let t2 = env.add_thumbs("v2", &p2);
        {
            // Fails the second video's row delete after the first was
            // already deleted inside the same transaction.
            let conn = env.db.writer.lock().unwrap();
            conn.execute_batch(
                "CREATE TRIGGER block_v2 BEFORE DELETE ON videos WHEN OLD.id = 'v2'
                 BEGIN SELECT RAISE(ABORT, 'blocked'); END;",
            )
            .unwrap();
        }

        let result = remove_videos_from_catalog(&env.db, Some(&env.thumbs), &ids(&["v1", "v2"]));

        assert!(result.is_err());
        assert!(env.row_exists("v1") && env.row_exists("v2"));
        assert!(t1.iter().chain(&t2).all(|p| p.exists()));
        assert!(p1.exists() && p2.exists());
    }
}
