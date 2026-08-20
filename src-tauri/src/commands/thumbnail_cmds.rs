use base64::{engine::general_purpose::STANDARD, Engine};
use std::path::Path;
use tauri::State;

use crate::db::{queries, Db};
use crate::thumbnail::paths as thumbnail_paths;
use crate::thumbnail::worker::THUMBNAILS_PER_VIDEO;
use crate::thumbnail::ThumbnailQueueHandle;

#[tauri::command]
pub fn toggle_thumbnail_pause(queue: State<ThumbnailQueueHandle>, paused: bool) {
    queue.set_paused(paused);
}

/// Reads all 6 `thumbnails/<folder-subdir>/[video_id]_0..5.webp` files and
/// returns them as `data:` URLs (in slot order), or `None` if even one of
/// the 6 hasn't been generated yet (or `video_id` doesn't resolve to a known
/// video at all) -- a video's thumbnails are all-or-nothing (see
/// `thumbnail::worker::generate_thumbnail_for_video`), so a partial set is
/// never meaningful to the caller. Deliberately not the Tauri asset
/// protocol: the thumbnails dir is portable (next to the exe, not a fixed OS
/// directory), so its absolute path can't be pinned in `tauri.conf.json`'s
/// static `assetProtocol.scope` at build time without a scope wide enough to
/// defeat its own purpose. A plain command needs no capability beyond the
/// `core:default` already granted.
///
/// The frontend-facing signature (`video_id` only) is unchanged since #6 --
/// `db: State<Db>` is a Tauri-managed dependency, not a caller-supplied
/// argument, so resolving `file_path` -> `watch_folders` -> the video's
/// resolved subdirectory here doesn't affect the IPC contract at all.
#[tauri::command]
pub fn get_thumbnails(db: State<Db>, video_id: String) -> Result<Option<Vec<String>>, String> {
    let file_path = queries::find_video_by_id(&db.read_pool, &video_id)
        .map_err(|e| e.to_string())?
        .map(|row| row.file_path);
    let Some(file_path) = file_path else {
        return Ok(None);
    };

    let thumbnails_root = crate::paths::app_data_dir()
        .map(|dir| thumbnail_paths::thumbnails_root(&dir))
        .map_err(|e| e.to_string())?;
    let watch_folders = {
        let conn = db.read_pool.get().map_err(|e| e.to_string())?;
        queries::get_watch_folders(&conn).map_err(|e| e.to_string())?
    };
    let video_dir =
        thumbnail_paths::video_thumbnail_dir(&thumbnails_root, &watch_folders, &file_path);
    read_thumbnails(&video_dir, &video_id)
}

fn read_thumbnails(thumbnails_dir: &Path, video_id: &str) -> Result<Option<Vec<String>>, String> {
    let paths: Vec<_> = (0..THUMBNAILS_PER_VIDEO)
        .map(|i| thumbnails_dir.join(format!("{video_id}_{i}.webp")))
        .collect();
    if !paths.iter().all(|p| p.exists()) {
        return Ok(None);
    }
    let mut data_urls = Vec::with_capacity(THUMBNAILS_PER_VIDEO);
    for path in &paths {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        data_urls.push(format!("data:image/webp;base64,{}", STANDARD.encode(bytes)));
    }
    Ok(Some(data_urls))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_all_six(dir: &Path, id: &str) {
        for i in 0..THUMBNAILS_PER_VIDEO {
            std::fs::write(
                dir.join(format!("{id}_{i}.webp")),
                format!("fake webp bytes {i}"),
            )
            .unwrap();
        }
    }

    #[test]
    fn returns_none_when_no_thumbnail_files_exist() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_thumbnails(dir.path(), "missing-id").unwrap(), None);
    }

    #[test]
    fn returns_none_when_only_some_of_the_six_files_exist() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..(THUMBNAILS_PER_VIDEO - 1) {
            std::fs::write(
                dir.path().join(format!("vid-1_{i}.webp")),
                format!("fake webp bytes {i}"),
            )
            .unwrap();
        }
        assert_eq!(
            read_thumbnails(dir.path(), "vid-1").unwrap(),
            None,
            "5 of 6 present must still be treated as not-ready"
        );
    }

    #[test]
    fn returns_six_data_urls_in_slot_order_when_all_files_exist() {
        let dir = tempfile::tempdir().unwrap();
        write_all_six(dir.path(), "vid-1");
        let result = read_thumbnails(dir.path(), "vid-1").unwrap();
        let expected: Vec<String> = (0..THUMBNAILS_PER_VIDEO)
            .map(|i| {
                format!(
                    "data:image/webp;base64,{}",
                    STANDARD.encode(format!("fake webp bytes {i}"))
                )
            })
            .collect();
        assert_eq!(result, Some(expected));
    }
}
