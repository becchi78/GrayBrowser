use gb_core::ports::dialog::FolderPicker;
use tauri::State;

use crate::adapters;
use crate::db::{queries, Db};
use crate::events::{CatalogNotifier, TauriCatalogNotifier};
use crate::watch::RealtimeWatchManager;

/// Pure-ish mapping: given a picker result and the existing folder list,
/// decides the merged list. Kept separate from the command body so it can
/// be unit-tested against `FakeFolderPicker` without opening a real dialog.
fn pick_and_merge(
    picker: &impl FolderPicker,
    existing: Vec<String>,
) -> Result<Vec<String>, String> {
    match picker.pick_folders().map_err(|e| e.to_string())? {
        None => Ok(existing), // user cancelled: leave the list unchanged
        Some(picked) => {
            let picked: Vec<String> = picked
                .into_iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            Ok(gb_core::watch_folders::merge(existing, picked))
        }
    }
}

/// Re-classifying and restarting watchers/pollers (`RealtimeWatchManager::
/// reconfigure`) is what closes the "restart required to pick up a new
/// folder" gap: every successful pick immediately routes the merged folder
/// list through drive-type detection again, not just at the next app start.
#[tauri::command]
pub fn pick_watch_folders(
    app: tauri::AppHandle,
    db: State<Db>,
    watch_manager: State<RealtimeWatchManager>,
) -> Result<Vec<String>, String> {
    let (merged, nas_poll_interval_secs) = {
        let conn = db.writer.lock().unwrap();
        let existing = queries::get_watch_folders(&conn).map_err(|e| e.to_string())?;
        let picker = adapters::dialog::RealFolderPicker::new(app.clone());
        let merged = pick_and_merge(&picker, existing)
            .inspect_err(|e| log::warn!("folder picker failed: {e}"))?;
        queries::set_watch_folders(&conn, &merged).map_err(|e| e.to_string())?;
        let interval = queries::get_nas_poll_interval_secs(&conn).map_err(|e| e.to_string())?;
        (merged, interval)
        // `conn` (and its writer-lock guard) drops here, before reconfigure
        // does its own DB reads/watcher-startup work.
    };

    let thumbnails_root = crate::thumbnail::paths::thumbnails_root(
        &crate::paths::app_data_dir().map_err(|e| e.to_string())?,
    );
    crate::watch::reconfigure_real_watch_manager(
        &app,
        db.inner(),
        &watch_manager,
        &thumbnails_root,
        &merged,
        nas_poll_interval_secs,
    );

    Ok(merged)
}

#[tauri::command]
pub fn list_watch_folders(db: State<Db>) -> Result<Vec<String>, String> {
    let conn = db.read_pool.get().map_err(|e| e.to_string())?;
    queries::get_watch_folders(&conn).map_err(|e| e.to_string())
}

/// Read-only precursor to `remove_watch_folder`: counts how many `videos`
/// rows would be lost so `FolderDialog`'s ✕ delete confirmation can
/// show a real number *before* the user commits to the destructive action.
#[tauri::command]
pub fn count_videos_under_folder(db: State<Db>, folder_path: String) -> Result<u32, String> {
    queries::count_videos_under_folder(&db.read_pool, &folder_path).map_err(|e| e.to_string())
}

