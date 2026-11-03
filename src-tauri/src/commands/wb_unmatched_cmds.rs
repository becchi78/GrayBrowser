//! Legacy thumbnails the `.wb` import could not attach to a video: listing
//! them, finding offline videos to attach them to, and attaching by hand. The
//! `#[tauri::command]` wrappers are thin; the logic lives in plain functions
//! over `&Db` and fakeable ports so it can be tested without a Tauri runtime.

use std::path::Path;

use gb_core::ports::ffmpeg::FfmpegAdapter;
use gb_core::wb_import::default_candidate_search_term;
use gb_core::wb_thumbnail_split::parse_thum_size;
use serde::Serialize;
use tauri::State;

use crate::adapters;
use crate::adapters::long_path;
use crate::db::{queries, Db};
use crate::events::{CatalogNotifier, TauriCatalogNotifier};
use crate::wb_import::pipeline::{convert_thumbnail_to_slots, ItemResult};

/// Most candidates one search may return, whatever the caller asks for: the
/// list is rendered in full, so an unbounded request would stall the UI.
const MAX_CANDIDATES: u32 = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnmatchedThumbnail {
    pub folder_path: String,
    pub file_name: String,
    /// `empty_hash` or `no_matching_video`.
    pub kind: String,
    /// What the search box should start with; `None` when the file name
    /// carries no usable video name.
    pub default_search_term: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkCandidate {
    pub id: String,
    pub file_name: String,
    pub file_path: String,
    /// Whether the video already has thumbnails, which linking replaces.
    pub thumbnail_ready: bool,
}

fn list_impl(db: &Db) -> anyhow::Result<Vec<UnmatchedThumbnail>> {
    let conn = db.read_pool.get()?;
    Ok(queries::list_unmatched_thumbnails(&conn)?
        .into_iter()
        .map(|row| UnmatchedThumbnail {
            default_search_term: default_candidate_search_term(&row.file_name),
            folder_path: row.folder_path,
            file_name: row.file_name,
            kind: row.kind,
        })
        .collect())
}

fn search_impl(db: &Db, term: &str, limit: u32) -> anyhow::Result<Vec<LinkCandidate>> {
    let conn = db.read_pool.get()?;
    Ok(
        queries::search_offline_videos(&conn, term.trim(), limit.min(MAX_CANDIDATES))?
            .into_iter()
            .map(|c| LinkCandidate {
                id: c.id,
                file_name: c.file_name,
                file_path: c.file_path,
                thumbnail_ready: c.thumbnail_ready,
            })
            .collect(),
    )
}

/// Attaches the recorded legacy thumbnail `(folder_path, file_name)` to an
/// offline video, replacing any thumbnails it has.
///
/// The source file is always the recorded folder joined with the recorded
/// name: the caller only names a record, never a path, so this cannot be used
/// to read an arbitrary file. On failure the record stays and no slot or
/// scratch file is left behind; the `Err` text is meant for the user.
fn link_impl<F: FfmpegAdapter, C: CatalogNotifier>(
    db: &Db,
    ffmpeg: &F,
    notifier: &C,
    thumbnails_root: &Path,
    folder_path: &str,
    file_name: &str,
    video_id: &str,
) -> Result<(), String> {
    let record = {
        let conn = db.read_pool.get().map_err(db_err)?;
        queries::list_unmatched_thumbnails(&conn).map_err(db_err)?
    }
    .into_iter()
    .find(|r| r.folder_path == folder_path && r.file_name == file_name)
    .ok_or("この旧サムネイルは一覧にありません。")?;

    let video = queries::find_video_by_id(&db.read_pool, video_id)
        .map_err(db_err)?
        .ok_or("紐付け先の動画が見つかりません。")?;
    if video.status != "offline" {
        return Err("オフラインの動画にだけ紐付けできます。".to_string());
    }

    let src_path = Path::new(&record.folder_path).join(&record.file_name);
    if std::fs::metadata(long_path::to_long_path(&src_path)).is_err() {
        return Err("旧サムネイルのファイルが見つかりません。".to_string());
    }

    // A stored value that no longer parses is treated as "no layout" (import
    // the image whole), the same as when the .wb stated none.
    let layout = record.thum_layout.as_deref().and_then(parse_thum_size);
    let watch_folders = {
        let conn = db.read_pool.get().map_err(db_err)?;
        queries::get_watch_folders(&conn).map_err(db_err)?
    };

    match convert_thumbnail_to_slots(
        ffmpeg,
        thumbnails_root,
        &watch_folders,
        &video.id,
        &video.file_path,
        &src_path,
        layout.as_ref(),
    ) {
        ItemResult::Linked { .. } => {}
        ItemResult::Failed(reason) => {
            // The failed attempt removed the video's slot files, including
            // any it had before; the flag must not keep claiming them.
            if video.thumbnail_ready {
                let conn = db.writer.lock().unwrap();
                if let Err(e) = queries::clear_thumbnail_ready(&conn, &video.id) {
                    log::warn!("failed to clear thumbnail_ready of {}: {e}", video.id);
                }
                drop(conn);
                notifier.notify_changed();
            }
            return Err(format!("紐付けに失敗しました: {reason}"));
        }
    }

    // One transaction: the flag and the record change together, so a failure
    // cannot leave a linked video still listed as unmatched.
    {
        let conn = db.writer.lock().unwrap();
        let tx = conn.unchecked_transaction().map_err(db_err)?;
        queries::mark_thumbnail_ready(&tx, &video.id).map_err(db_err)?;
        queries::delete_unmatched_thumbnail(&tx, &record.folder_path, &record.file_name)
            .map_err(db_err)?;
        tx.commit().map_err(db_err)?;
    }
    notifier.notify_changed();
    Ok(())
}

fn dismiss_impl(db: &Db, folder_path: &str, file_name: &str) -> anyhow::Result<()> {
    let conn = db.writer.lock().unwrap();
    queries::delete_unmatched_thumbnail(&conn, folder_path, file_name)?;
    Ok(())
}

fn db_err(e: impl std::fmt::Display) -> String {
    format!("データベースの操作に失敗しました: {e}")
}

#[tauri::command]
pub fn list_unmatched_thumbnails(db: State<Db>) -> Result<Vec<UnmatchedThumbnail>, String> {
    list_impl(&db).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn search_link_candidates(
    db: State<Db>,
    term: String,
    limit: u32,
) -> Result<Vec<LinkCandidate>, String> {
    search_impl(&db, &term, limit).map_err(|e| e.to_string())
}

/// `async` so the ffmpeg run happens off the UI thread.
#[tauri::command(async)]
pub fn link_unmatched_thumbnail(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    folder_path: String,
    file_name: String,
    video_id: String,
) -> Result<(), String> {
    let thumbnails_root = crate::paths::app_data_dir()
        .map(|dir| crate::thumbnail::paths::thumbnails_root(&dir))
        .map_err(|e| e.to_string())?;
    link_impl(
        &db,
        &adapters::ffmpeg::RealFfmpegAdapter,
        &TauriCatalogNotifier::new(app),
        &thumbnails_root,
        &folder_path,
        &file_name,
        &video_id,
    )
}

#[tauri::command]
pub fn dismiss_unmatched_thumbnail(
    db: State<Db>,
    folder_path: String,
    file_name: String,
) -> Result<(), String> {
    dismiss_impl(&db, &folder_path, &file_name).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gb_core::ports::ffmpeg::{FfmpegError, VideoMetadata};
    use gb_core::testing::fake_ffmpeg::{FakeCall, FakeFfmpegAdapter};
    use std::path::PathBuf;

    use crate::db::queries::NewUnmatchedThumbnail;
    use crate::db::test_support::init_temp_db;
    use crate::events::FakeCatalogNotifier;

    const FOLDER_NAME: &str = "thumbs";
    const FILE: &str = "movie.mp4.#abc.jpg";

    struct Env {
        _db_dir: tempfile::TempDir,
        db: Db,
        _root_dir: tempfile::TempDir,
        root: PathBuf,
        folder: String,
        notifier: FakeCatalogNotifier,
    }

    impl Env {
        /// A recorded unmatched thumbnail `FILE` whose file exists on disk.
        fn new(layout: Option<&str>) -> Env {
            let (db_dir, db) = init_temp_db();
            let root_dir = tempfile::tempdir().unwrap();
            let folder_path = root_dir.path().join(FOLDER_NAME);
            std::fs::create_dir_all(&folder_path).unwrap();
            std::fs::write(folder_path.join(FILE), b"not really a jpeg").unwrap();
            let folder = folder_path.to_string_lossy().into_owned();
            {
                let mut conn = db.writer.lock().unwrap();
                queries::replace_unmatched_thumbnails(
                    &mut conn,
                    &folder,
                    &[NewUnmatchedThumbnail {
                        file_name: FILE.to_string(),
                        kind: "no_matching_video".to_string(),
                        thum_layout: layout.map(str::to_string),
                    }],
                )
                .unwrap();
            }
            Env {
                _db_dir: db_dir,
                db,
                root: root_dir.path().join("thumbnails"),
                _root_dir: root_dir,
                folder,
                notifier: FakeCatalogNotifier::default(),
            }
        }

        fn add_video(&self, id: &str, status: &str, ready: i64) {
            let conn = self.db.writer.lock().unwrap();
            conn.execute(
                "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status, \
                 thumbnail_ready) VALUES (?1, ?2, 'movie.mp4', 1, 'h', ?3, ?4)",
                rusqlite::params![id, format!("Z:\\v\\{id}.mp4"), status, ready],
            )
            .unwrap();
        }

        fn link(
            &self,
            ffmpeg: &FakeFfmpegAdapter,
            file_name: &str,
            video_id: &str,
        ) -> Result<(), String> {
            link_impl(
                &self.db,
                ffmpeg,
                &self.notifier,
                &self.root,
                &self.folder,
                file_name,
                video_id,
            )
        }

        fn ready(&self, id: &str) -> bool {
            let conn = self.db.writer.lock().unwrap();
            conn.query_row(
                "SELECT thumbnail_ready FROM videos WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
        }

        fn recorded(&self) -> usize {
            let conn = self.db.writer.lock().unwrap();
            queries::list_unmatched_thumbnails(&conn).unwrap().len()
        }

        /// Every file under the thumbnails root.
        fn files(&self) -> Vec<PathBuf> {
            fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
                let Ok(entries) = std::fs::read_dir(dir) else {
                    return;
                };
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, out);
                    } else {
                        out.push(p);
                    }
                }
            }
            let mut out = Vec::new();
            walk(&self.root, &mut out);
            out
        }
    }

    fn converting_ffmpeg() -> FakeFfmpegAdapter {
        FakeFfmpegAdapter {
            convert_result: Box::new(|_| Ok(())),
            ..Default::default()
        }
    }

    fn failing_ffmpeg() -> FakeFfmpegAdapter {
        FakeFfmpegAdapter {
            convert_result: Box::new(|_| Err(FfmpegError::Spawn("boom".into()))),
            ..Default::default()
        }
    }

    fn slot_names(files: &[PathBuf], id: &str) -> Vec<String> {
        let mut names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(id))
            .collect();
        names.sort();
        names
    }

    fn expected_slots(id: &str) -> Vec<String> {
        (0..6).map(|i| format!("{id}_{i}.webp")).collect()
    }

    #[test]
    fn linking_places_six_slots_marks_ready_removes_the_record_and_notifies() {
        let env = Env::new(None);
        env.add_video("vid", "offline", 0);

        env.link(&converting_ffmpeg(), FILE, "vid").unwrap();

        assert_eq!(slot_names(&env.files(), "vid"), expected_slots("vid"));
        assert!(env.ready("vid"));
        assert_eq!(env.recorded(), 0);
        assert_eq!(env.notifier.calls(), 1);
    }

    #[test]
    fn the_source_is_the_recorded_folder_joined_with_the_recorded_name() {
        let env = Env::new(None);
        env.add_video("vid", "offline", 0);
        let ffmpeg = converting_ffmpeg();

        env.link(&ffmpeg, FILE, "vid").unwrap();

        let calls = ffmpeg.calls.lock().unwrap();
        assert!(calls.iter().any(|c| matches!(
            c,
            FakeCall::ConvertImageToWebp { src_path, .. }
                if *src_path == Path::new(&env.folder).join(FILE)
        )));
    }

    #[test]
    fn a_conversion_failure_keeps_the_record_and_leaves_no_files() {
        let env = Env::new(None);
        env.add_video("vid", "offline", 0);

        let err = env.link(&failing_ffmpeg(), FILE, "vid").unwrap_err();

        assert!(err.contains("紐付けに失敗しました") && err.contains("WebPへの変換に失敗"));
        assert_eq!(env.recorded(), 1);
        assert!(env.files().is_empty(), "leftover: {:?}", env.files());
        assert!(!env.ready("vid"));
        assert_eq!(env.notifier.calls(), 0);
    }

    #[test]
    fn a_split_failure_with_partial_writes_leaves_no_files() {
        let env = Env::new(Some("100x50x3x2"));
        env.add_video("vid", "offline", 0);
        let ffmpeg = FakeFfmpegAdapter {
            metadata: Ok(VideoMetadata {
                width: Some(300),
                height: Some(100),
                ..Default::default()
            }),
            split_result: Box::new(|_, _| Err(FfmpegError::Spawn("boom".into()))),
            split_partial_writes: 3,
            ..Default::default()
        };

        let err = env.link(&ffmpeg, FILE, "vid").unwrap_err();

        assert!(err.contains("分割に失敗"));
        assert_eq!(env.recorded(), 1);
        assert!(env.files().is_empty(), "leftover: {:?}", env.files());
    }

    #[test]
    fn a_missing_source_file_is_reported_and_the_record_stays() {
        let env = Env::new(None);
        env.add_video("vid", "offline", 0);
        std::fs::remove_file(Path::new(&env.folder).join(FILE)).unwrap();
        let ffmpeg = converting_ffmpeg();

        let err = env.link(&ffmpeg, FILE, "vid").unwrap_err();

        assert!(err.contains("旧サムネイルのファイルが見つかりません"));
        assert_eq!(env.recorded(), 1);
        assert!(ffmpeg.calls.lock().unwrap().is_empty());
        assert!(env.files().is_empty());
    }

    #[test]
    fn an_unknown_video_id_is_rejected() {
        let env = Env::new(None);
        let ffmpeg = converting_ffmpeg();

        let err = env.link(&ffmpeg, FILE, "nope").unwrap_err();

        assert!(err.contains("動画が見つかりません"));
        assert_eq!(env.recorded(), 1);
        assert!(ffmpeg.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn an_online_video_is_rejected_without_touching_anything() {
        let env = Env::new(None);
        env.add_video("vid", "online", 0);
        let ffmpeg = converting_ffmpeg();

        let err = env.link(&ffmpeg, FILE, "vid").unwrap_err();

        assert!(err.contains("オフライン"));
        assert_eq!(env.recorded(), 1);
        assert!(ffmpeg.calls.lock().unwrap().is_empty());
        assert!(env.files().is_empty());
        assert!(!env.ready("vid"));
    }

    #[test]
    fn an_unrecorded_file_name_is_rejected_even_if_it_escapes_the_folder() {
        let env = Env::new(None);
        env.add_video("vid", "offline", 0);
        // A real file next to the recorded folder, reachable by `..`.
        std::fs::write(Path::new(&env.folder).parent().unwrap().join("x.jpg"), b"x").unwrap();
        let ffmpeg = converting_ffmpeg();

        for name in ["..\\x.jpg", "../x.jpg", "other.jpg", ""] {
            let err = env.link(&ffmpeg, name, "vid").unwrap_err();
            assert!(err.contains("一覧にありません"), "{name}: {err}");
        }
        let err = link_impl(
            &env.db,
            &ffmpeg,
            &env.notifier,
            &env.root,
            "somewhere else",
            FILE,
            "vid",
        )
        .unwrap_err();
        assert!(err.contains("一覧にありません"));
        assert!(ffmpeg.calls.lock().unwrap().is_empty());
        assert_eq!(env.recorded(), 1);
        assert!(env.files().is_empty());
    }

    #[test]
    fn linking_replaces_the_thumbnails_a_video_already_has() {
        let env = Env::new(None);
        env.add_video("vid", "offline", 1);
        let dir = env.root.join(gb_core::paths::THUMBNAIL_UNASSIGNED_SUBDIR);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..6 {
            std::fs::write(dir.join(format!("vid_{i}.webp")), b"old").unwrap();
        }

        env.link(&converting_ffmpeg(), FILE, "vid").unwrap();

        assert_eq!(slot_names(&env.files(), "vid"), expected_slots("vid"));
        for i in 0..6 {
            assert_eq!(
                std::fs::read(dir.join(format!("vid_{i}.webp"))).unwrap(),
                b"fake webp bytes"
            );
        }
        assert!(env.ready("vid"));
        assert_eq!(env.recorded(), 0);
    }

    #[test]
    fn a_failed_overwrite_clears_the_flag_of_the_thumbnails_it_removed() {
        let env = Env::new(None);
        env.add_video("vid", "offline", 1);
        let dir = env.root.join(gb_core::paths::THUMBNAIL_UNASSIGNED_SUBDIR);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..6 {
            std::fs::write(dir.join(format!("vid_{i}.webp")), b"old").unwrap();
        }

        env.link(&failing_ffmpeg(), FILE, "vid").unwrap_err();

        assert!(env.files().is_empty());
        assert!(!env.ready("vid"));
        assert_eq!(env.recorded(), 1);
    }

    #[test]
    fn a_recorded_layout_that_fits_the_image_splits_it_into_frames() {
        let env = Env::new(Some("100x50x3x2"));
        env.add_video("vid", "offline", 0);
        let ffmpeg = FakeFfmpegAdapter {
            metadata: Ok(VideoMetadata {
                width: Some(300),
                height: Some(100),
                ..Default::default()
            }),
            split_result: Box::new(|_, _| Ok(())),
            ..Default::default()
        };

        env.link(&ffmpeg, FILE, "vid").unwrap();

        let calls = ffmpeg.calls.lock().unwrap();
        assert!(calls.iter().any(
            |c| matches!(c, FakeCall::SplitImageToWebp { outputs, .. } if outputs.len() == 6)
        ));
        assert!(!calls
            .iter()
            .any(|c| matches!(c, FakeCall::ConvertImageToWebp { .. })));
        drop(calls);
        assert_eq!(slot_names(&env.files(), "vid"), expected_slots("vid"));
        assert_eq!(env.recorded(), 0);
    }

    #[test]
    fn without_a_layout_the_whole_image_is_converted() {
        let env = Env::new(None);
        env.add_video("vid", "offline", 0);
        let ffmpeg = converting_ffmpeg();

        env.link(&ffmpeg, FILE, "vid").unwrap();

        let calls = ffmpeg.calls.lock().unwrap();
        assert!(calls
            .iter()
            .any(|c| matches!(c, FakeCall::ConvertImageToWebp { .. })));
        assert!(!calls
            .iter()
            .any(|c| matches!(c, FakeCall::SplitImageToWebp { .. })));
    }

    #[test]
    fn an_unparsable_stored_layout_falls_back_to_the_whole_image() {
        let env = Env::new(Some("garbage"));
        env.add_video("vid", "offline", 0);
        let ffmpeg = converting_ffmpeg();

        env.link(&ffmpeg, FILE, "vid").unwrap();

        assert!(!ffmpeg
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|c| matches!(c, FakeCall::SplitImageToWebp { .. })));
    }

    #[test]
    fn list_adds_the_default_search_term() {
        let env = Env::new(None);
        let rows = list_impl(&env.db).unwrap();
        assert_eq!(
            rows,
            vec![UnmatchedThumbnail {
                folder_path: env.folder.clone(),
                file_name: FILE.to_string(),
                kind: "no_matching_video".to_string(),
                default_search_term: Some("movie.mp4".to_string()),
            }]
        );
    }

    #[test]
    fn search_returns_offline_videos_only_with_the_ready_flag_and_a_capped_limit() {
        let env = Env::new(None);
        env.add_video("a", "offline", 1);
        env.add_video("b", "offline", 0);
        env.add_video("c", "online", 0);

        let found = search_impl(&env.db, " movie ", 10).unwrap();
        let ids: Vec<(&str, bool)> = found
            .iter()
            .map(|c| (c.id.as_str(), c.thumbnail_ready))
            .collect();
        assert_eq!(ids, [("a", true), ("b", false)]);
        assert_eq!(search_impl(&env.db, "movie", 1).unwrap().len(), 1);
    }

    #[test]
    fn dismiss_removes_only_the_named_record() {
        let env = Env::new(None);
        dismiss_impl(&env.db, &env.folder, "other.jpg").unwrap();
        assert_eq!(env.recorded(), 1);
        dismiss_impl(&env.db, &env.folder, FILE).unwrap();
        assert_eq!(env.recorded(), 0);
    }
}
