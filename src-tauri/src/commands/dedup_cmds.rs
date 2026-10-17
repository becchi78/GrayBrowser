//! Duplicate-group listing/refresh/cleanup commands. Thin passthroughs to
//! `dedup::`/`db::queries` -- no additional detection logic lives here.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use gb_core::file_delete_policy::{check_file_delete_allowed, FileDeleteFacts, FileState};
use gb_core::ports::drive_type::{DriveKind, DriveTypeDetector};
use gb_core::ports::trash::FileTrasher;
use tauri::State;

use crate::adapters::long_path;
use crate::db::Db;
use crate::dedup::{self, DuplicateGroup, DuplicateGroupsState};
use crate::events::{CatalogNotifier, TauriCatalogNotifier, TauriDedupNotifier};

/// Returns the most recently detected duplicate groups
/// (`dedup::refresh_duplicate_groups`'s last completed run's snapshot), with
/// each member's row values re-read from the DB so rating/tag edits made
/// since detection are visible. Does not itself trigger detection -- call
/// `refresh_duplicate_groups` for that.
#[tauri::command]
pub fn list_duplicate_groups(
    db: State<Db>,
    state: State<DuplicateGroupsState>,
) -> Vec<DuplicateGroup> {
    dedup::refresh_group_members(&db, state.get())
}

/// Fire-and-forget: kicks off a fresh duplicate-detection pass in the
/// background (mirrors `scan_cmds::start_scan`'s post-scan thumbnail/
/// metadata enqueue calls) and returns immediately. The frontend learns of
/// the result via the `dedup:updated` event, or by calling
/// `list_duplicate_groups` again later.
#[tauri::command]
pub async fn refresh_duplicate_groups(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    state: State<'_, DuplicateGroupsState>,
) -> Result<(), String> {
    dedup::refresh_duplicate_groups(
        db.inner().clone(),
        state.inner().clone(),
        Arc::new(TauriDedupNotifier::new(app)),
    );
    Ok(())
}

/// Removes the catalog entry for `video_id` -- the `videos` row plus every
/// `video_tags`/`path_collisions` row referencing it
/// (`queries::delete_video_cascade`) -- and deletes its cached thumbnails
/// (all 6 `[video_id]_0.webp`..`[video_id]_5.webp` slot files, under
/// whichever registered-folder subdirectory the video's `file_path`
/// currently resolves to) if any exist. Never touches the source video file.
///
/// `thumbnails_root` is `None` when the app data directory could not be
/// resolved; the thumbnail cleanup is then skipped but the catalog delete
/// still happens.
fn remove_video_from_catalog(
    db: &Db,
    thumbnails_root: Option<&Path>,
    video_id: &str,
) -> Result<(), String> {
    // Looked up *before* the DB delete below -- once the row is gone,
    // `file_path` (and therefore the resolved thumbnail subdirectory) is no
    // longer recoverable from the DB. A lookup failure/absence is not fatal
    // to this function: it just means the thumbnail-cleanup step below has
    // nothing to do.
    let file_path = crate::db::queries::find_video_by_id(&db.read_pool, video_id)
        .ok()
        .flatten()
        .map(|row| row.file_path);

    if let (Some(file_path), Some(thumbnails_root)) = (file_path, thumbnails_root) {
        let watch_folders = db
            .read_pool
            .get()
            .ok()
            .and_then(|conn| crate::db::queries::get_watch_folders(&conn).ok())
            .unwrap_or_default();
        let video_dir = crate::thumbnail::paths::video_thumbnail_dir(
            thumbnails_root,
            &watch_folders,
            &file_path,
        );
        // Best-effort: the thumbnails may never have been generated (e.g.
        // an offline video), same "may not exist" reasoning as
        // thumbnail::worker's own tmp-file cleanup.
        for i in 0..crate::thumbnail::worker::THUMBNAILS_PER_VIDEO {
            let thumb_path = crate::thumbnail::paths::slot_path(&video_dir, video_id, i);
            let _ = std::fs::remove_file(&thumb_path);
        }
    }

    let mut conn = db.writer.lock().unwrap();
    crate::db::queries::delete_video_cascade(&mut conn, video_id).map_err(|e| e.to_string())
}