/// `FolderDialog`'s ✕ delete action: removes every `videos` row under
/// `folder_path` -- plus every
/// `video_tags`/`path_collisions` row referencing one of them, all in one
/// transaction (`queries::delete_videos_under_folder_cascade`) -- deletes
/// their cached thumbnails (the whole per-folder subdirectory at once, since
/// every video under it is gone), then drops `folder_path` from the
/// persisted `watch_folders` setting and re-`reconfigure`s the watch
/// manager, mirroring `pick_watch_folders`'s own "merge the folder list,
/// persist it, then immediately re-classify/restart watchers" shape (rather
/// than only taking effect at the next app start).
///
/// Returns the remaining watch folder list, same shape as
/// `pick_watch_folders`'s return value.
#[tauri::command]
pub fn remove_watch_folder(
    app: tauri::AppHandle,
    db: State<Db>,
    watch_manager: State<RealtimeWatchManager>,
    folder_path: String,
) -> Result<Vec<String>, String> {
    // Resolved *before* any DB write below (mirrors `dedup_cmds::
    // delete_duplicate_video`/`scan_cmds::start_scan`'s ordering): this
    // command's whole point is the DB mutation that follows, so if the
    // `thumbnails/` root can't even be resolved, the command must fail
    // before touching the DB at all -- never after, which would otherwise
    // leave the DB already changed while the command still returns `Err`
    // (and `notify_changed` never fires, so the frontend never learns the
    // DB actually changed).
    let thumbnails_root = crate::thumbnail::paths::thumbnails_root(
        &crate::paths::app_data_dir().map_err(|e| e.to_string())?,
    );

    let (remaining, nas_poll_interval_secs) = {
        let mut conn = db.writer.lock().unwrap();
        // The deleted ids themselves are no longer needed here (see
        // `delete_videos_under_folder_cascade`'s doc comment) -- thumbnail
        // cleanup below removes the whole per-folder subdirectory in one
        // shot instead of looping per id.
        let _ = queries::delete_videos_under_folder_cascade(&mut conn, &folder_path)
            .map_err(|e| e.to_string())?;
        let existing = queries::get_watch_folders(&conn).map_err(|e| e.to_string())?;
        let remaining: Vec<String> = existing.into_iter().filter(|f| f != &folder_path).collect();
        queries::set_watch_folders(&conn, &remaining).map_err(|e| e.to_string())?;
        let interval = queries::get_nas_poll_interval_secs(&conn).map_err(|e| e.to_string())?;
        (remaining, interval)
        // `conn` (and its writer-lock guard) drops here, before reconfigure
        // does its own DB reads/watcher-startup work -- same ordering
        // `pick_watch_folders` uses, for the same reason.
    };

    // Best-effort, same "a deleted video may never have had a thumbnail
    // generated" reasoning as `dedup_cmds::delete_duplicate_video`'s own
    // cleanup -- a missing/already-absent subdirectory is not an error.
    if !crate::thumbnail::paths::remove_folder_thumbnail_dir(&thumbnails_root, &folder_path) {
        log::warn!("failed to fully remove the thumbnail directory for folder {folder_path}");
    }

    crate::watch::reconfigure_real_watch_manager(
        &app,
        db.inner(),
        &watch_manager,
        &thumbnails_root,
        &remaining,
        nas_poll_interval_secs,
    );

    // The removed videos' rows are already gone from the DB by this point --
    // unlike `pick_watch_folders` (which only adds folders, so the grid's
    // existing content is still valid until a future scan/watch event
    // refreshes it), this command's whole point is an immediate, visible
    // change to `videos`, so the frontend must be told to refetch right now
    // rather than waiting for whatever watcher/poller activity happens next.
    TauriCatalogNotifier::new(app).notify_changed();

    Ok(remaining)
}

/// Return payload for `rename_watch_folder`: the updated folder list plus
/// how many `videos` rows were rewritten vs. left untouched due to a path
/// collision, so the frontend can surface both without a second round trip.
#[derive(serde::Serialize)]
pub struct RenameWatchFolderResult {
    pub folders: Vec<String>,
    pub renamed_count: u32,
    pub collision_skipped_count: u32,
}

/// Pure pre-flight validation for `rename_watch_folder`, kept separate from
/// the command body (mirrors `pick_and_merge`'s split) so it is unit-
/// testable without a Tauri `State`/`AppHandle`. Returns `Err` describing why
/// the rename must be rejected *before* touching the DB at all:
///
/// - `old_folder_path` must actually be a currently-registered watch folder
///   (guards against a stale/mistyped row triggering a folder-boundary
///   `LIKE` rewrite scoped to a path nobody is actually watching);
/// - `new_folder_path` must not duplicate or overlap (parent/child, either
///   direction) an *other* already-registered watch folder
///   (`gb_core::paths::folder_paths_conflict`). Renaming a folder to
///   itself (`new_folder_path` unchanged) is *not* rejected, since that
///   overlap is with itself, not an "other" folder.
fn validate_rename_target(
    existing: &[String],
    old_folder_path: &str,
    new_folder_path: &str,
) -> Result<(), String> {
    if !existing.iter().any(|f| f == old_folder_path) {
        return Err(format!(
            "登録済みの監視フォルダではありません: \"{old_folder_path}\""
        ));
    }
    if existing
        .iter()
        .any(|f| f != old_folder_path && gb_core::paths::folder_paths_conflict(f, new_folder_path))
    {
        return Err(format!(
            "既存の登録済みフォルダと重複または包含関係にあります: \"{new_folder_path}\""
        ));
    }
    Ok(())
}

