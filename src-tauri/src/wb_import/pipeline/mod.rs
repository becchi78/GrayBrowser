//! `.wb` import pipeline: reads every `movie` row via a `WbSourceAdapter`,
//! converts+writes each one through
//! `gb_core::wb_import`/`db::queries::import_wb_video`, then links legacy
//! thumbnails via `gb_core::wb_import::match_thumbnail_files`. Runs on a
//! background thread (`run_wb_import`) so the command layer never blocks on
//! it -- a ~3072-row real library must not freeze the UI.

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use gb_core::ports::dir_lister::DirLister;
use gb_core::ports::ffmpeg::FfmpegAdapter;
use gb_core::ports::folder_probe::FolderProbe;
use gb_core::ports::volume_label::VolumeLabelResolver;
use gb_core::ports::wb_import_clock::{ClockReading, WbImportClock};
use gb_core::ports::wb_import_log::WbImportLog;
use gb_core::ports::wb_source::{WbMovieRow, WbSourceAdapter};
use gb_core::wb_import::{count_clamped_scores, count_source_tags, WbImportStage};
use gb_core::wb_import_log as wb_log;
use gb_core::wb_thumbnail_split::ThumbLayout;

// Only the tests use these names here: they reach this module's imports
// through `use super::*`, while the code that needs them lives in `rows`.
#[cfg(test)]
use gb_core::wb_skipped_payload::WbSkippedPayload;
#[cfg(test)]
use std::collections::HashSet;

use crate::db::{queries, Db};
use crate::events::{CatalogNotifier, WatchReconfigurer, WbImportNotifier, WbImportSummary};
use crate::reachability::ReachabilityRegistry;
use crate::thumbnail::reassign::reassign_thumbnails_after_folders_added;
use crate::thumbnail::{self, ThumbnailQueueHandle};

mod file_state;
mod import_log;
mod rows;
mod share;
mod thumbnails;

#[cfg(test)]
mod tests;

pub(crate) use thumbnails::attach_recorded_thumbnail;

use import_log::{format_finish_text, write_import_log, StageStarts};
use rows::{register_rows, RowTally};
use share::{
    format_unreachable_share, resolve_volume_labels, ShareReachability, VolumeLabelReport,
};
use thumbnails::{
    link_empty_hash_thumbnails_by_name, link_thumbnails, read_thumb_layout,
    save_unmatched_thumbnails, LinkOutcome,
};

/// How often `notifier.notify_progress` fires while importing movie rows --
/// often enough for a responsive progress indicator across the real
/// ~3072-row library, without emitting an event (and its IPC round trip) on
/// every single row.
const PROGRESS_NOTIFY_INTERVAL: u32 = 50;

/// The three filesystem paths `run_wb_import` needs, bundled into one struct
/// so the function's own parameter count stays under
/// `clippy::too_many_arguments`'s default threshold once the source adapter
/// and three notifier-ish handles are added alongside `db`.
pub struct WbImportPaths {
    /// The `.wb` file path -- `wb_source` is already opened against it by
    /// the caller; this is kept only for log messages.
    pub wb_path: PathBuf,
    /// Legacy WhiteBrowser thumbnail folder (flat, `[name].#<hash>.jpg`
    /// files).
    pub thumbnail_folder: PathBuf,
    /// This app's `thumbnails/` directory (`[id].webp`).
    pub thumbnails_dir: PathBuf,
}