fn thumbnails_root_from_app_dir() -> Option<PathBuf> {
    crate::paths::app_data_dir()
        .ok()
        .map(|app_dir| crate::thumbnail::paths::thumbnails_root(&app_dir))
}

/// Removes a video from the catalog (see `remove_video_from_catalog`).
///
/// **Does not touch the source video file on disk.** Only the catalog row
/// and cached thumbnail are removed; there is deliberately no
/// `std::fs::remove_file`/`remove_dir*` call anywhere in this function (or
/// in anything it calls) against `file_path`/the video's real on-disk
/// location. Duplicate detection is
/// notify-and-list only, with **no automatic deletion of the video file
/// itself** -- this command is the user's explicit "clear this catalog
/// entry" action after they've manually decided what (if anything) to do
/// with the underlying file, not a stand-in for deleting it on their behalf.
/// Moving the file to the recycle bin is the separate, explicitly named
/// `delete_duplicate_video_and_file`.
#[tauri::command]
pub fn delete_duplicate_video(
    app: tauri::AppHandle,
    db: State<Db>,
    video_id: String,
) -> Result<(), String> {
    let thumbnails_root = thumbnails_root_from_app_dir();
    remove_video_from_catalog(&db, thumbnails_root.as_deref(), &video_id)?;
    TauriCatalogNotifier::new(app).notify_changed();
    Ok(())
}

/// Serializes `delete_video_with_file` calls. The "another online member
/// remains" check and the trash move are not atomic, so without this two
/// concurrent calls on the last two copies could each see the other as the
/// survivor and both be recycled.
static FILE_DELETE_LOCK: Mutex<()> = Mutex::new(());

/// Moves the video's source file to the recycle bin and then removes its
/// catalog entry. Every decision input comes from the DB row and the file
/// system, never from the caller: the only caller-supplied value is
/// `video_id`.
///
/// On any refusal or failure before the file has been moved, neither the
/// recycle bin nor the catalog is touched. The catalog is only changed after
/// the move has succeeded, so a failed move leaves the entry in place; if the
/// catalog delete fails after a successful move, the file is already in the
/// recycle bin and the error says so.
pub fn delete_video_with_file(
    db: &Db,
    groups: Vec<DuplicateGroup>,
    drive_detector: &impl DriveTypeDetector,
    trasher: &impl FileTrasher,
    thumbnails_root: Option<&Path>,
    notifier: &impl CatalogNotifier,
    video_id: &str,
) -> Result<(), String> {
    let _guard = FILE_DELETE_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let row = crate::db::queries::find_video_by_id(&db.read_pool, video_id)
        .map_err(|e| format!("動画の情報を取得できませんでした: {e}"))?
        .ok_or_else(|| "動画がカタログに見つかりません".to_string())?;

    let online_members_in_group = dedup::refresh_group_members(db, groups)
        .iter()
        .find(|g| g.members.iter().any(|m| m.video_id == video_id))
        .map(|g| g.members.iter().filter(|m| m.status == "online").count())
        .ok_or_else(|| {
            "重複グループに見つかりません。重複を再検出してからやり直してください".to_string()
        })?;

    let watch_folders = db
        .read_pool
        .get()
        .map_err(|e| e.to_string())
        .and_then(|conn| crate::db::queries::get_watch_folders(&conn).map_err(|e| e.to_string()))
        .map_err(|e| format!("登録フォルダを取得できませんでした: {e}"))?;
    let under_registered_folder = watch_folders
        .iter()
        .any(|folder| gb_core::paths::is_path_under_folder(&row.file_path, folder));

    // A detection failure is treated as Unknown, which the policy refuses.
    let drive = drive_detector
        .detect(Path::new(&row.file_path))
        .unwrap_or(DriveKind::Unknown);

    // `symlink_metadata` so that a symlink is seen as a link, not as the
    // regular file it points to. Any stat failure counts as missing.
    let file = match std::fs::symlink_metadata(long_path::to_long_path(Path::new(&row.file_path))) {
        Ok(meta) if meta.is_file() => FileState::Regular { size: meta.len() },
        Ok(_) => FileState::NotRegularFile,
        Err(_) => FileState::Missing,
    };

    check_file_delete_allowed(&FileDeleteFacts {
        status: &row.status,
        drive,
        online_members_in_group,
        under_registered_folder,
        file,
        recorded_size: row.file_size,
    })
    .map_err(|refusal| refusal.to_string())?;

    trasher
        .move_to_trash(Path::new(&row.file_path))
        .map_err(|e| format!("ゴミ箱への移動に失敗しました: {e}"))?;

    remove_video_from_catalog(db, thumbnails_root, video_id).map_err(|e| {
        format!("ファイルはゴミ箱に移動しましたが、カタログからの削除に失敗しました: {e}")
    })?;
    notifier.notify_changed();
    Ok(())
}

