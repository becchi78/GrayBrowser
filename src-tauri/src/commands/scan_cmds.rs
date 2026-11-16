use std::sync::Arc;

use gb_core::ports::path_existence::PathExistence;
use gb_core::presence::Presence;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::adapters;
use crate::db::{queries, Db};
use crate::dedup::{self, DuplicateGroupsState};
use crate::events::{CatalogNotifier, TauriCatalogNotifier, TauriDedupNotifier};
use crate::presence::presence_of;
use crate::reachability::ReachabilityRegistry;
use crate::scan;
use crate::scan::reachable::scan_reachable_folders;
use crate::thumbnail::{self, ThumbnailQueueHandle};

#[derive(Serialize)]
pub struct VideoDto {
    pub id: String,
    pub file_path: String,
    pub file_name: String,
    pub file_size: i64,
    pub duration: Option<i64>,
    pub status: String,
    pub rating: i64,
    pub thumbnail_ready: bool,
    /// Automatic retries are used up and there is still no thumbnail, so the
    /// row should say it failed instead of "generating" forever.
    pub thumbnail_failed: bool,
}

impl VideoDto {
    /// `thumbnail_ready` is read straight off `VideoRow.thumbnail_ready`
    /// (a DB column kept in sync when thumbnails are written) -- **no
    /// filesystem `stat()` call happens here**.
    /// Checking whether `thumbnails/[id].webp` exists on every call would cost
    /// up to 100,000 stat() calls for a 100k-video full-library browse, which
    /// would dominate `list_videos`, not the LIKE-based search query itself.
    /// The DB column is the source of truth for this hot path specifically --
    /// it is *not* a blanket replacement of the app's stateless-resume design:
    /// `thumbnail::worker::list_videos_missing_thumbnails` (the "what needs
    /// (re)generating" resume pass that runs once per scan/startup, not on
    /// every browse) still treats the filesystem itself as authoritative,
    /// and self-heals this column when it's stale.
    fn from_row(row: queries::VideoRow) -> Self {
        let thumbnail_failed = !row.thumbnail_ready
            && !gb_core::retry::is_eligible_for_automatic_retry(
                u32::try_from(row.thumbnail_attempts).unwrap_or(0),
            );
        Self {
            id: row.id,
            file_path: row.file_path,
            file_name: row.file_name,
            file_size: row.file_size,
            duration: row.duration,
            status: row.status,
            rating: row.rating,
            thumbnail_ready: row.thumbnail_ready,
            thumbnail_failed,
        }
    }
}

/// IPC-boundary mirror of `gb_core::sort::SortField` -- serde does the raw
/// frontend-string -> enum parsing/validation here (an unrecognized string
/// is a deserialization error, not something that reaches SQL text), and
/// this type's sole job is converting into the closed gb-core enum that
/// `queries::list_videos_filtered` actually consumes.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortFieldParam {
    FileName,
    CreatedAt,
    UpdatedDate,
    Rating,
    FileSize,
    Folder,
}

