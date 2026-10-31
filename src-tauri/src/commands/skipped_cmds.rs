//! Commands acting on the "unregistered files" list (`skipped_files`).

use std::path::Path;
use std::sync::Arc;

use gb_core::ports::file_rename::{FileRenamer, RenameError};
use gb_core::ports::path_existence::PathExistence;
use gb_core::presence::{classify_presence, Presence};
use gb_core::rename_validation::validate_new_file_name;
use gb_core::scan_pipeline::NewVideo;
use gb_core::wb_skipped_payload::WbSkippedPayload;
use serde::Serialize;
use tauri::State;

use crate::adapters;
use crate::adapters::long_path;
use crate::db::{queries, Db};
use crate::events::{CatalogNotifier, TauriCatalogNotifier};
use crate::reachability::ReachabilityRegistry;
use crate::scan;
use crate::thumbnail::{self, ThumbnailQueueHandle};

/// Whether the file recorded at `file_path` is verifiably gone. A path under
/// an unreachable watch folder is `Unknown` without any file-system access:
/// stat-ing a dead share blocks for a long time and could not prove anything.
pub(crate) fn presence_of(
    file_path: &str,
    watch_folders: &[String],
    reachability: &ReachabilityRegistry,
    fs: &dyn PathExistence,
) -> Presence {
    if reachability.is_path_unreachable(file_path, watch_folders) {
        return classify_presence(true, false, false);
    }
    let path = Path::new(file_path);
    let file_exists = fs.exists(path);
    // Only a missing file needs the parent checked.
    let parent_exists = file_exists || path.parent().is_some_and(|parent| fs.exists(parent));
    classify_presence(false, parent_exists, file_exists)
}

/// Hides an unregistered-file record whose file is gone. The file system is
/// re-checked here rather than trusting the list the UI is showing, which may
/// be stale (the file may have come back, or its share gone offline since).
fn dismiss_skipped_file_impl(
    db: &Db,
    reachability: &ReachabilityRegistry,
    fs: &dyn PathExistence,
    notifier: &dyn CatalogNotifier,
    id: i64,
) -> Result<(), String> {
    let row = queries::get_skipped_file(&db.read_pool, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "この未登録ファイルはすでに一覧にありません。".to_string())?;

    let watch_folders = {
        let conn = db.read_pool.get().map_err(|e| e.to_string())?;
        queries::get_watch_folders(&conn).map_err(|e| e.to_string())?
    };
    match presence_of(&row.file_path, &watch_folders, reachability, fs) {
        Presence::Missing => {}
        Presence::Present => {
            return Err(
                "元のファイルが存在するため、一覧から削除できません。「名前を変更」してください。"
                    .to_string(),
            )
        }
        Presence::Unknown => {
            return Err(
                "元のファイルがないことを確認できないため、一覧から削除できません。フォルダに接続できる状態で、もう一度お試しください。"
                    .to_string(),
            )
        }
    }

    let dismissed = {
        let conn = db.writer.lock().unwrap();
        queries::dismiss_skipped_file(&conn, id)
    }
    .map_err(|e| e.to_string())?;
    if !dismissed {
        return Err("この未登録ファイルはすでに一覧にありません。".to_string());
    }
    log::info!(
        "dismissed unregistered record for missing file {}",
        row.file_path
    );
    notifier.notify_changed();
    Ok(())
}

#[tauri::command(async)]
pub fn dismiss_skipped_file(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    reachability: State<'_, ReachabilityRegistry>,
    id: i64,
) -> Result<(), String> {
    let notifier = TauriCatalogNotifier::new(app);
    dismiss_skipped_file_impl(
        &db,
        &reachability,
        &adapters::path_existence::RealPathExistence,
        &notifier,
        id,
    )
}