/// Spawns the `.wb` import on a background thread and returns immediately --
/// mirrors `thumbnail::enqueue_missing_thumbnails`'s "fire-and-forget, never
/// blocks the caller" shape. See `import_all` for the actual synchronous
/// row-import + legacy-thumbnail-linking logic (factored out so it can be
/// unit-tested directly, without threading or timing, the same way
/// `thumbnail::worker`'s functions are tested directly while
/// `enqueue_missing_thumbnails` itself is not).
///
/// After `import_all` finishes, this also kicks off `thumbnail::
/// enqueue_missing_thumbnails` for every online video still missing a
/// thumbnail (this import's newly registered online rows, plus any earlier
/// ones) -- online rows never go through `import_all`'s legacy-JPG linking
/// (see the comment at the `Inserted` match arm of
/// `rows::RowTally::record_written`), so without this second
/// step they would simply never get a thumbnail at all. This call sits here
/// in `run_wb_import`, not inside `import_all`, deliberately:
/// `enqueue_missing_thumbnails` needs owned `Arc<F>`/`Arc<N>` and `'static`,
/// which would conflict with `import_all`'s own `&F`/`&N`/`&C` borrowed-
/// reference signature (kept borrowed there specifically so tests can pass
/// plain fakes by reference without wrapping them in `Arc`).
// One argument over clippy's limit: each is an independent handle the spawned
// thread needs, and grouping them would only rename the problem.
#[allow(clippy::too_many_arguments)]
pub fn run_wb_import<S, F, N, C, W, D, P, V, K, L>(
    db: Db,
    paths: WbImportPaths,
    wb_source: S,
    ffmpeg: Arc<F>,
    notifier: Arc<N>,
    catalog_notifier: Arc<C>,
    thumbnails_queue: ThumbnailQueueHandle,
    reachability: ReachabilityRegistry,
    register_watch_folders: bool,
    watch_reconfigurer: Arc<W>,
    dir_lister: Arc<D>,
    folder_probe: Arc<P>,
    volume_labels: Arc<V>,
    clock: Arc<K>,
    import_log: L,
) where
    S: WbSourceAdapter + 'static,
    F: FfmpegAdapter + 'static,
    N: WbImportNotifier + 'static,
    C: CatalogNotifier + 'static,
    W: WatchReconfigurer + 'static,
    D: DirLister + 'static,
    P: FolderProbe + 'static,
    V: VolumeLabelResolver + 'static,
    K: WbImportClock + 'static,
    L: WbImportLog + 'static,
{
    thread::spawn(move || {
        import_all(
            &db,
            &paths,
            &wb_source,
            ffmpeg.as_ref(),
            notifier.as_ref(),
            catalog_notifier.as_ref(),
            register_watch_folders.then_some(watch_reconfigurer.as_ref()),
            dir_lister.as_ref(),
            folder_probe.as_ref(),
            volume_labels.as_ref(),
            clock.as_ref(),
            &import_log,
        );

        // `db`/`paths.thumbnails_dir`/`ffmpeg`/`catalog_notifier` were only
        // ever borrowed above (import_all takes references), so they're
        // still owned here and can move straight into this call -- no
        // Arc::clone needed since nothing in this closure uses them again
        // afterward.
        thumbnail::enqueue_missing_thumbnails(
            db,
            paths.thumbnails_dir,
            thumbnails_queue,
            ffmpeg,
            catalog_notifier,
            reachability,
        );
    });
}

/// Whether a progress count is worth an event: every
/// `PROGRESS_NOTIFY_INTERVAL` items plus the last one.
fn is_progress_tick(processed: u32, total: u32) -> bool {
    processed.is_multiple_of(PROGRESS_NOTIFY_INTERVAL) || processed == total
}

/// Reports progress after row `idx` (0-based), throttled by
/// [`is_progress_tick`]. `reported` remembers the last count actually sent, so
/// the caller can tell whether the final count still has to be sent: rows that
/// fail early never reach this function, so the last row alone cannot be
/// relied on to deliver `processed == total`.
fn notify_row_progress<N: WbImportNotifier>(
    notifier: &N,
    idx: usize,
    total: u32,
    reported: &mut Option<u32>,
) {
    let processed = (idx + 1) as u32;
    if is_progress_tick(processed, total) {
        notifier.notify_progress(WbImportStage::Registering, processed, Some(total));
        *reported = Some(processed);
    }
}