/// "Delete file too": moves the source file to the recycle bin, then removes
/// the catalog entry. Errors are Japanese, user-displayable strings (a
/// refusal reason, or a failure description).
#[tauri::command]
pub fn delete_duplicate_video_and_file(
    app: tauri::AppHandle,
    db: State<Db>,
    state: State<DuplicateGroupsState>,
    video_id: String,
) -> Result<(), String> {
    let thumbnails_root = thumbnails_root_from_app_dir();
    let result = delete_video_with_file(
        &db,
        state.get(),
        &crate::adapters::drive_type::RealDriveTypeDetector,
        &crate::adapters::trash::RealFileTrasher,
        thumbnails_root.as_deref(),
        &TauriCatalogNotifier::new(app),
        &video_id,
    );
    match &result {
        Ok(()) => log::info!("moved the file of duplicate video {video_id} to the recycle bin"),
        Err(e) => log::warn!("did not delete the file of duplicate video {video_id}: {e}"),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::init_temp_db;
    use crate::dedup::{DuplicateGroupKind, DuplicateGroupMember};
    use crate::events::FakeCatalogNotifier;
    use gb_core::ports::drive_type::DriveTypeError;
    use gb_core::ports::trash::TrashError;
    use gb_core::testing::fake_drive_type::FakeDriveTypeDetector;
    use gb_core::testing::fake_trash::FakeFileTrasher;
    use std::sync::atomic::{AtomicBool, Ordering};

    const CONTENT: &[u8] = b"duplicate video bytes";

    struct Env {
        dir: tempfile::TempDir,
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
                dir,
                db,
                folder,
                thumbs,
            }
        }

        fn insert_row(&self, id: &str, path: &Path, status: &str, size: i64) {
            let conn = self.db.writer.lock().unwrap();
            conn.execute(
                "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status)
                 VALUES (?1, ?2, ?3, ?4, 'h', ?5)",
                rusqlite::params![
                    id,
                    path.to_string_lossy(),
                    path.file_name().unwrap().to_string_lossy(),
                    size,
                    status
                ],
            )
            .unwrap();
        }

        /// A throwaway file inside the registered folder plus its row.
        fn add_video(&self, id: &str, status: &str) -> PathBuf {
            let path = self.folder.join(format!("{id}.mp4"));
            std::fs::write(&path, CONTENT).unwrap();
            self.insert_row(id, &path, status, CONTENT.len() as i64);
            path
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

        fn add_tag(&self, id: &str) {
            let mut conn = self.db.writer.lock().unwrap();
            crate::db::queries::assign_tag_to_video(&mut conn, id, "tag").unwrap();
        }

        fn thumb_path(&self, id: &str, path: &Path) -> PathBuf {
            let dir = crate::thumbnail::paths::video_thumbnail_dir(
                &self.thumbs,
                &[self.folder.to_string_lossy().into_owned()],
                &path.to_string_lossy(),
            );
            crate::thumbnail::paths::slot_path(&dir, id, 0)
        }

        fn add_thumb(&self, id: &str, path: &Path) -> PathBuf {
            let thumb = self.thumb_path(id, path);
            std::fs::create_dir_all(thumb.parent().unwrap()).unwrap();
            std::fs::write(&thumb, b"webp").unwrap();
            thumb
        }
    }

    /// Answers the same drive kind for every path, so a test does not depend
    /// on which tempdir location a row happens to use.
    struct AnyDrive(Result<DriveKind, DriveTypeError>);

    impl DriveTypeDetector for AnyDrive {
        fn detect(&self, _folder: &Path) -> Result<DriveKind, DriveTypeError> {
            self.0.clone()
        }
    }

    fn group(ids: &[&str]) -> Vec<DuplicateGroup> {
        vec![DuplicateGroup {
            kind: DuplicateGroupKind::QuickHashConfirmed,
            members: ids
                .iter()
                .map(|id| DuplicateGroupMember {
                    video_id: id.to_string(),
                    file_path: String::new(),
                    file_name: String::new(),
                    file_size: 0,
                    status: String::new(),
                    created_at: String::new(),
                    duration: None,
                    width: None,
                    height: None,
                })
                .collect(),
        }]
    }

    fn trash_ok() -> FakeFileTrasher {
        FakeFileTrasher {
            result: Ok(()),
            ..Default::default()
        }
    }

    fn run(
        env: &Env,
        groups: Vec<DuplicateGroup>,
        drive: &impl DriveTypeDetector,
        trasher: &impl FileTrasher,
        notifier: &FakeCatalogNotifier,
        id: &str,
    ) -> Result<(), String> {
        delete_video_with_file(
            &env.db,
            groups,
            drive,
            trasher,
            Some(&env.thumbs),
            notifier,
            id,
        )
    }

    /// Asserts the call was refused: the trasher was never called, the row
    /// (with its tag) is intact, and no change was announced.
    fn assert_refused(
        env: &Env,
        trasher: &FakeFileTrasher,
        notifier: &FakeCatalogNotifier,
        result: &Result<(), String>,
        id: &str,
        expected: &str,
    ) {
        let err = result.as_ref().expect_err("must be refused");
        assert!(err.contains(expected), "{err}");
        assert!(trasher.calls.lock().unwrap().is_empty());
        assert!(env.row_exists(id));
        assert_eq!(env.tag_count(id), 1);
        assert_eq!(notifier.calls(), 0);
    }

    /// Fake trasher that records, at the moment of the move, whether the
    /// catalog row still existed -- proving the move precedes the delete.
    struct OrderProbe<'a> {
        env: &'a Env,
        id: &'a str,
        row_existed_at_move: AtomicBool,
        inner: FakeFileTrasher,
    }

    impl FileTrasher for OrderProbe<'_> {
        fn move_to_trash(&self, path: &Path) -> Result<(), TrashError> {
            self.row_existed_at_move
                .store(self.env.row_exists(self.id), Ordering::SeqCst);
            self.inner.move_to_trash(path)
        }
    }

    fn two_online(env: &Env) -> (PathBuf, PathBuf) {
        let a = env.add_video("a", "online");
        let b = env.add_video("b", "online");
        env.add_tag("a");
        env.add_tag("b");
        (a, b)
    }

    #[test]
    fn success_moves_to_trash_before_removing_the_catalog_entry() {
        let env = Env::new();
        let (a, b) = two_online(&env);
        let thumb = env.add_thumb("a", &a);
        let drive = AnyDrive(Ok(DriveKind::Local));
        let probe = OrderProbe {
            env: &env,
            id: "a",
            row_existed_at_move: AtomicBool::new(false),
            inner: trash_ok(),
        };
        let notifier = FakeCatalogNotifier::default();

        run(&env, group(&["a", "b"]), &drive, &probe, &notifier, "a").unwrap();

        assert!(probe.row_existed_at_move.load(Ordering::SeqCst));
        assert_eq!(
            probe.inner.calls.lock().unwrap().as_slice(),
            std::slice::from_ref(&a)
        );
        assert!(!env.row_exists("a"));
        assert_eq!(env.tag_count("a"), 0);
        assert!(!thumb.exists());
        assert_eq!(notifier.calls(), 1);
        // The other copy is untouched, and the fake never removes files.
        assert!(env.row_exists("b"));
        assert!(b.exists());
        assert!(a.exists());
    }

    #[test]
    fn failed_move_leaves_the_catalog_untouched() {
        let env = Env::new();
        let (a, _b) = two_online(&env);
        let thumb = env.add_thumb("a", &a);
        let drive = AnyDrive(Ok(DriveKind::Local));
        let trasher = FakeFileTrasher {
            result: Err(TrashError::Failed {
                path: "p".into(),
                message: "locked".into(),
            }),
            ..Default::default()
        };
        let notifier = FakeCatalogNotifier::default();

        let err = run(&env, group(&["a", "b"]), &drive, &trasher, &notifier, "a").unwrap_err();

        assert!(err.contains("ゴミ箱への移動に失敗しました"), "{err}");
        assert_eq!(trasher.calls.lock().unwrap().len(), 1);
        assert!(env.row_exists("a"));
        assert_eq!(env.tag_count("a"), 1);
        assert!(thumb.exists());
        assert_eq!(notifier.calls(), 0);
    }

    #[test]
    fn a_second_online_member_is_enough_even_if_others_are_offline() {
        let env = Env::new();
        two_online(&env);
        env.add_video("c", "offline");
        let drive = AnyDrive(Ok(DriveKind::Local));
        let trasher = trash_ok();
        let notifier = FakeCatalogNotifier::default();

        run(
            &env,
            group(&["a", "b", "c"]),
            &drive,
            &trasher,
            &notifier,
            "a",
        )
        .unwrap();
        assert_eq!(trasher.calls.lock().unwrap().len(), 1);
        assert!(!env.row_exists("a"));
    }

    fn refusal_case(
        setup: impl FnOnce(&Env) -> (Vec<DuplicateGroup>, Result<DriveKind, DriveTypeError>),
        target: &str,
        expected: &str,
    ) {
        let env = Env::new();
        let (groups, kind) = setup(&env);
        env.add_tag(target);
        let drive = AnyDrive(kind);
        let trasher = trash_ok();
        let notifier = FakeCatalogNotifier::default();
        let result = run(&env, groups, &drive, &trasher, &notifier, target);
        assert_refused(&env, &trasher, &notifier, &result, target, expected);
    }

    fn local() -> Result<DriveKind, DriveTypeError> {
        Ok(DriveKind::Local)
    }

    #[test]
    fn refuses_an_offline_member() {
        refusal_case(
            |env| {
                env.add_video("a", "offline");
                env.add_video("b", "online");
                env.add_video("c", "online");
                (group(&["a", "b", "c"]), local())
            },
            "a",
            "オフライン",
        );
    }

    #[test]
    fn refuses_network_removable_and_unknown_drives_and_detection_failure() {
        let cases = [
            Ok(DriveKind::Network),
            Ok(DriveKind::Removable),
            Ok(DriveKind::Unknown),
            Err(DriveTypeError::DetectionFailed {
                path: "p".into(),
                message: "m".into(),
            }),
        ];
        for kind in cases {
            refusal_case(
                |env| {
                    two_online(env);
                    // Tag is added by `refusal_case` too; keep one per row.
                    (group(&["a", "b"]), kind.clone())
                },
                "a",
                "ゴミ箱に移動できない場所",
            );
        }
    }

    #[test]
    fn refuses_a_drive_that_the_detector_was_not_configured_for() {
        let env = Env::new();
        two_online(&env);
        let drive = FakeDriveTypeDetector::default();
        let trasher = trash_ok();
        let notifier = FakeCatalogNotifier::default();
        let result = run(&env, group(&["a", "b"]), &drive, &trasher, &notifier, "a");
        assert_refused(&env, &trasher, &notifier, &result, "a", "ゴミ箱");
    }

    #[test]
    fn refuses_the_last_online_member_when_the_others_are_offline() {
        refusal_case(
            |env| {
                env.add_video("a", "online");
                env.add_video("b", "offline");
                env.add_video("c", "offline");
                (group(&["a", "b", "c"]), local())
            },
            "a",
            "最後の1件",
        );
    }

    #[test]
    fn refuses_a_group_with_only_the_target() {
        refusal_case(
            |env| {
                env.add_video("a", "online");
                (group(&["a"]), local())
            },
            "a",
            "最後の1件",
        );
    }

    #[test]
    fn refuses_a_file_outside_the_registered_folders() {
        refusal_case(
            |env| {
                env.add_video("b", "online");
                let other = env.dir.path().join("other");
                std::fs::create_dir_all(&other).unwrap();
                let out = other.join("a.mp4");
                std::fs::write(&out, CONTENT).unwrap();
                env.insert_row("a", &out, "online", CONTENT.len() as i64);
                (group(&["a", "b"]), local())
            },
            "a",
            "登録フォルダの外",
        );
    }

    #[test]
    fn a_sibling_folder_sharing_a_name_prefix_is_outside() {
        refusal_case(
            |env| {
                env.add_video("b", "online");
                let sibling = env.dir.path().join("lib2");
                std::fs::create_dir_all(&sibling).unwrap();
                let out = sibling.join("a.mp4");
                std::fs::write(&out, CONTENT).unwrap();
                env.insert_row("a", &out, "online", CONTENT.len() as i64);
                (group(&["a", "b"]), local())
            },
            "a",
            "登録フォルダの外",
        );
    }

    #[test]
    fn refuses_a_missing_file() {
        refusal_case(
            |env| {
                env.add_video("b", "online");
                let gone = env.folder.join("a.mp4");
                env.insert_row("a", &gone, "online", CONTENT.len() as i64);
                (group(&["a", "b"]), local())
            },
            "a",
            "見つからない",
        );
    }

    #[test]
    fn refuses_a_directory_at_the_recorded_path() {
        refusal_case(
            |env| {
                env.add_video("b", "online");
                let dir = env.folder.join("a.mp4");
                std::fs::create_dir_all(&dir).unwrap();
                env.insert_row("a", &dir, "online", 0);
                (group(&["a", "b"]), local())
            },
            "a",
            "通常のファイルではありません",
        );
    }

    #[test]
    fn refuses_a_size_mismatch() {
        for delta in [-1_i64, 1] {
            refusal_case(
                |env| {
                    env.add_video("b", "online");
                    let path = env.folder.join("a.mp4");
                    std::fs::write(&path, CONTENT).unwrap();
                    env.insert_row("a", &path, "online", CONTENT.len() as i64 + delta);
                    (group(&["a", "b"]), local())
                },
                "a",
                "再スキャン",
            );
        }
    }

    #[test]
    fn refuses_a_video_that_is_in_no_duplicate_group() {
        let env = Env::new();
        two_online(&env);
        let drive = AnyDrive(local());
        let trasher = trash_ok();
        let notifier = FakeCatalogNotifier::default();
        let result = run(&env, group(&["b", "c"]), &drive, &trasher, &notifier, "a");
        assert_refused(&env, &trasher, &notifier, &result, "a", "重複グループ");
        let result = run(&env, Vec::new(), &drive, &trasher, &notifier, "a");
        assert_refused(&env, &trasher, &notifier, &result, "a", "重複グループ");
    }

    #[test]
    fn refuses_an_unknown_video_id() {
        let env = Env::new();
        let drive = AnyDrive(local());
        let trasher = trash_ok();
        let notifier = FakeCatalogNotifier::default();
        let err = run(&env, group(&["x"]), &drive, &trasher, &notifier, "x").unwrap_err();
        assert!(err.contains("見つかりません"), "{err}");
        assert!(trasher.calls.lock().unwrap().is_empty());
        assert_eq!(notifier.calls(), 0);
    }

    #[test]
    fn catalog_only_removal_leaves_the_source_file_in_place() {
        let env = Env::new();
        let (a, _b) = two_online(&env);
        let thumb = env.add_thumb("a", &a);

        remove_video_from_catalog(&env.db, Some(&env.thumbs), "a").unwrap();

        assert!(!env.row_exists("a"));
        assert_eq!(env.tag_count("a"), 0);
        assert!(!thumb.exists());
        assert!(a.exists());
        assert!(env.row_exists("b"));
    }

    #[test]
    fn catalog_only_removal_without_a_thumbnail_root_still_removes_the_row() {
        let env = Env::new();
        let (a, _b) = two_online(&env);
        remove_video_from_catalog(&env.db, None, "a").unwrap();
        assert!(!env.row_exists("a"));
        assert!(a.exists());
    }
}
