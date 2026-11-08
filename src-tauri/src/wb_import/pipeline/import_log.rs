//! Writing the import log: the stage timings, and everything that is only
//! known once linking has finished (rows, linking notes, summary).

use std::collections::{HashMap, HashSet};

use gb_core::ports::wb_import_clock::ClockReading;
use gb_core::ports::wb_import_log::WbImportLog;
use gb_core::wb_import::WbImportStage;
use gb_core::wb_import_log::{self as wb_log, LogSummary, RowLog};

use crate::events::WbImportSummary;

use super::thumbnails::LinkOutcome;

/// When each stage began, as read from the clock.
pub(super) struct StageStarts {
    pub(super) reading: ClockReading,
    pub(super) registering: ClockReading,
    pub(super) linking: ClockReading,
}

/// The closing lines (end time and durations) of an import whose stages began
/// at the given monotonic times and which ended at `finished`.
pub(super) fn format_finish_text(
    starts: &[(WbImportStage, u64)],
    finished: &ClockReading,
) -> String {
    let (stages, total) = wb_log::stage_durations(starts, finished.elapsed_ms);
    wb_log::format_finish(&finished.local, &stages, total)
}

/// Writes everything that is only known once linking has finished: the rows
/// (each with its thumbnail result), the linking notes, the folders added to
/// the watch list and the closing summary.
pub(super) fn write_import_log<L: WbImportLog>(
    import_log: &L,
    row_logs: &[RowLog],
    link_outcome: &LinkOutcome,
    name_linked: &[(String, String)],
    summary: &WbImportSummary,
    starts: &StageStarts,
    finished: &ClockReading,
) {
    let linked: HashSet<String> = link_outcome.linked_video_ids.iter().cloned().collect();
    let failed: HashMap<String, String> = link_outcome.failed_video_ids.iter().cloned().collect();

    import_log.append(&wb_log::format_stage_heading(
        WbImportStage::Registering,
        &starts.registering.local,
    ));
    for row in row_logs {
        let link = wb_log::resolve_link_status(row, &linked, &failed);
        import_log.append(&wb_log::format_row(row, &link));
    }

    import_log.append(&wb_log::format_stage_heading(
        WbImportStage::Linking,
        &starts.linking.local,
    ));
    if let Some(reason) = &link_outcome.folder_error {
        import_log.append(&wb_log::format_thumbnail_folder_error(reason));
    }
    // What was attached by name is no longer waiting for a person, so it is
    // listed apart rather than under "could not be linked automatically".
    let name_linked_images: HashSet<&str> = name_linked.iter().map(|(i, _)| i.as_str()).collect();
    let still_unmatched: Vec<String> = link_outcome
        .unmatched_filenames
        .iter()
        .filter(|f| !name_linked_images.contains(f.as_str()))
        .cloned()
        .collect();
    import_log.append(&wb_log::format_name_linked_thumbnails(name_linked));
    import_log.append(&wb_log::format_unmatched_filenames(&still_unmatched));
    import_log.append(&wb_log::format_watch_folders(&summary.watch_folders_added));
    import_log.append(&wb_log::format_skipped_root_folders(
        &summary.watch_folders_skipped_roots,
    ));
    import_log.append(&wb_log::format_summary(&LogSummary {
        total: summary.total,
        registered: summary.registered,
        registered_online: summary.registered_online,
        registered_offline: summary.registered_offline,
        skipped: summary.skipped,
        skipped_invalid_name: summary.skipped_invalid_name,
        failed_rows: summary.failed_rows,
        clamped_scores: summary.clamped_scores,
        tags_assigned: summary.tags_assigned,
        source_tag_count: summary.source_tag_count,
        ratings_from_star_tags: summary.ratings_from_star_tags,
        thumbnails_linked: summary.thumbnails_linked,
        thumbnails_linked_by_name: summary.thumbnails_linked_by_name,
        thumbnails_failed: summary.thumbnails_failed,
        thumbnails_unmatched: summary.thumbnails_unmatched,
        thumbnails_unmatched_empty_hash: summary.thumbnails_unmatched_empty_hash,
        thumbnails_skipped_online: summary.thumbnails_skipped_online,
        volume_label_resolved: summary.volume_label_resolved,
        volume_label_unresolved: summary.volume_label_unresolved,
    }));
    import_log.append(&format_finish_text(
        &[
            (WbImportStage::Reading, starts.reading.elapsed_ms),
            (WbImportStage::Registering, starts.registering.elapsed_ms),
            (WbImportStage::Linking, starts.linking.elapsed_ms),
        ],
        finished,
    ));
}