/// The actual import logic, run synchronously. `run_wb_import` is the only
/// production caller (from its background thread); tests call this directly
/// so counts/idempotency can be asserted without any thread-timing
/// coordination.
///
/// `watch` is `Some` only when the user opted in to registering the imported
/// videos' folders as watch folders; it is then also told to re-apply the
/// watchers once the list actually grew.
// One argument over clippy's limit, same as `run_wb_import`: each is an
// independent handle, and grouping them would only rename the problem.
#[allow(clippy::too_many_arguments)]
fn import_all<S, F, N, C, W, D, P, V, K, L>(
    db: &Db,
    paths: &WbImportPaths,
    wb_source: &S,
    ffmpeg: &F,
    notifier: &N,
    catalog_notifier: &C,
    watch: Option<&W>,
    dir_lister: &D,
    folder_probe: &P,
    volume_labels: &V,
    clock: &K,
    import_log: &L,
) where
    S: WbSourceAdapter,
    F: FfmpegAdapter,
    N: WbImportNotifier,
    C: CatalogNotifier,
    W: WatchReconfigurer,
    D: DirLister,
    P: FolderProbe,
    V: VolumeLabelResolver,
    K: WbImportClock,
    L: WbImportLog,
{
    log::info!(
        "wb import started: wb_path={} import_log={}",
        paths.wb_path.display(),
        import_log.location()
    );
    // One reading serves as the import's start, the header time and the start
    // of the first stage, so the three can never differ.
    let started = clock.now();
    let Some(mut rows) = read_rows(
        paths,
        wb_source,
        notifier,
        watch.is_some(),
        clock,
        import_log,
        started,
    ) else {
        return;
    };

    let thumb_layout = read_thumb_layout(wb_source);

    let total = rows.len() as u32;
    // Reading has no intermediate count (one query fills the whole `Vec`), so
    // the size is only known, and reported, once it has finished.
    notifier.notify_progress(WbImportStage::Reading, total, Some(total));
    import_log.append(&wb_log::format_read_count(total));
    let source = SourceCounts {
        total,
        clamped_scores: count_clamped_scores(&rows) as u32,
        source_tag_count: count_source_tags(&rows) as u32,
    };

    // Before the share probe: a rewritten path names a real drive, which the
    // probe, the `?` restoration and the file checks below must all see.
    let wb_paths: Vec<String> = rows.iter().map(|r| r.movie_path.clone()).collect();
    let label_report = resolve_volume_labels(&mut rows, volume_labels);
    import_log.append(&wb_log::format_volume_labels(&label_report.logs));

    // Decided up front, before any row touches the filesystem, so a stopped
    // share is waited for once (and concurrently with other stopped shares).
    let shares = ShareReachability::probe(folder_probe, rows.iter().map(|r| r.movie_path.as_str()));
    for root in shares.unreachable_roots() {
        log::warn!("wb import: share unreachable, registering its rows offline: {root}");
        import_log.append(&format_unreachable_share(root));
    }

    let registering_started = clock.now();
    notifier.notify_stage(WbImportStage::Registering);
    let tally = register_rows(db, &rows, &wb_paths, &shares, dir_lister, notifier, total);

    // Must precede `link_thumbnails` and the thumbnail queue: both decide
    // where a thumbnail is stored from the watch folders in force at that
    // moment, so adding the folders first puts new thumbnails straight into
    // their final directory.
    let folders = settle_watch_folders(db, paths, watch.is_some(), &tally.registered_movie_paths);
    let linking_started = clock.now();
    notifier.notify_stage(WbImportStage::Linking);
    let thumbs = link_legacy_thumbnails(
        db,
        paths,
        &folders.current,
        &tally,
        ffmpeg,
        thumb_layout.as_ref(),
        notifier,
    );

    let summary = build_summary(
        &source,
        &tally,
        &thumbs,
        folders.added,
        folders.skipped_roots,
        label_report,
    );

    let stage_starts = StageStarts {
        reading: started,
        registering: registering_started,
        linking: linking_started,
    };
    write_import_log(
        import_log,
        &tally.row_logs,
        &thumbs.outcome,
        &thumbs.name_linked,
        &summary,
        &stage_starts,
        &clock.now(),
    );

    announce_completion(
        import_log,
        notifier,
        catalog_notifier,
        watch,
        folders.change.as_ref(),
        &summary,
    );
}