impl From<SortFieldParam> for gb_core::sort::SortField {
    fn from(value: SortFieldParam) -> Self {
        match value {
            SortFieldParam::FileName => gb_core::sort::SortField::FileName,
            SortFieldParam::CreatedAt => gb_core::sort::SortField::CreatedAt,
            SortFieldParam::UpdatedDate => gb_core::sort::SortField::UpdatedDate,
            SortFieldParam::Rating => gb_core::sort::SortField::Rating,
            SortFieldParam::FileSize => gb_core::sort::SortField::FileSize,
            SortFieldParam::Folder => gb_core::sort::SortField::Folder,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirectionParam {
    Asc,
    Desc,
}

impl From<SortDirectionParam> for gb_core::sort::SortDirection {
    fn from(value: SortDirectionParam) -> Self {
        match value {
            SortDirectionParam::Asc => gb_core::sort::SortDirection::Asc,
            SortDirectionParam::Desc => gb_core::sort::SortDirection::Desc,
        }
    }
}

#[derive(Serialize)]
pub struct SkippedFileDto {
    pub id: i64,
    pub file_path: String,
    pub file_name: String,
    pub reason: String,
    pub detected_char: Option<String>,
    /// The file is verifiably gone (not merely unreachable), so the row can
    /// be dismissed but not renamed.
    pub file_missing: bool,
}

impl SkippedFileDto {
    fn from_row(row: queries::SkippedFileRow, file_missing: bool) -> Self {
        Self {
            id: row.id,
            file_path: row.file_path,
            file_name: row.file_name,
            reason: row.reason,
            detected_char: row.detected_char,
            file_missing,
        }
    }
}

/// Watch folders a manual scan covers: those whose update check is on. A
/// folder with the check off is meant to stay frozen, so a manual scan must
/// not touch it either; re-enabling the check is how its newer files get
/// picked up.
fn load_scan_target_folders(conn: &rusqlite::Connection) -> anyhow::Result<Vec<String>> {
    let folders = queries::get_watch_folders(conn)?;
    let disabled = queries::get_watch_check_disabled(conn)?;
    Ok(gb_core::watch_folders::filter_checked(&folders, &disabled))
}

/// `async` so the (blocking) walk+hash+DB work runs off Tauri's main event
/// loop thread and doesn't freeze the UI while a scan is in progress.
#[tauri::command]
pub async fn start_scan(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    queue: State<'_, ThumbnailQueueHandle>,
    dedup_state: State<'_, DuplicateGroupsState>,
    reachability: State<'_, ReachabilityRegistry>,
) -> Result<scan::ScanSummary, String> {
    let folders = {
        let conn = db.writer.lock().unwrap();
        load_scan_target_folders(&conn).map_err(|e| e.to_string())?
    };
    let thumbnails_root =
        crate::thumbnail::paths::resolve_thumbnails_root().map_err(|e| e.to_string())?;
    let summary = scan_reachable_folders(&folders, &reachability, &db, &thumbnails_root)
        .map_err(|e| e.to_string())?;
    // Cloned before `app` moves into `TauriCatalogNotifier` below --
    // `TauriDedupNotifier` needs its own `AppHandle` for the fire-and-forget
    // dedup refresh further down.
    let dedup_app_handle = app.clone();
    let notifier = Arc::new(TauriCatalogNotifier::new(app));
    // One notification for the scan itself, regardless of how many
    // files it touched -- distinct from the per-thumbnail notifications
    // enqueue_missing_thumbnails fires later as generation completes in the
    // background.
    notifier.notify_changed();

    // Fire-and-forget: re-enumerate videos missing a thumbnail and kick off
    // generation in the background (a "resume after scan" pass). This
    // does not block the response to the frontend.
    let _ = std::fs::create_dir_all(&thumbnails_root);
    thumbnail::enqueue_missing_thumbnails(
        db.inner().clone(),
        thumbnails_root,
        queue.inner().clone(),
        Arc::new(adapters::ffmpeg::RealFfmpegAdapter),
        Arc::clone(&notifier),
        reachability.inner().clone(),
    );

    // Fire-and-forget, same "resume after scan" reasoning as above.
    crate::metadata::enqueue_missing_metadata_probes(
        db.inner().clone(),
        Arc::new(adapters::ffmpeg::RealFfmpegAdapter),
        notifier,
        reachability.inner().clone(),
    );

    // Fire-and-forget, same "resume after scan" reasoning: a scan can
    // change quick_hash values and online/offline status, both of which
    // duplicate detection depends on.
    dedup::refresh_duplicate_groups(
        db.inner().clone(),
        dedup_state.inner().clone(),
        Arc::new(TauriDedupNotifier::new(dedup_app_handle)),
        reachability.inner().clone(),
    );

    Ok(summary)
}

/// `search` is the raw, unparsed search-box string (term-splitting happens
/// in `gb_core::search::parse_search_terms`, not on the frontend).
/// `sort_field`/`sort_direction` default to the original behavior
/// (`created_at DESC`) when omitted, so existing callers that don't pass
/// them see no change. `tag_ids` is an AND filter: a video must carry
/// every listed tag.
/// `folder_path`, when present, restricts results to videos whose path
/// falls under that folder (the folder sidebar) -- if the frontend never
/// passes it, this argument is a no-op (matches every row, same as
/// omitting it).
/// `ratings`, when non-empty, restricts results to videos whose `rating`
/// equals any listed value exactly (the rating bar; OR) -- `None` or an empty
/// list is a no-op (matches every row, same as omitting it).
#[tauri::command]
pub fn list_videos(
    db: State<Db>,
    search: Option<String>,
    sort_field: Option<SortFieldParam>,
    sort_direction: Option<SortDirectionParam>,
    tag_ids: Option<Vec<i64>>,
    folder_path: Option<String>,
    ratings: Option<Vec<u8>>,
) -> Result<Vec<VideoDto>, String> {
    let terms = search
        .as_deref()
        .map(gb_core::search::parse_search_terms)
        .unwrap_or_default();
    let field = sort_field
        .map(Into::into)
        .unwrap_or(gb_core::sort::SortField::CreatedAt);
    let direction = sort_direction
        .map(Into::into)
        .unwrap_or(gb_core::sort::SortDirection::Desc);
    let tag_ids = tag_ids.unwrap_or_default();
    let ratings = ratings
        .unwrap_or_default()
        .into_iter()
        .map(super::rating_cmds::validate_rating_arg)
        .collect::<Result<Vec<u8>, String>>()?;

    queries::list_videos_filtered(
        &db.read_pool,
        &terms,
        field,
        direction,
        &tag_ids,
        folder_path.as_deref(),
        &ratings,
    )
    .map(|rows| rows.into_iter().map(VideoDto::from_row).collect())
    .map_err(|e| e.to_string())
}

fn list_skipped_files_impl(
    db: &Db,
    reachability: &ReachabilityRegistry,
    fs: &dyn PathExistence,
) -> Result<Vec<SkippedFileDto>, String> {
    let rows = queries::list_skipped_files(&db.read_pool).map_err(|e| e.to_string())?;
    let watch_folders = queries::read_watch_folders(&db.read_pool).map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let presence = presence_of(&row.file_path, &watch_folders, reachability, fs);
            SkippedFileDto::from_row(row, presence == Presence::Missing)
        })
        .collect())
}

