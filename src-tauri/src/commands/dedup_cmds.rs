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
/// (all 6 `[video_id]_0.webp`..`[video_id]_5.webp` slot files, under
/// whichever registered-folder subdirectory the video's `file_path`
/// currently resolves to) if any exist.
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
    // Looked up *before* the DB delete below -- once the row is gone,
    // `file_path` (and therefore the resolved thumbnail subdirectory) is no
    // longer recoverable from the DB. A lookup failure/absence is not fatal
    // to this command: it just means the thumbnail-cleanup step below has
    // nothing to do.
    let file_path = crate::db::queries::find_video_by_id(&db.read_pool, &video_id)
        .ok()
        .flatten()
        .map(|row| row.file_path);

    if let (Some(file_path), Ok(app_dir)) = (file_path, crate::paths::app_data_dir()) {
        let thumbnails_root = crate::thumbnail::paths::thumbnails_root(&app_dir);
        let watch_folders = db
            .read_pool
            .get()
            .ok()
            .and_then(|conn| crate::db::queries::get_watch_folders(&conn).ok())
            .unwrap_or_default();
        let video_dir = crate::thumbnail::paths::video_thumbnail_dir(
            &thumbnails_root,
            &watch_folders,
            &file_path,
        );
        // Best-effort: the thumbnails may never have been generated (e.g.
        // an offline video), same "may not exist" reasoning as
        // thumbnail::worker's own tmp-file cleanup.
        for i in 0..crate::thumbnail::worker::THUMBNAILS_PER_VIDEO {
            let thumb_path = crate::thumbnail::paths::slot_path(&video_dir, &video_id, i);
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