/// The reading stage: the log's header, the read itself and, when the read
/// fails, the whole ending of the import. `None` means there is nothing left
/// to do.
fn read_rows<S, N, K, L>(
    paths: &WbImportPaths,
    wb_source: &S,
    notifier: &N,
    register_watch_folders: bool,
    clock: &K,
    import_log: &L,
    started: ClockReading,
) -> Option<Vec<WbMovieRow>>
where
    S: WbSourceAdapter,
    N: WbImportNotifier,
    K: WbImportClock,
    L: WbImportLog,
{
    import_log.append(&wb_log::format_header(
        &started.local,
        &paths.wb_path.display().to_string(),
        &paths.thumbnail_folder.display().to_string(),
        register_watch_folders,
    ));
    import_log.append(&wb_log::format_stage_heading(
        WbImportStage::Reading,
        &started.local,
    ));

    notifier.notify_stage(WbImportStage::Reading);
    match wb_source.read_movies() {
        Ok(rows) => Some(rows),
        Err(e) => {
            // notify_failed, not notify_complete: nothing was attempted, so
            // there is no summary to report -- an empty-everything
            // WbImportSummary would misleadingly read as "50 rows imported,
            // 0 succeeded" rather than "never started". Without a distinct
            // failure notification here, start_wb_import's Ok(()) return
            // gives the frontend no way to learn the import silently died --
            // it would just look "in progress" forever.
            let reason = format!(
                "failed to read movies from {}: {e}",
                paths.wb_path.display()
            );
            log::error!("wb import: {reason}");
            import_log.append(&wb_log::format_failure(&reason));
            // The time up to the failure is still worth knowing, e.g. a read
            // that hangs on a network drive before it errors.
            import_log.append(&format_finish_text(
                &[(WbImportStage::Reading, started.elapsed_ms)],
                &clock.now(),
            ));
            notifier.notify_failed(&reason);
            None
        }
    }
}

/// The counts taken from the `.wb` as read, before any row is written.
struct SourceCounts {
    total: u32,
    clamped_scores: u32,
    source_tag_count: u32,
}

/// The watch folders as linking will see them, settled in one step because
/// each part depends on the registration before it.
struct WatchFolders {
    /// `Some` only when folders were actually added.
    change: Option<WatchFolderChange>,
    added: Vec<String>,
    /// The stored list once registration is done.
    current: Vec<String>,
    skipped_roots: Vec<String>,
}

/// Registers the imported videos' folders (when opted in), moves existing
/// thumbnails to follow them, reads back the list linking will use, and
/// judges against it the root-level folders left unregistered
/// (`skipped_roots`).
fn settle_watch_folders(
    db: &Db,
    paths: &WbImportPaths,
    register_watch_folders: bool,
    registered_movie_paths: &[String],
) -> WatchFolders {
    let watch_change = if register_watch_folders {
        register_import_folders(db, registered_movie_paths)
    } else {
        None
    };
    if let Some(change) = &watch_change {
        // Videos registered before this import (re-import, earlier scans)
        // and legacy-linked offline videos already have their files under
        // the directory resolved by the old list.
        match db.read_pool.get() {
            Ok(conn) => {
                reassign_thumbnails_after_folders_added(
                    &conn,
                    &paths.thumbnails_dir,
                    &change.before,
                    &change.after,
                );
            }
            Err(e) => log::warn!(
                "wb import: failed to acquire a DB connection to move thumbnails into the new \
                 watch folders: {e}"
            ),
        }
    }
    let watch_folders_added = watch_change
        .as_ref()
        .map(|c| c.added.clone())
        .unwrap_or_default();

    // Fetched once, up front, and reused by `link_thumbnails` for every
    // matched video rather than re-querying it per video.
    let watch_folders = match db.read_pool.get() {
        Ok(conn) => queries::get_watch_folders(&conn).unwrap_or_default(),
        Err(e) => {
            log::warn!(
                "wb import: failed to acquire a DB connection to read watch_folders before \
                 linking legacy thumbnails: {e}"
            );
            Vec::new()
        }
    };
    // Independent of `watch_change`, which is `None` when nothing was added --
    // exactly the case where only root-level videos were imported. Judged
    // against the list after registration so a root the user already watches
    // is not reported.
    let watch_folders_skipped_roots = if register_watch_folders {
        gb_core::watch_folders::skipped_root_folders(registered_movie_paths, &watch_folders)
    } else {
        Vec::new()
    };

    WatchFolders {
        change: watch_change,
        added: watch_folders_added,
        current: watch_folders,
        skipped_roots: watch_folders_skipped_roots,
    }
}

/// What linking the legacy thumbnails came to, by hash and then by name.
struct LinkedThumbnails {
    outcome: LinkOutcome,
    name_linked: Vec<(String, String)>,
    linked: u32,
    linked_by_name: u32,
    failed: u32,
    unmatched: u32,
    unmatched_empty_hash: u32,
    skipped_online: u32,
}