/// `async` because the per-row existence check touches the file system, which
/// can stall on a share that has just gone away.
#[tauri::command(async)]
pub fn list_skipped_files(
    db: State<'_, Db>,
    reachability: State<'_, ReachabilityRegistry>,
) -> Result<Vec<SkippedFileDto>, String> {
    list_skipped_files_impl(
        &db,
        &reachability,
        &adapters::path_existence::RealPathExistence,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{registry_over, strings};

    #[test]
    fn scan_targets_leave_out_the_folders_with_the_update_check_off() {
        let (_d, db) = crate::db::test_support::init_temp_db();
        let conn = db.writer.lock().unwrap();
        queries::set_watch_folders(&conn, &strings(&["A", "B", "C"])).unwrap();
        assert_eq!(
            load_scan_target_folders(&conn).unwrap(),
            strings(&["A", "B", "C"]),
            "nothing unchecked: every folder"
        );

        queries::set_watch_check_disabled(&conn, &strings(&["B"])).unwrap();
        assert_eq!(
            load_scan_target_folders(&conn).unwrap(),
            strings(&["A", "C"])
        );

        queries::set_watch_check_disabled(&conn, &strings(&["A", "B", "C"])).unwrap();
        assert!(load_scan_target_folders(&conn).unwrap().is_empty());
    }

    fn video_row(thumbnail_ready: bool) -> queries::VideoRow {
        queries::VideoRow {
            id: "vid-1".to_string(),
            file_path: "C:/videos/a.mp4".to_string(),
            file_name: "a.mp4".to_string(),
            file_size: 1234,
            duration: None,
            quick_hash: "h".to_string(),
            full_hash: None,
            status: "online".to_string(),
            rating: 0,
            created_at: "2026-01-01 00:00:00".to_string(),
            mtime: None,
            thumbnail_ready,
            thumbnail_attempts: 0,
        }
    }

    #[test]
    fn from_row_derives_thumbnail_failed_and_carries_the_ready_flag_through() {
        let max = i64::from(gb_core::retry::MAX_GENERATION_ATTEMPTS);
        let cases = [
            (false, 0, false),
            (false, max - 1, false),
            (false, max, true),
            (false, max + 5, true),
            (true, 0, false),
            (true, max, false),
            (true, max + 5, false),
        ];
        for (ready, attempts, expected) in cases {
            let mut row = video_row(ready);
            row.thumbnail_attempts = attempts;
            let dto = VideoDto::from_row(row);
            assert_eq!(
                dto.thumbnail_failed, expected,
                "ready={ready} attempts={attempts}"
            );
            // The flag comes from the DB column, never from a filesystem
            // check (`video_row`'s file does not exist on disk).
            assert_eq!(dto.thumbnail_ready, ready, "attempts={attempts}");
        }
    }

    #[test]
    fn skipped_list_flags_only_files_verifiably_gone() {
        use crate::db::test_support::init_temp_db;
        use gb_core::scan_pipeline::NewSkippedFile;
        use gb_core::testing::fake_folder_probe::FakeFolderProbe;
        use gb_core::testing::fake_path_existence::FakePathExistence;

        let (_d, db) = init_temp_db();
        {
            let conn = db.writer.lock().unwrap();
            queries::set_watch_folders(&conn, &["C:/a".to_string(), "W:/n".to_string()]).unwrap();
            for path in [
                "C:/a/gone\u{2460}.mp4",
                "C:/a/here\u{2460}.mp4",
                "W:/n/x\u{2460}.mp4",
            ] {
                queries::upsert_skipped_file(
                    &conn,
                    &NewSkippedFile {
                        file_path: path.to_string(),
                        file_name: path[4..].to_string(),
                        reason: "machine_dependent_char",
                        detected_char: '\u{2460}',
                    },
                )
                .unwrap();
            }
        }
        // W:/n is down; its parent "exists" in the fake, so only the
        // reachability check can keep its row from reading as missing.
        let registry = registry_over(Arc::new(FakeFolderProbe::with_unreachable(&["W:/n"])));
        let fs = FakePathExistence::with_paths(&["C:/a", "C:/a/here\u{2460}.mp4", "W:/n"]);

        let list = list_skipped_files_impl(&db, &registry, &fs).unwrap();

        let missing = |suffix: &str| {
            list.iter()
                .find(|d| d.file_path.ends_with(suffix))
                .unwrap()
                .file_missing
        };
        assert!(missing("gone\u{2460}.mp4"));
        assert!(!missing("here\u{2460}.mp4"));
        assert!(!missing("x\u{2460}.mp4"));
    }
}
