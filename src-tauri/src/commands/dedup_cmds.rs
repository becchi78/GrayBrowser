//! Duplicate-group listing/refresh/cleanup commands. Thin passthroughs to
//! `dedup::`/`db::queries` -- no additional detection logic lives here.

use std::sync::Arc;

use tauri::State;

use crate::db::Db;
use crate::dedup::{self, DuplicateGroup, DuplicateGroupsState};
use crate::events::{CatalogNotifier, TauriCatalogNotifier, TauriDedupNotifier};

/// Returns the most recently detected duplicate groups
/// (`dedup::refresh_duplicate_groups`'s last completed run's snapshot).
/// Does not itself trigger detection -- call `refresh_duplicate_groups` for
/// that; this just reads `DuplicateGroupsState`.
#[tauri::command]
pub fn list_duplicate_groups(state: State<DuplicateGroupsState>) -> Vec<DuplicateGroup> {
    state.get()
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

/// Removes a video from the catalog -- its `videos` row plus every
/// `video_tags`/`path_collisions` row referencing it
/// (`queries::delete_video_cascade`) -- and deletes its cached thumbnails
/// (`thumbnails/[video_id]_0.webp`..`[video_id]_5.webp`) if any exist.
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
#[tauri::command]
pub fn delete_duplicate_video(
    app: tauri::AppHandle,
    db: State<Db>,
    video_id: String,
) -> Result<(), String> {
    if let Ok(thumbnails_dir) = crate::paths::app_data_dir().map(|dir| dir.join("thumbnails")) {
        // Best-effort: the thumbnails may never have been generated (e.g. an
        // offline video), same "may not exist" reasoning as
        // thumbnail::worker's own tmp-file cleanup.
        for i in 0..crate::thumbnail::worker::THUMBNAILS_PER_VIDEO {
            let thumb_path = thumbnails_dir.join(format!("{video_id}_{i}.webp"));
            let _ = std::fs::remove_file(&thumb_path);
        }
    }

    {
        let mut conn = db.writer.lock().unwrap();
        crate::db::queries::delete_video_cascade(&mut conn, &video_id)
            .map_err(|e| e.to_string())?;
    }

    TauriCatalogNotifier::new(app).notify_changed();
    Ok(())
}