/// The linking stage after its announcement: links the legacy JPGs, flags
/// the linked videos ready, keeps the unmatched list and links what it can
/// by name. Its counts are final, as the by-name links are moved out of the
/// unmatched ones here.
fn link_legacy_thumbnails<F, N>(
    db: &Db,
    paths: &WbImportPaths,
    watch_folders: &[String],
    tally: &RowTally,
    ffmpeg: &F,
    thumb_layout: Option<&ThumbLayout>,
    notifier: &N,
) -> LinkedThumbnails
where
    F: FfmpegAdapter,
    N: WbImportNotifier,
{
    let link_outcome = link_thumbnails(
        &paths.thumbnail_folder,
        &paths.thumbnails_dir,
        watch_folders,
        &tally.registered_movies,
        &tally.online_hashes,
        &tally.invalid_name_hashes,
        &tally.already_registered_offline_hashes,
        ffmpeg,
        thumb_layout,
        |processed, total| {
            notifier.notify_progress(WbImportStage::Linking, processed, Some(total));
        },
    );

    // The list view only fetches thumbnail images for rows whose
    // `thumbnail_ready` is set, so slot files on disk alone leave the row
    // stuck on "generating". One lock for the whole batch keeps the writer
    // free for other callers; a single failure is logged and does not
    // change the linked count, since the files really are in place.
    if !link_outcome.linked_video_ids.is_empty() {
        let mut conn = db.writer.lock().unwrap();
        queries::mark_thumbnails_ready(&mut conn, &link_outcome.linked_video_ids);
    }
    let list_saved =
        save_unmatched_thumbnails(db, &paths.thumbnail_folder, &link_outcome, thumb_layout);
    // Only on a freshly saved list: otherwise the stored rows are from an
    // earlier import and may not match what is in the folder now.
    let name_linked = if list_saved {
        link_empty_hash_thumbnails_by_name(
            db,
            ffmpeg,
            &paths.thumbnails_dir,
            watch_folders,
            &paths.thumbnail_folder,
        )
        .linked
    } else {
        Vec::new()
    };
    // Each by-name link was counted unmatched (and empty-hash) by the linking
    // above, so it moves from there to linked.
    let linked_by_name = name_linked.len() as u32;
    let thumbnails_linked = link_outcome.linked + linked_by_name;
    let thumbnails_failed = link_outcome.failed;
    let thumbnails_unmatched = link_outcome.unmatched.saturating_sub(linked_by_name);
    let thumbnails_unmatched_empty_hash = link_outcome
        .unmatched_empty_hash
        .saturating_sub(linked_by_name);
    let thumbnails_skipped_online = link_outcome.skipped_online;

    LinkedThumbnails {
        outcome: link_outcome,
        name_linked,
        linked: thumbnails_linked,
        linked_by_name,
        failed: thumbnails_failed,
        unmatched: thumbnails_unmatched,
        unmatched_empty_hash: thumbnails_unmatched_empty_hash,
        skipped_online: thumbnails_skipped_online,
    }
}

/// Gathers every stage's counts into the summary that the log, the screen
/// and the completion log line all report.
fn build_summary(
    source: &SourceCounts,
    tally: &RowTally,
    thumbs: &LinkedThumbnails,
    watch_folders_added: Vec<String>,
    watch_folders_skipped_roots: Vec<String>,
    label_report: VolumeLabelReport,
) -> WbImportSummary {
    WbImportSummary {
        total: source.total,
        registered: tally.registered_online + tally.registered_offline,
        registered_online: tally.registered_online,
        registered_offline: tally.registered_offline,
        failed_rows: tally.failed_rows,
        skipped: tally.skipped,
        skipped_invalid_name: tally.skipped_invalid_name,
        clamped_scores: source.clamped_scores,
        tags_assigned: tally.tags_assigned,
        source_tag_count: source.source_tag_count,
        ratings_from_star_tags: tally.ratings_from_star_tags,
        thumbnails_linked: thumbs.linked,
        thumbnails_linked_by_name: thumbs.linked_by_name,
        thumbnails_failed: thumbs.failed,
        thumbnails_unmatched: thumbs.unmatched,
        thumbnails_unmatched_empty_hash: thumbs.unmatched_empty_hash,
        thumbnails_skipped_online: thumbs.skipped_online,
        watch_folders_added,
        watch_folders_skipped_roots,
        volume_label_resolved: label_report.resolved_rows,
        volume_label_unresolved: label_report.unresolved_rows,
        volume_label_unresolved_labels: label_report.unresolved_labels,
    }
}