/// `FolderDialog`'s ✎ path edit action: rewrites every
/// `videos.file_path` under `old_folder_path` to fall under
/// `new_folder_path` instead (`queries::
/// rename_watch_folder_videos`), then replaces `old_folder_path` in the
/// persisted `watch_folders` setting with `new_folder_path`, all under one
/// `db.writer.lock()` acquisition, mirroring `remove_watch_folder`'s
/// "mutate videos, then persist the folder-list setting, then re-
/// `reconfigure` the watch manager" shape.
#[tauri::command]
pub fn rename_watch_folder(
    app: tauri::AppHandle,
    db: State<Db>,
    watch_manager: State<RealtimeWatchManager>,
    old_folder_path: String,
    new_folder_path: String,
) -> Result<RenameWatchFolderResult, String> {
    // Resolved *before* any DB write below, same reasoning as
    // `remove_watch_folder`: this command's DB mutation is its main point,
    // so a `thumbnails/` root resolution failure must abort before touching
    // the DB, never after (which would otherwise leave the DB already
    // renamed while the command still returns `Err` and `notify_changed`
    // never fires).
    let thumbnails_root = crate::thumbnail::paths::thumbnails_root(
        &crate::paths::app_data_dir().map_err(|e| e.to_string())?,
    );

    let (result, nas_poll_interval_secs, renamed_videos) = {
        let mut conn = db.writer.lock().unwrap();
        let existing = queries::get_watch_folders(&conn).map_err(|e| e.to_string())?;
        validate_rename_target(&existing, &old_folder_path, &new_folder_path)?;

        let outcome =
            queries::rename_watch_folder_videos(&mut conn, &old_folder_path, &new_folder_path)
                .map_err(|e| e.to_string())?;

        let updated: Vec<String> = existing
            .into_iter()
            .map(|f| {
                if f == old_folder_path {
                    new_folder_path.clone()
                } else {
                    f
                }
            })
            .collect();
        queries::set_watch_folders(&conn, &updated).map_err(|e| e.to_string())?;
        let interval = queries::get_nas_poll_interval_secs(&conn).map_err(|e| e.to_string())?;

        (
            RenameWatchFolderResult {
                folders: updated,
                renamed_count: outcome.renamed_count,
                collision_skipped_count: outcome.collision_skipped_count,
            },
            interval,
            outcome.renamed_videos,
        )
        // `conn` (and its writer-lock guard) drops here, before reconfigure
        // does its own DB reads/watcher-startup work -- same ordering
        // `pick_watch_folders`/`remove_watch_folder` use, for the same
        // reason.
    };

    // Video-by-video, never a single whole-subdirectory rename of the old
    // folder's thumbnail subdirectory to the new one -- see
    // `queries::RenameWatchFolderOutcome`'s doc comment for why: a
    // collision-skipped video's `file_path` never actually changed, so its
    // thumbnails must stay exactly where they already are, not get dragged
    // along with the videos that did rename.
    for video in &renamed_videos {
        if !crate::thumbnail::paths::move_video_thumbnails_between_folders(
            &thumbnails_root,
            &video.video_id,
            &old_folder_path,
            &new_folder_path,
        ) {
            log::warn!(
                "failed to fully move thumbnails for video {} from {old_folder_path} to \
                 {new_folder_path}",
                video.video_id
            );
        }
    }
    // Best-effort cleanup of the now-empty old subdirectory. Non-recursive
    // (`remove_dir`, not `remove_dir_all`): silently no-ops if anything is
    // still in there (e.g. a collision-skipped video's thumbnails, or a
    // move failure above) rather than risk deleting thumbnails that still
    // belong to a video whose `file_path` didn't actually change.
    let old_thumbnail_dir =
        thumbnails_root.join(gb_core::paths::thumbnail_folder_subdir(&old_folder_path));
    let _ = std::fs::remove_dir(&old_thumbnail_dir);

    crate::watch::reconfigure_real_watch_manager(
        &app,
        db.inner(),
        &watch_manager,
        &thumbnails_root,
        &result.folders,
        nas_poll_interval_secs,
    );

    // Same reasoning as `remove_watch_folder`: `file_path`/`status` changes
    // are already committed by this point, so the grid must refetch now
    // rather than wait for the next watcher/poller event.
    TauriCatalogNotifier::new(app).notify_changed();

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gb_core::ports::dialog::DialogError;
    use gb_core::testing::fake_dialog::FakeFolderPicker;
    use std::path::PathBuf;

    #[test]
    fn cancelled_pick_leaves_existing_list_unchanged() {
        let picker = FakeFolderPicker {
            result: Ok(None),
            ..Default::default()
        };
        let existing = vec!["C:/Videos".to_string()];
        assert_eq!(pick_and_merge(&picker, existing.clone()).unwrap(), existing);
    }

    #[test]
    fn picked_folders_are_merged_into_existing() {
        let picker = FakeFolderPicker {
            result: Ok(Some(vec![PathBuf::from("D:/Movies")])),
            ..Default::default()
        };
        let existing = vec!["C:/Videos".to_string()];
        assert_eq!(
            pick_and_merge(&picker, existing).unwrap(),
            vec!["C:/Videos", "D:/Movies"]
        );
    }

    #[test]
    fn dialog_failure_is_propagated_as_an_error() {
        let picker = FakeFolderPicker {
            result: Err(DialogError::Failed("boom".into())),
            ..Default::default()
        };
        assert!(pick_and_merge(&picker, vec![]).is_err());
    }

    // --- validate_rename_target ----------------------------------------

    #[test]
    fn rejects_renaming_a_folder_that_is_not_registered() {
        let existing = vec![r"C:\Videos".to_string()];
        assert!(validate_rename_target(&existing, r"C:\NotWatched", r"D:\Movies").is_err());
    }

    #[test]
    fn rejects_a_new_path_that_exactly_duplicates_another_watch_folder() {
        let existing = vec![r"C:\Videos".to_string(), r"D:\Movies".to_string()];
        assert!(validate_rename_target(&existing, r"C:\Videos", r"D:\Movies").is_err());
    }

    #[test]
    fn rejects_a_new_path_that_is_a_child_of_another_watch_folder() {
        let existing = vec![r"C:\Videos".to_string(), r"D:\Movies".to_string()];
        assert!(validate_rename_target(&existing, r"C:\Videos", r"D:\Movies\Sub").is_err());
    }

    #[test]
    fn rejects_a_new_path_that_is_a_parent_of_another_watch_folder() {
        let existing = vec![r"C:\Videos".to_string(), r"D:\Movies\Sub".to_string()];
        assert!(validate_rename_target(&existing, r"C:\Videos", r"D:\Movies").is_err());
    }

    #[test]
    fn allows_a_new_path_that_only_shares_a_string_prefix_with_another_watch_folder() {
        let existing = vec![r"C:\Videos".to_string(), r"D:\Movies".to_string()];
        assert!(validate_rename_target(&existing, r"C:\Videos", r"D:\Movies2").is_ok());
    }

    #[test]
    fn allows_renaming_a_folder_to_its_own_unchanged_path() {
        let existing = vec![r"C:\Videos".to_string()];
        assert!(validate_rename_target(&existing, r"C:\Videos", r"C:\Videos").is_ok());
    }

    #[test]
    fn allows_an_otherwise_non_conflicting_rename() {
        let existing = vec![r"C:\Videos".to_string(), r"D:\Movies".to_string()];
        assert!(validate_rename_target(&existing, r"C:\Videos", r"E:\Renamed").is_ok());
    }

    // --- rename_watch_folder's thumbnail-moving sequence -------------------
    //
    // `rename_watch_folder` itself is a `#[tauri::command]` that needs a real
    // `tauri::AppHandle` to notify with -- this codebase's own `events`
    // module documents constructing one as unsafe under `cargo test` on
    // Windows (`STATUS_ENTRYPOINT_NOT_FOUND`, see `events.rs`'s top doc
    // comment), the same reason `generation_retry_cmds`'s tests bypass their
    // own `#[tauri::command]` wrappers. So this instead exercises the exact
    // same sequence the command body performs -- `queries::
    // rename_watch_folder_videos` followed by moving only the videos its
    // `renamed_videos` list reports, one at a time -- directly against
    // `queries`/`thumbnail::paths`, with no `AppHandle` involved.

    /// The most important regression test in this module: a collision-
    /// skipped video's `file_path` never actually changes, so its cached
    /// thumbnails must be left exactly where they already are -- never
    /// moved to the new folder, which is exactly what a (forbidden) whole-
    /// subdirectory rename would have wrongly done to it. Only the video
    /// that *did* rename gets its thumbnails moved.
    #[test]
    fn rename_leaves_a_collision_skipped_videos_thumbnails_untouched_and_moves_only_the_renamed_one(
    ) {
        let (_db_dir, db) = crate::db::test_support::init_temp_db();
        let thumbs_root = tempfile::tempdir().unwrap();
        let old_folder = r"C:\OldVideos".to_string();
        let new_folder = r"D:\NewVideos".to_string();

        // "clashing" will collide with "outside" (which already occupies
        // the rename's target path) and must be skipped; "free" renames
        // normally.
        {
            let conn = db.writer.lock().unwrap();
            conn.execute(
                "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status) \
                 VALUES ('clashing', 'C:\\OldVideos\\taken.mp4', 'taken.mp4', 1, 'h', 'online')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status) \
                 VALUES ('free', 'C:\\OldVideos\\free.mp4', 'free.mp4', 1, 'h', 'online')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status) \
                 VALUES ('outside', 'D:\\NewVideos\\taken.mp4', 'taken.mp4', 1, 'h', 'online')",
                [],
            )
            .unwrap();
        }

        // Both "clashing" and "free" have cached thumbnails sitting in the
        // (shared, since both are under old_folder) old subdirectory before
        // the rename.
        let old_dir = thumbs_root
            .path()
            .join(gb_core::paths::thumbnail_folder_subdir(&old_folder));
        std::fs::create_dir_all(&old_dir).unwrap();
        for id in ["clashing", "free"] {
            for i in 0..crate::thumbnail::worker::THUMBNAILS_PER_VIDEO {
                std::fs::write(
                    old_dir.join(format!("{id}_{i}.webp")),
                    format!("{id}-slot-{i}"),
                )
                .unwrap();
            }
        }

        let outcome = {
            let mut conn = db.writer.lock().unwrap();
            queries::rename_watch_folder_videos(&mut conn, &old_folder, &new_folder).unwrap()
        };
        assert_eq!(outcome.renamed_count, 1, "only \"free\" should be renamed");
        assert_eq!(
            outcome.collision_skipped_count, 1,
            "\"clashing\" must be skipped"
        );

        // Mirrors rename_watch_folder's actual loop: only the reported
        // renamed_videos are moved, one video at a time -- never a whole-
        // subdirectory rename.
        for video in &outcome.renamed_videos {
            assert!(
                crate::thumbnail::paths::move_video_thumbnails_between_folders(
                    thumbs_root.path(),
                    &video.video_id,
                    &old_folder,
                    &new_folder,
                )
            );
        }

        let new_dir = thumbs_root
            .path()
            .join(gb_core::paths::thumbnail_folder_subdir(&new_folder));

        // "free" (actually renamed) must have moved to the new subdirectory.
        for i in 0..crate::thumbnail::worker::THUMBNAILS_PER_VIDEO {
            assert!(
                !old_dir.join(format!("free_{i}.webp")).exists(),
                "free's slot {i} must no longer be in the old folder's subdirectory"
            );
            assert_eq!(
                std::fs::read_to_string(new_dir.join(format!("free_{i}.webp"))).unwrap(),
                format!("free-slot-{i}")
            );
        }

        // "clashing" (collision-skipped) must be left exactly where it was:
        // still fully present in the old subdirectory, and absent from the
        // new one -- the core guarantee this test exists to protect.
        for i in 0..crate::thumbnail::worker::THUMBNAILS_PER_VIDEO {
            assert!(
                old_dir.join(format!("clashing_{i}.webp")).exists(),
                "clashing's slot {i} must remain in the old folder's subdirectory"
            );
            assert_eq!(
                std::fs::read_to_string(old_dir.join(format!("clashing_{i}.webp"))).unwrap(),
                format!("clashing-slot-{i}")
            );
            assert!(
                !new_dir.join(format!("clashing_{i}.webp")).exists(),
                "clashing's slot {i} must NOT have been moved to the new folder's subdirectory"
            );
        }
    }
}
