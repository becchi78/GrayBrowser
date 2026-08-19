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

    crate::watch::reconfigure_real_watch_manager(
        &app,
        db.inner(),
        &watch_manager,
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
/// their cached thumbnails, then drops `folder_path` from the persisted
/// `watch_folders` setting and re-`reconfigure`s the watch manager, mirroring
/// `pick_watch_folders`'s own "merge the folder list, persist it, then
/// immediately re-classify/restart watchers" shape (rather than only taking
/// effect at the next app start).
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
    let (remaining, nas_poll_interval_secs, deleted_ids) = {
        let mut conn = db.writer.lock().unwrap();
        let deleted_ids = queries::delete_videos_under_folder_cascade(&mut conn, &folder_path)
            .map_err(|e| e.to_string())?;
        let existing = queries::get_watch_folders(&conn).map_err(|e| e.to_string())?;
        let remaining: Vec<String> = existing.into_iter().filter(|f| f != &folder_path).collect();
        queries::set_watch_folders(&conn, &remaining).map_err(|e| e.to_string())?;
        let interval = queries::get_nas_poll_interval_secs(&conn).map_err(|e| e.to_string())?;
        (remaining, interval, deleted_ids)
        // `conn` (and its writer-lock guard) drops here, before reconfigure
        // does its own DB reads/watcher-startup work -- same ordering
        // `pick_watch_folders` uses, for the same reason.
    };

    // Best-effort, mirrors `dedup_cmds::delete_duplicate_video`'s own
    // thumbnail cleanup: a deleted video may never have had a thumbnail
    // generated (e.g. it was offline), so a missing file here is expected,
    // not an error.
    if let Ok(thumbnails_dir) = crate::paths::app_data_dir().map(|dir| dir.join("thumbnails")) {
        for id in &deleted_ids {
            // Each video now has 6 thumbnail slot files, not 1.
            for i in 0..crate::thumbnail::worker::THUMBNAILS_PER_VIDEO {
                let thumb_path = thumbnails_dir.join(format!("{id}_{i}.webp"));
                let _ = std::fs::remove_file(&thumb_path);
            }
        }
    }

    crate::watch::reconfigure_real_watch_manager(
        &app,
        db.inner(),
        &watch_manager,
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
    let (result, nas_poll_interval_secs) = {
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
        )
        // `conn` (and its writer-lock guard) drops here, before reconfigure
        // does its own DB reads/watcher-startup work -- same ordering
        // `pick_watch_folders`/`remove_watch_folder` use, for the same
        // reason.
    };

    crate::watch::reconfigure_real_watch_manager(
        &app,
        db.inner(),
        &watch_manager,
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
}