/// Ends an import whose log is already written: the screen learns the
/// result, the list is re-fetched, and the watchers restart on the grown
/// folder list.
fn announce_completion<N, C, W, L>(
    import_log: &L,
    notifier: &N,
    catalog_notifier: &C,
    watch: Option<&W>,
    watch_change: Option<&WatchFolderChange>,
    summary: &WbImportSummary,
) where
    N: WbImportNotifier,
    C: CatalogNotifier,
    W: WatchReconfigurer,
    L: WbImportLog,
{
    log::info!(
        "wb import complete: import_log={} total={} registered={} registered_online={} \
         registered_offline={} skipped={} failed_rows={} skipped_invalid_name={} \
         clamped_scores={} tags_assigned={} \
         source_tag_count={} ratings_from_star_tags={} thumbnails_linked={} \
         thumbnails_linked_by_name={} thumbnails_failed={} thumbnails_unmatched={} thumbnails_skipped_online={} \
         watch_folders_added={:?}",
        import_log.location(),
        summary.total,
        summary.registered,
        summary.registered_online,
        summary.registered_offline,
        summary.skipped,
        summary.failed_rows,
        summary.skipped_invalid_name,
        summary.clamped_scores,
        summary.tags_assigned,
        summary.source_tag_count,
        summary.ratings_from_star_tags,
        summary.thumbnails_linked,
        summary.thumbnails_linked_by_name,
        summary.thumbnails_failed,
        summary.thumbnails_unmatched,
        summary.thumbnails_skipped_online,
        summary.watch_folders_added
    );

    notifier.notify_complete(summary);
    // Separate from `notifier` by design: this is what triggers the
    // frontend's list re-fetch, same event `start_scan` fires after a
    // manual scan.
    catalog_notifier.notify_changed();

    if let (Some(watch), Some(change)) = (watch, watch_change) {
        watch.reconfigure(&change.after);
    }
}

/// Records an import that failed before `import_all` could start (the `.wb`
/// could not be opened): the same header and failure text a failed read gets,
/// so the log has the same shape whichever step failed.
pub fn record_start_failure<L: WbImportLog, K: WbImportClock>(
    import_log: &L,
    clock: &K,
    paths: &WbImportPaths,
    register_watch_folders: bool,
    reason: &str,
) {
    let started = clock.now();
    import_log.append(&wb_log::format_header(
        &started.local,
        &paths.wb_path.display().to_string(),
        &paths.thumbnail_folder.display().to_string(),
        register_watch_folders,
    ));
    import_log.append(&wb_log::format_stage_heading(
        WbImportStage::Reading,
        &started.local,
    ));
    import_log.append(&wb_log::format_failure(reason));
    import_log.append(&format_finish_text(
        &[(WbImportStage::Reading, started.elapsed_ms)],
        &clock.now(),
    ));
}

/// Outcome of [`register_import_folders`].
struct WatchFolderChange {
    before: Vec<String>,
    after: Vec<String>,
    added: Vec<String>,
}

/// Adds the parent folders of `registered_movie_paths` to the stored
/// watch-folder list. `None` when nothing needed adding or the DB could not be updated --
/// registering folders is a convenience, so a failure is logged and the
/// import carries on without it. Read-merge-write happens under one writer
/// lock so a concurrent change to the list is not overwritten.
fn register_import_folders(
    db: &Db,
    registered_movie_paths: &[String],
) -> Option<WatchFolderChange> {
    if registered_movie_paths.is_empty() {
        return None;
    }
    let conn = db.writer.lock().unwrap();
    let before = match queries::get_watch_folders(&conn) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("wb import: failed to read watch_folders; not registering folders: {e}");
            return None;
        }
    };
    let added = gb_core::watch_folders::suggest_watch_folders(registered_movie_paths, &before);
    if added.is_empty() {
        return None;
    }
    let after = gb_core::watch_folders::merge(before.clone(), added.clone());
    if let Err(e) = queries::set_watch_folders(&conn, &after) {
        log::warn!("wb import: failed to save watch_folders; not registering folders: {e}");
        return None;
    }
    Some(WatchFolderChange {
        before,
        after,
        added,
    })
}