/// Registers the record of a file that is gone as an offline video under the
/// name the user typed, and hides the record. Nothing is renamed on disk: the
/// user keeps the entry as a note that they once had the video (e.g. moved to
/// cold storage). Like the .wb import's offline rows it has no size or hash,
/// and it is not queued for thumbnails or probing since there is no file.
fn register_missing_skipped_file_impl(
    db: &Db,
    reachability: &ReachabilityRegistry,
    fs: &dyn PathExistence,
    notifier: &dyn CatalogNotifier,
    id: i64,
    input: &str,
) -> Result<MissingRegistration, String> {
    const GONE: &str = "この未登録ファイルはすでに一覧にありません。";
    let row = queries::get_skipped_file(&db.read_pool, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| GONE.to_string())?;

    let new_name = validate_new_file_name(&row.file_name, input).map_err(|e| e.to_string())?;

    // The folder comes from the recorded path, never from the input, so the
    // video cannot be registered outside the original folder. The recorded
    // separator style is kept so the path matches what scans store.
    let folder_end = row
        .file_path
        .rfind(['/', '\\'])
        .ok_or_else(|| "元のファイルの場所を特定できません。".to_string())?;
    let new_path = format!("{}{}", &row.file_path[..=folder_end], new_name);

    let watch_folders = {
        let conn = db.read_pool.get().map_err(|e| e.to_string())?;
        queries::get_watch_folders(&conn).map_err(|e| e.to_string())?
    };
    match presence_of(&row.file_path, &watch_folders, reachability, fs) {
        Presence::Missing => {}
        Presence::Present => {
            return Err(
                "元のファイルが存在するため、この方法では登録できません。「名前を変更」してください。"
                    .to_string(),
            )
        }
        Presence::Unknown => {
            return Err(
                "元のファイルがないことを確認できないため、登録できません。フォルダに接続できる状態で、もう一度お試しください。"
                    .to_string(),
            )
        }
    }

    let video = NewVideo {
        id: uuid::Uuid::new_v4().to_string(),
        file_path: new_path.clone(),
        file_name: new_name,
        file_size: 0,
        quick_hash: String::new(),
        status: "offline",
        mtime: 0,
    };
    let wb = row
        .wb_payload
        .as_deref()
        .and_then(WbSkippedPayload::from_json);
    let outcome = {
        let mut conn = db.writer.lock().unwrap();
        queries::register_missing_skipped_as_video(&mut conn, id, &video, wb.as_ref())
    }
    .map_err(|e| e.to_string())?;
    match outcome {
        queries::RegisterMissingOutcome::Registered => {}
        queries::RegisterMissingOutcome::PathTaken => {
            return Err("同じ名前の動画がすでに登録されています。".to_string())
        }
        queries::RegisterMissingOutcome::RecordGone => return Err(GONE.to_string()),
    }
    log::info!(
        "registered missing file {} as offline video {new_path}",
        row.file_path
    );
    notifier.notify_changed();
    Ok(MissingRegistration {
        carried_wb_data: wb.is_some(),
    })
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct MissingRegistration {
    /// `false` when the record had no .wb data (it was not found through a .wb
    /// import), so the video has no rating or tags.
    pub carried_wb_data: bool,
}

#[tauri::command(async)]
pub fn register_missing_skipped_file(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    reachability: State<'_, ReachabilityRegistry>,
    id: i64,
    new_name: String,
) -> Result<MissingRegistration, String> {
    let notifier = TauriCatalogNotifier::new(app);
    register_missing_skipped_file_impl(
        &db,
        &reachability,
        &adapters::path_existence::RealPathExistence,
        &notifier,
        id,
        &new_name,
    )
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct RenameOutcome {
    pub new_file_name: String,
    /// `false` when the file was renamed but registering it failed; the next
    /// scan or watch event registers it, so the caller only needs to tell
    /// the user to expect that.
    pub registered: bool,
}

/// Renames an unregistered file in place and registers it as a normal video.
///
/// Order matters: the record is deleted only after the rename succeeded (a
/// failed rename must leave it so the user can retry), and registration comes
/// last and is allowed to fail without undoing the rename -- the file then has
/// a valid name and the regular scan picks it up.
fn rename_skipped_file_impl(
    db: &Db,
    renamer: &dyn FileRenamer,
    notifier: &dyn CatalogNotifier,
    register: impl Fn(&Path) -> anyhow::Result<()>,
    id: i64,
    input: &str,
) -> Result<RenameOutcome, String> {
    let row = queries::get_skipped_file(&db.read_pool, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "この未登録ファイルはすでに一覧にありません。".to_string())?;

    let new_name = validate_new_file_name(&row.file_name, input).map_err(|e| e.to_string())?;

    let old_path = Path::new(&row.file_path);
    // The parent comes from the recorded path, never from the input, so a
    // rename cannot leave the original folder.
    let parent = old_path
        .parent()
        .ok_or_else(|| "元のファイルの場所を特定できません。".to_string())?;
    let new_path = parent.join(&new_name);

    renamer
        .rename_no_overwrite(old_path, &new_path)
        .map_err(|e| match e {
            RenameError::TargetExists => "同じ名前のファイルがすでにあります。".to_string(),
            RenameError::SourceMissing => "元のファイルが見つかりません。".to_string(),
            RenameError::Failed { message } => {
                log::warn!("rename of {} failed: {message}", row.file_path);
                format!("名前を変更できませんでした（使用中の可能性があります）: {message}")
            }
        })?;
    log::info!("renamed unregistered file {} -> {new_name}", row.file_path);

    let delete_result = {
        let conn = db.writer.lock().unwrap();
        queries::delete_skipped_file(&conn, id)
    };
    if let Err(e) = &delete_result {
        log::error!("failed to delete skipped record {id} after rename: {e}");
    }

    let registered = match register(&new_path) {
        Ok(()) => true,
        Err(e) => {
            log::error!(
                "renamed {} but registering it failed: {e}",
                new_path.display()
            );
            false
        }
    };
    notifier.notify_changed();

    if delete_result.is_err() {
        return Err(
            "名前は変更しましたが、未登録の記録を更新できませんでした。再起動後に確認してください。"
                .to_string(),
        );
    }
    Ok(RenameOutcome {
        new_file_name: new_name,
        registered,
    })
}

#[tauri::command(async)]
pub fn rename_skipped_file(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    queue: State<'_, ThumbnailQueueHandle>,
    reachability: State<'_, ReachabilityRegistry>,
    id: i64,
    new_name: String,
) -> Result<RenameOutcome, String> {
    let thumbnails_root = crate::paths::app_data_dir()
        .map(|dir| thumbnail::paths::thumbnails_root(&dir))
        .map_err(|e| e.to_string())?;
    let notifier = Arc::new(TauriCatalogNotifier::new(app));
    let register_db = db.inner().clone();
    let register_root = thumbnails_root.clone();
    // Same stat-then-`process_detected_file` path as the file watcher, so the
    // outcome is identical whichever of the two sees the renamed file first.
    let register = move |path: &Path| -> anyhow::Result<()> {
        let metadata = std::fs::metadata(long_path::to_long_path(path))?;
        let mtime = scan::mtime_from_metadata(&metadata).map_err(anyhow::Error::msg)?;
        match scan::process_detected_file(
            &register_db,
            &register_root,
            path,
            metadata.len(),
            mtime,
        )? {
            scan::ProcessOutcome::SkippedUnreadable => {
                anyhow::bail!("the file is locked or unreadable")
            }
            _ => Ok(()),
        }
    };

    let outcome = rename_skipped_file_impl(
        &db,
        &adapters::file_rename::RealFileRenamer,
        notifier.as_ref(),
        register,
        id,
        &new_name,
    )?;

    if outcome.registered {
        // Same "resume after registration" passes `start_scan` runs, so the
        // new video gets its thumbnails/duration without waiting for a scan.
        let _ = std::fs::create_dir_all(&thumbnails_root);
        thumbnail::enqueue_missing_thumbnails(
            db.inner().clone(),
            thumbnails_root,
            queue.inner().clone(),
            Arc::new(adapters::ffmpeg::RealFfmpegAdapter),
            Arc::clone(&notifier),
            reachability.inner().clone(),
        );
        crate::metadata::enqueue_missing_metadata_probes(
            db.inner().clone(),
            Arc::new(adapters::ffmpeg::RealFfmpegAdapter),
            notifier,
            reachability.inner().clone(),
        );
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::init_temp_db;
    use crate::events::{FakeCatalogNotifier, FakeReachabilityNotifier};
    use gb_core::scan_pipeline::NewSkippedFile;
    use gb_core::testing::fake_file_rename::FakeFileRenamer;
    use gb_core::testing::fake_folder_probe::FakeFolderProbe;
    use gb_core::testing::fake_path_existence::FakePathExistence;
    use std::cell::RefCell;

    const OLD_PATH: &str = "C:/videos/movie\u{2460}.mp4";

    fn setup(existing: &[&str]) -> (tempfile::TempDir, Db, i64, FakeFileRenamer) {
        let (dir, db) = init_temp_db();
        queries::upsert_skipped_file(
            &db.writer.lock().unwrap(),
            &NewSkippedFile {
                file_path: OLD_PATH.to_string(),
                file_name: "movie\u{2460}.mp4".to_string(),
                reason: "machine_dependent_char",
                detected_char: '\u{2460}',
            },
        )
        .unwrap();
        let id = queries::list_skipped_files(&db.read_pool).unwrap()[0].id;
        let mut files = vec![OLD_PATH];
        files.extend_from_slice(existing);
        (dir, db, id, FakeFileRenamer::with_files(&files))
    }

    fn remaining(db: &Db) -> usize {
        queries::list_skipped_files(&db.read_pool).unwrap().len()
    }

    fn registry(unreachable: &[&str]) -> ReachabilityRegistry {
        ReachabilityRegistry::new(
            Arc::new(FakeFolderProbe::with_unreachable(unreachable)),
            Arc::new(FakeReachabilityNotifier::default()),
        )
    }

    fn watch(db: &Db, folders: &[&str]) {
        let folders: Vec<String> = folders.iter().map(|f| f.to_string()).collect();
        queries::set_watch_folders(&db.writer.lock().unwrap(), &folders).unwrap();
    }

    #[test]
    fn dismiss_hides_the_record_of_a_missing_file_and_notifies() {
        let (_d, db, id, _renamer) = setup(&[]);
        watch(&db, &["C:/videos"]);
        let fs = FakePathExistence::with_paths(&["C:/videos"]);
        let notifier = FakeCatalogNotifier::default();

        dismiss_skipped_file_impl(&db, &registry(&[]), &fs, &notifier, id).unwrap();

        assert_eq!(remaining(&db), 0);
        assert!(queries::get_skipped_file(&db.read_pool, id)
            .unwrap()
            .is_some());
        assert_eq!(notifier.calls(), 1);
    }

    #[test]
    fn dismiss_refuses_when_the_file_exists() {
        let (_d, db, id, _renamer) = setup(&[]);
        watch(&db, &["C:/videos"]);
        let fs = FakePathExistence::with_paths(&["C:/videos", OLD_PATH]);
        let notifier = FakeCatalogNotifier::default();

        let r = dismiss_skipped_file_impl(&db, &registry(&[]), &fs, &notifier, id);

        assert!(r.unwrap_err().contains("存在"));
        assert_eq!(remaining(&db), 1);
        assert_eq!(notifier.calls(), 0);
    }

    #[test]
    fn dismiss_refuses_when_the_folder_is_unreachable() {
        let (_d, db, id, _renamer) = setup(&[]);
        watch(&db, &["C:/videos"]);
        // Would read as Missing if the dead folder were stat-ed.
        let fs = FakePathExistence::with_paths(&["C:/videos"]);
        let notifier = FakeCatalogNotifier::default();

        let r = dismiss_skipped_file_impl(&db, &registry(&["C:/videos"]), &fs, &notifier, id);

        assert!(r.unwrap_err().contains("確認できない"));
        assert_eq!(remaining(&db), 1);
        assert_eq!(notifier.calls(), 0);
    }

    #[test]
    fn dismiss_refuses_when_the_parent_folder_is_gone_too() {
        let (_d, db, id, _renamer) = setup(&[]);
        watch(&db, &["C:/videos"]);
        let fs = FakePathExistence::default();
        let notifier = FakeCatalogNotifier::default();

        let r = dismiss_skipped_file_impl(&db, &registry(&[]), &fs, &notifier, id);

        assert!(r.is_err());
        assert_eq!(remaining(&db), 1);
        assert_eq!(notifier.calls(), 0);
    }

    #[test]
    fn dismiss_of_an_unknown_id_is_an_error() {
        let (_d, db, id, _renamer) = setup(&[]);
        let fs = FakePathExistence::with_paths(&["C:/videos"]);
        let notifier = FakeCatalogNotifier::default();

        let r = dismiss_skipped_file_impl(&db, &registry(&[]), &fs, &notifier, id + 99);

        assert!(r.is_err());
        assert_eq!(notifier.calls(), 0);
    }

    fn video_count(db: &Db) -> i64 {
        db.read_pool
            .get()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap()
    }

    fn add_video(db: &Db, path: &str) {
        let name = path.rsplit('/').next().unwrap();
        queries::insert_video(
            &db.writer.lock().unwrap(),
            &NewVideo {
                id: uuid::Uuid::new_v4().to_string(),
                file_path: path.to_string(),
                file_name: name.to_string(),
                file_size: 1,
                quick_hash: "h".to_string(),
                status: "online",
                mtime: 1,
            },
        )
        .unwrap();
    }

    #[test]
    fn register_missing_adds_an_offline_video_and_hides_the_record() {
        let (_d, db, id, _renamer) = setup(&[]);
        watch(&db, &["C:/videos"]);
        let fs = FakePathExistence::with_paths(&["C:/videos"]);
        let notifier = FakeCatalogNotifier::default();

        register_missing_skipped_file_impl(&db, &registry(&[]), &fs, &notifier, id, "movie.mp4")
            .unwrap();

        assert_eq!(remaining(&db), 0);
        assert_eq!(notifier.calls(), 1);
        let conn = db.read_pool.get().unwrap();
        let row = queries::find_video_by_path(&conn, "C:/videos/movie.mp4")
            .unwrap()
            .unwrap();
        assert_eq!(row.status, "offline");
        assert_eq!(row.quick_hash, "");
        assert_eq!(row.file_size, 0);
        assert_eq!(row.mtime, Some(0));
        assert_eq!(row.file_name, "movie.mp4");
        assert_eq!(video_count(&db), 1);
    }

    fn payload() -> WbSkippedPayload {
        WbSkippedPayload {
            rating: 4,
            rating_from_star_tags: false,
            kana: "かな".to_string(),
            roma: "kana".to_string(),
            tags: vec!["Action".to_string(), "旅行".to_string()],
        }
    }

    #[test]
    fn register_missing_with_wb_payload_carries_rating_readings_and_tags() {
        let (_d, db, id, _renamer) = setup(&[]);
        queries::upsert_skipped_file_with_wb_payload(
            &db.writer.lock().unwrap(),
            &NewSkippedFile {
                file_path: OLD_PATH.to_string(),
                file_name: "movie\u{2460}.mp4".to_string(),
                reason: "machine_dependent_char",
                detected_char: '\u{2460}',
            },
            &payload().to_json(),
        )
        .unwrap();
        watch(&db, &["C:/videos"]);
        let fs = FakePathExistence::with_paths(&["C:/videos"]);
        let notifier = FakeCatalogNotifier::default();

        let out = register_missing_skipped_file_impl(
            &db,
            &registry(&[]),
            &fs,
            &notifier,
            id,
            "movie.mp4",
        )
        .unwrap();

        assert!(out.carried_wb_data);
        assert_eq!(remaining(&db), 0);
        let conn = db.read_pool.get().unwrap();
        let (status, hash, size, rating, kana, roma): (String, String, i64, i64, String, String) =
            conn.query_row(
                "SELECT status, quick_hash, file_size, rating, kana, roma FROM videos",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            (status.as_str(), hash.as_str(), size, rating),
            ("offline", "", 0, 4)
        );
        assert_eq!((kana.as_str(), roma.as_str()), ("かな", "kana"));
        let tags: i64 = conn
            .query_row("SELECT COUNT(*) FROM video_tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tags, 2);
    }

    #[test]
    fn register_missing_without_payload_registers_with_no_tags() {
        let (_d, db, id, _renamer) = setup(&[]);
        watch(&db, &["C:/videos"]);
        let fs = FakePathExistence::with_paths(&["C:/videos"]);
        let notifier = FakeCatalogNotifier::default();

        let out = register_missing_skipped_file_impl(
            &db,
            &registry(&[]),
            &fs,
            &notifier,
            id,
            "movie.mp4",
        )
        .unwrap();

        assert!(!out.carried_wb_data);
        let conn = db.read_pool.get().unwrap();
        let tags: i64 = conn
            .query_row("SELECT COUNT(*) FROM video_tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tags, 0);
        assert_eq!(video_count(&db), 1);
    }

    #[test]
    fn register_missing_treats_a_damaged_payload_as_absent() {
        let (_d, db, id, _renamer) = setup(&[]);
        db.writer
            .lock()
            .unwrap()
            .execute("UPDATE skipped_files SET wb_payload = '{broken'", [])
            .unwrap();
        watch(&db, &["C:/videos"]);
        let fs = FakePathExistence::with_paths(&["C:/videos"]);
        let notifier = FakeCatalogNotifier::default();

        let out = register_missing_skipped_file_impl(
            &db,
            &registry(&[]),
            &fs,
            &notifier,
            id,
            "movie.mp4",
        )
        .unwrap();

        assert!(!out.carried_wb_data);
        assert_eq!(video_count(&db), 1);
    }

    #[test]
    fn registered_record_stays_hidden_after_a_reimport_upsert() {
        let (_d, db, id, _renamer) = setup(&[]);
        watch(&db, &["C:/videos"]);
        let fs = FakePathExistence::with_paths(&["C:/videos"]);
        let notifier = FakeCatalogNotifier::default();
        register_missing_skipped_file_impl(&db, &registry(&[]), &fs, &notifier, id, "movie.mp4")
            .unwrap();

        queries::upsert_skipped_file(
            &db.writer.lock().unwrap(),
            &NewSkippedFile {
                file_path: OLD_PATH.to_string(),
                file_name: "movie\u{2460}.mp4".to_string(),
                reason: "machine_dependent_char",
                detected_char: '\u{2460}',
            },
        )
        .unwrap();

        assert_eq!(remaining(&db), 0);
    }

    #[test]
    fn register_missing_refuses_a_name_already_registered_ignoring_case() {
        let (_d, db, id, _renamer) = setup(&[]);
        watch(&db, &["C:/videos"]);
        add_video(&db, "C:/videos/Movie.MP4");
        let fs = FakePathExistence::with_paths(&["C:/videos"]);
        let notifier = FakeCatalogNotifier::default();

        let r = register_missing_skipped_file_impl(
            &db,
            &registry(&[]),
            &fs,
            &notifier,
            id,
            "movie.mp4",
        );

        assert!(r.unwrap_err().contains("すでに登録"));
        assert_eq!(remaining(&db), 1);
        assert_eq!(video_count(&db), 1);
        assert_eq!(notifier.calls(), 0);
    }

    #[test]
    fn register_missing_with_invalid_input_changes_nothing() {
        for input in [
            "movie\u{2461}.mp4",
            "",
            "a/b.mp4",
            "movie.txt",
            "bad:name.mp4",
        ] {
            let (_d, db, id, _renamer) = setup(&[]);
            watch(&db, &["C:/videos"]);
            let fs = FakePathExistence::with_paths(&["C:/videos"]);
            let notifier = FakeCatalogNotifier::default();

            let r =
                register_missing_skipped_file_impl(&db, &registry(&[]), &fs, &notifier, id, input);

            assert!(r.is_err(), "{input:?} should be rejected");
            assert_eq!(remaining(&db), 1, "{input:?}");
            assert_eq!(video_count(&db), 0, "{input:?}");
            assert_eq!(notifier.calls(), 0, "{input:?}");
        }
    }

    #[test]
    fn register_missing_refuses_when_the_file_exists_or_is_unverifiable() {
        let cases: [(&[&str], &[&str]); 3] = [
            (&["C:/videos", OLD_PATH], &[]),
            (&["C:/videos"], &["C:/videos"]),
            (&[], &[]),
        ];
        for (existing, unreachable) in cases {
            let (_d, db, id, _renamer) = setup(&[]);
            watch(&db, &["C:/videos"]);
            let fs = FakePathExistence::with_paths(existing);
            let notifier = FakeCatalogNotifier::default();

            let r = register_missing_skipped_file_impl(
                &db,
                &registry(unreachable),
                &fs,
                &notifier,
                id,
                "movie.mp4",
            );

            assert!(r.is_err(), "{existing:?} {unreachable:?}");
            assert_eq!(remaining(&db), 1);
            assert_eq!(video_count(&db), 0);
            assert_eq!(notifier.calls(), 0);
        }
    }

    #[test]
    fn register_missing_of_an_unknown_id_is_an_error() {
        let (_d, db, id, _renamer) = setup(&[]);
        let fs = FakePathExistence::with_paths(&["C:/videos"]);
        let notifier = FakeCatalogNotifier::default();

        let r = register_missing_skipped_file_impl(
            &db,
            &registry(&[]),
            &fs,
            &notifier,
            id + 99,
            "movie.mp4",
        );

        assert!(r.is_err());
        assert_eq!(video_count(&db), 0);
        assert_eq!(notifier.calls(), 0);
    }

    #[test]
    fn success_renames_deletes_the_record_registers_and_notifies() {
        let (_d, db, id, renamer) = setup(&[]);
        let notifier = FakeCatalogNotifier::default();
        let registered = RefCell::new(Vec::new());

        let out = rename_skipped_file_impl(
            &db,
            &renamer,
            &notifier,
            |p| {
                registered.borrow_mut().push(p.to_path_buf());
                Ok(())
            },
            id,
            " movie1.mp4 ",
        )
        .unwrap();

        assert_eq!(
            out,
            RenameOutcome {
                new_file_name: "movie1.mp4".into(),
                registered: true
            }
        );
        assert!(renamer.exists(Path::new("C:/videos/movie1.mp4")));
        assert!(!renamer.exists(Path::new(OLD_PATH)));
        assert_eq!(remaining(&db), 0);
        assert_eq!(
            registered.borrow().as_slice(),
            [Path::new("C:/videos").join("movie1.mp4")]
        );
        assert_eq!(notifier.calls(), 1);
    }

    #[test]
    fn rejected_input_touches_nothing() {
        for input in [
            "",
            "movie\u{2460}.mp4",
            "movie\u{2461}.mp4",
            "..\\x.mp4",
            "a.txt",
            "con.mp4",
        ] {
            let (_d, db, id, renamer) = setup(&[]);
            let notifier = FakeCatalogNotifier::default();
            let called = RefCell::new(false);

            let r = rename_skipped_file_impl(
                &db,
                &renamer,
                &notifier,
                |_| {
                    *called.borrow_mut() = true;
                    Ok(())
                },
                id,
                input,
            );

            assert!(r.is_err(), "{input:?}");
            assert!(renamer.calls.lock().unwrap().is_empty(), "{input:?}");
            assert_eq!(remaining(&db), 1, "{input:?}");
            assert!(!*called.borrow());
            assert_eq!(notifier.calls(), 0);
        }
    }

    #[test]
    fn unknown_id_is_an_error() {
        let (_d, db, id, renamer) = setup(&[]);
        let notifier = FakeCatalogNotifier::default();
        let r = rename_skipped_file_impl(&db, &renamer, &notifier, |_| Ok(()), id + 99, "a.mp4");
        assert!(r.is_err());
        assert!(renamer.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn existing_target_keeps_the_record_and_both_files() {
        let (_d, db, id, renamer) = setup(&["C:/videos/Movie1.mp4"]);
        let notifier = FakeCatalogNotifier::default();
        let called = RefCell::new(false);

        let r = rename_skipped_file_impl(
            &db,
            &renamer,
            &notifier,
            |_| {
                *called.borrow_mut() = true;
                Ok(())
            },
            id,
            "movie1.mp4",
        );

        assert!(r.unwrap_err().contains("同じ名前"));
        assert_eq!(remaining(&db), 1);
        assert!(renamer.exists(Path::new(OLD_PATH)));
        assert!(renamer.exists(Path::new("C:/videos/Movie1.mp4")));
        assert!(!*called.borrow());
        assert_eq!(notifier.calls(), 0);
    }

    #[test]
    fn rename_failure_keeps_the_record() {
        let (_d, db, id, renamer) = setup(&[]);
        *renamer.forced_error.lock().unwrap() = Some(RenameError::Failed {
            message: "locked".into(),
        });
        let notifier = FakeCatalogNotifier::default();
        let called = RefCell::new(false);

        let r = rename_skipped_file_impl(
            &db,
            &renamer,
            &notifier,
            |_| {
                *called.borrow_mut() = true;
                Ok(())
            },
            id,
            "movie1.mp4",
        );

        assert!(r.unwrap_err().contains("locked"));
        assert_eq!(remaining(&db), 1);
        assert!(!*called.borrow());
        assert_eq!(notifier.calls(), 0);
    }

    #[test]
    fn missing_source_keeps_the_record() {
        let (_d, db, id, renamer) = setup(&[]);
        *renamer.forced_error.lock().unwrap() = Some(RenameError::SourceMissing);
        let notifier = FakeCatalogNotifier::default();
        let r = rename_skipped_file_impl(&db, &renamer, &notifier, |_| Ok(()), id, "movie1.mp4");
        assert!(r.unwrap_err().contains("見つかりません"));
        assert_eq!(remaining(&db), 1);
    }

    #[test]
    fn registration_failure_still_drops_the_record_and_reports_renamed() {
        let (_d, db, id, renamer) = setup(&[]);
        let notifier = FakeCatalogNotifier::default();

        let out = rename_skipped_file_impl(
            &db,
            &renamer,
            &notifier,
            |_| anyhow::bail!("locked"),
            id,
            "movie1.mp4",
        )
        .unwrap();

        assert_eq!(
            out,
            RenameOutcome {
                new_file_name: "movie1.mp4".into(),
                registered: false
            }
        );
        assert!(renamer.exists(Path::new("C:/videos/movie1.mp4")));
        assert_eq!(remaining(&db), 0);
        assert_eq!(notifier.calls(), 1);
    }
}
