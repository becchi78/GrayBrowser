//! `.wb` import pipeline: reads every `movie` row via a `WbSourceAdapter`,
//! converts+writes each one through
//! `gb_core::wb_import`/`db::queries::import_wb_video`, then links legacy
//! thumbnails via `gb_core::wb_import::match_thumbnail_files`. Runs on a
//! background thread (`run_wb_import`) so the command layer never blocks on
//! it -- a ~3072-row real library must not freeze the UI.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use gb_core::path_restore::last_segment;
use gb_core::ports::dir_lister::DirLister;
use gb_core::ports::ffmpeg::FfmpegAdapter;
use gb_core::ports::folder_probe::FolderProbe;
use gb_core::ports::volume_label::VolumeLabelResolver;
use gb_core::ports::wb_import_clock::WbImportClock;
use gb_core::ports::wb_import_log::WbImportLog;
use gb_core::ports::wb_source::WbSourceAdapter;
use gb_core::scan_pipeline::{self, NewVideo, ScannedFile, ValidationOutcome};
use gb_core::wb_import::{
    count_clamped_scores, count_source_tags, wb_row_to_import_candidate, WbImportStage,
};
use gb_core::wb_import_log::{self as wb_log, Availability, PathRestoration, RowLog, RowOutcome};
use gb_core::wb_skipped_payload::WbSkippedPayload;

use crate::db::{queries, Db};
use crate::events::{CatalogNotifier, WatchReconfigurer, WbImportNotifier, WbImportSummary};
use crate::reachability::ReachabilityRegistry;
use crate::thumbnail::reassign::reassign_thumbnails_after_folders_added;
use crate::thumbnail::{self, ThumbnailQueueHandle};

mod file_state;
mod import_log;
mod share;
mod thumbnails;

#[cfg(test)]
mod tests;

pub(crate) use thumbnails::attach_recorded_thumbnail;

use file_state::{resolve_file_state, restore_missing_path, CachingDirLister};
use import_log::{format_finish_text, write_import_log, StageStarts};
use share::{format_unreachable_share, resolve_volume_labels, ShareReachability};
use thumbnails::{
    link_empty_hash_thumbnails_by_name, link_thumbnails, read_thumb_layout,
    save_unmatched_thumbnails,
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
/// (see the comment at its `Inserted` match arm), so without this second
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
    import_log.append(&wb_log::format_header(
        &started.local,
        &paths.wb_path.display().to_string(),
        &paths.thumbnail_folder.display().to_string(),
        watch.is_some(),
    ));
    import_log.append(&wb_log::format_stage_heading(
        WbImportStage::Reading,
        &started.local,
    ));

    notifier.notify_stage(WbImportStage::Reading);
    let mut rows = match wb_source.read_movies() {
        Ok(rows) => rows,
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
            return;
        }
    };

    let thumb_layout = read_thumb_layout(wb_source);

    let total = rows.len() as u32;
    // Reading has no intermediate count (one query fills the whole `Vec`), so
    // the size is only known, and reported, once it has finished.
    notifier.notify_progress(WbImportStage::Reading, total, Some(total));
    import_log.append(&wb_log::format_read_count(total));
    let clamped_scores = count_clamped_scores(&rows) as u32;
    let source_tag_count = count_source_tags(&rows) as u32;

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

    // Every row ends in exactly one of `registered_*`, `skipped`,
    // `skipped_invalid_name` and `failed_rows`, so the summary's counts add up
    // to `total`.
    let mut registered_online = 0u32;
    let mut registered_offline = 0u32;
    let mut failed_rows = 0u32;
    let mut skipped = 0u32;
    let mut skipped_invalid_name = 0u32;
    let mut tags_assigned = 0u32;
    let mut ratings_from_star_tags = 0u32;
    // (video_id, thumbnail_hash, file_path) of inserted rows with a thumbnail
    // hash, the input to match_thumbnail_files. `file_path` lets
    // `link_thumbnails` find each video's thumbnail subdirectory without
    // another DB query. Already-registered rows are excluded: whatever first
    // registered them already handled their thumbnail.
    let mut registered_movies: Vec<(String, String, String)> = Vec::new();
    // Thumbnail hashes of inserted online rows. Their legacy JPGs are not
    // migrated (a thumbnail is generated from the video), but the hashes keep
    // those JPGs from being reported as unmatched.
    let mut online_hashes: Vec<String> = Vec::new();
    // Hashes of rows refused for a machine-dependent file name: the JPG has
    // nothing to attach to, and `link_thumbnails` drops it unreported.
    let mut invalid_name_hashes: HashSet<String> = HashSet::new();
    // Hashes of already-registered offline rows (re-import): the JPG was
    // handled at first registration, so `link_thumbnails` drops it unreported.
    let mut already_registered_offline_hashes: HashSet<String> = HashSet::new();
    // Paths of rows newly registered by this run, online or offline, to become
    // watch folders. Offline rows count so a stopped NAS folder stays
    // reachable from the sidebar; skipped rows don't, so a re-import adds no
    // duplicates.
    let mut registered_movie_paths: Vec<String> = Vec::new();
    // Shared by every row so a parent folder is listed once per import, even
    // when it cannot be listed at all (see `CachingDirLister`).
    let dir_lister = CachingDirLister::new(dir_lister);
    // One entry per row (every exit of the row loop pushes exactly one),
    // written after linking so each can state what became of its thumbnail.
    let mut row_logs: Vec<RowLog> = Vec::with_capacity(rows.len());

    let registering_started = clock.now();
    notifier.notify_stage(WbImportStage::Registering);
    let mut reported_rows: Option<u32> = None;
    for (idx, row) in rows.iter().enumerate() {
        let mut entry = RowLog {
            movie_id: row.movie_id,
            name: row.movie_name.clone(),
            wb_path: wb_paths[idx].clone(),
            restoration: PathRestoration::NotChecked,
            availability: Availability::NotJudged,
            tag_count: count_source_tags(std::slice::from_ref(row)),
            thumbnail_hash: (!row.hash.is_empty()).then(|| row.hash.clone()),
            outcome: RowOutcome::Failed {
                reason: String::new(),
            },
            video_id: None,
        };
        let candidate = match wb_row_to_import_candidate(row) {
            Ok(c) => c,
            Err(e) => {
                // One malformed row (bad datetime) must not abort the
                // whole import -- log and move on to the next row.
                log::warn!(
                    "wb import: skipping movie_id={} ({}): {e}",
                    row.movie_id,
                    row.movie_name
                );
                failed_rows += 1;
                entry.outcome = RowOutcome::Failed {
                    reason: format!("日付が不正: {e}"),
                };
                row_logs.push(entry);
                continue;
            }
        };

        // The real path on disk: `movie_path` as stored, or -- when it holds
        // `?` for characters the old WhiteBrowser could not convert and the
        // stored path therefore does not exist -- the path restored from the
        // parent folder's contents. Everything downstream (DB row, watch
        // folders, thumbnail location) must use this one, never the stored
        // `?` form.
        //
        // The share is judged on the stored path: a restored path differs only
        // in the characters that were `?`, so it lives on the same share.
        let share_reachable = shares.is_reachable_path(&row.movie_path);
        let restored_path =
            restore_missing_path(&candidate.movie_path, share_reachable, &dir_lister);
        let file_path = restored_path
            .clone()
            .unwrap_or_else(|| candidate.movie_path.clone());
        entry.restoration = match &restored_path {
            Some(p) => PathRestoration::Restored(p.clone()),
            None => PathRestoration::Unchanged,
        };

        // Checked on the restored name (the stored `?` form has lost the
        // original character) and before any hashing, like a normal scan:
        // such a file is never registered, hashed or given a thumbnail.
        // Offline rows are judged by name alone, so they are covered too.
        let scanned_file = ScannedFile {
            file_path: file_path.clone(),
            file_name: last_segment(&file_path).to_string(),
            file_size: 0,
            mtime: 0,
        };
        if let ValidationOutcome::Invalid { detected_char } = scan_pipeline::validate(&scanned_file)
        {
            let skipped_file = scan_pipeline::build_skipped_file(&scanned_file, detected_char);
            log::warn!(
                "wb import: skipped machine-dependent filename: {} (char: {detected_char})",
                skipped_file.file_name
            );
            // Kept so the record can later be registered with this row's
            // rating and tags even after the file itself is gone.
            let payload = WbSkippedPayload::from_candidate(&candidate).to_json();
            let recorded = {
                let conn = db.writer.lock().unwrap();
                queries::upsert_skipped_file_with_wb_payload(&conn, &skipped_file, &payload)
            };
            if let Err(e) = recorded {
                log::error!(
                    "wb import: failed to record skipped file {} for movie_id={}: {e}",
                    file_path,
                    row.movie_id
                );
            }
            skipped_invalid_name += 1;
            entry.outcome = RowOutcome::InvalidName { detected_char };
            row_logs.push(entry);
            if let Some(hash) = candidate.thumbnail_hash {
                invalid_name_hashes.insert(hash);
            }
            notify_row_progress(notifier, idx, total, &mut reported_rows);
            continue;
        }

        let file_state = resolve_file_state(&file_path, share_reachable);
        entry.availability = if file_state.status == "offline" {
            Availability::Offline
        } else {
            Availability::Online
        };
        let id = uuid::Uuid::new_v4().to_string();

        let new_video = NewVideo {
            id: id.clone(),
            file_path: file_path.clone(),
            // movie_name is the file's basename, filled for every row --
            // reused directly rather than re-deriving from movie_path
            // (which may use `\`-separated legacy Windows paths not
            // guaranteed to parse the same way on every platform this
            // workspace builds/tests on). A restored row takes the name from
            // the restored path instead, as movie_name carries the same `?`.
            file_name: if restored_path.is_some() {
                last_segment(&file_path).to_string()
            } else {
                candidate.movie_name.clone()
            },
            file_size: file_state.file_size,
            quick_hash: file_state.quick_hash,
            status: file_state.status,
            mtime: file_state.mtime,
        };

        let outcome = {
            // Held only for this one row's write, then released -- mirrors
            // thumbnail::worker's short-lock convention so a long `.wb`
            // import never starves other writers (realtime watch, manual
            // scan) for its whole duration.
            let mut conn = db.writer.lock().unwrap();
            match queries::import_wb_video(
                &mut conn,
                &new_video,
                candidate.rating,
                &candidate.kana,
                &candidate.roma,
                &candidate.tags,
            ) {
                Ok(outcome) => outcome,
                Err(e) => {
                    log::error!(
                        "wb import: DB write failed for movie_id={} ({}): {e}",
                        row.movie_id,
                        row.movie_name
                    );
                    failed_rows += 1;
                    entry.outcome = RowOutcome::Failed {
                        reason: format!("データベースへの書き込みに失敗: {e}"),
                    };
                    row_logs.push(entry);
                    continue;
                }
            }
        };

        match outcome {
            queries::WbImportOutcome::Inserted {
                tags_assigned: row_tags_assigned,
            } => {
                if file_state.status == "offline" {
                    registered_offline += 1;
                } else {
                    registered_online += 1;
                }
                tags_assigned += row_tags_assigned as u32;
                if candidate.rating_from_star_tags {
                    ratings_from_star_tags += 1;
                }
                registered_movie_paths.push(file_path.clone());
                entry.outcome = RowOutcome::Registered;
                entry.video_id = Some(id.clone());
                // Legacy-JPG thumbnail migration (link_thumbnails, below) is
                // only for offline rows: their source video isn't available
                // to generate a fresh thumbnail from, so the old JPG is the
                // best available image. An online row's video file *is*
                // available, so `run_wb_import` hands it to the existing
                // thumbnail pipeline instead (10%-in frame extraction, the
                // same as a normal scan) once this loop finishes --
                // preferred over the old low-res JPG whenever the real
                // video can produce a fresh one.
                if let Some(hash) = candidate.thumbnail_hash {
                    if file_state.status == "offline" {
                        registered_movies.push((id, hash, file_path.clone()));
                    } else {
                        online_hashes.push(hash);
                    }
                }
            }
            queries::WbImportOutcome::Skipped => {
                skipped += 1;
                entry.outcome = RowOutcome::AlreadyRegistered;
                // An already-registered online row still owns its legacy
                // JPG. Without collecting the hash here, a re-import would
                // report that JPG as unmatched (needs manual attention).
                // An offline row's JPG was settled by whichever import first
                // registered it, so it is not linked again (a row that has no
                // thumbnail yet is not retried either), but its hash is still
                // collected so the JPG is not reported as unmatched.
                // Duplicate hashes are harmless because matching only tests
                // membership.
                if let Some(hash) = candidate.thumbnail_hash {
                    if file_state.status != "offline" {
                        online_hashes.push(hash);
                    } else {
                        already_registered_offline_hashes.insert(hash);
                    }
                }
            }
        }

        row_logs.push(entry);
        notify_row_progress(notifier, idx, total, &mut reported_rows);
    }
    // Also covers an empty `.wb` (no row ever reports) so the phase still ends
    // with a count.
    if reported_rows != Some(total) {
        notifier.notify_progress(WbImportStage::Registering, total, Some(total));
    }

    // Must precede `link_thumbnails` and the thumbnail queue: both decide
    // where a thumbnail is stored from the watch folders in force at that
    // moment, so adding the folders first puts new thumbnails straight into
    // their final directory.
    let watch_change = if watch.is_some() {
        register_import_folders(db, &registered_movie_paths)
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
    let watch_folders_skipped_roots = if watch.is_some() {
        gb_core::watch_folders::skipped_root_folders(&registered_movie_paths, &watch_folders)
    } else {
        Vec::new()
    };
    let linking_started = clock.now();
    notifier.notify_stage(WbImportStage::Linking);
    let link_outcome = link_thumbnails(
        &paths.thumbnail_folder,
        &paths.thumbnails_dir,
        &watch_folders,
        &registered_movies,
        &online_hashes,
        &invalid_name_hashes,
        &already_registered_offline_hashes,
        ffmpeg,
        thumb_layout.as_ref(),
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
    let list_saved = save_unmatched_thumbnails(
        db,
        &paths.thumbnail_folder,
        &link_outcome,
        thumb_layout.as_ref(),
    );
    // Only on a freshly saved list: otherwise the stored rows are from an
    // earlier import and may not match what is in the folder now.
    let name_linked = if list_saved {
        link_empty_hash_thumbnails_by_name(
            db,
            ffmpeg,
            &paths.thumbnails_dir,
            &watch_folders,
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

    let summary = WbImportSummary {
        total,
        registered: registered_online + registered_offline,
        registered_online,
        registered_offline,
        failed_rows,
        skipped,
        skipped_invalid_name,
        clamped_scores,
        tags_assigned,
        source_tag_count,
        ratings_from_star_tags,
        thumbnails_linked,
        thumbnails_linked_by_name: linked_by_name,
        thumbnails_failed,
        thumbnails_unmatched,
        thumbnails_unmatched_empty_hash,
        thumbnails_skipped_online,
        watch_folders_added,
        watch_folders_skipped_roots,
        volume_label_resolved: label_report.resolved_rows,
        volume_label_unresolved: label_report.unresolved_rows,
        volume_label_unresolved_labels: label_report.unresolved_labels,
    };

    let stage_starts = StageStarts {
        reading: started,
        registering: registering_started,
        linking: linking_started,
    };
    write_import_log(
        import_log,
        &row_logs,
        &link_outcome,
        &name_linked,
        &summary,
        &stage_starts,
        &clock.now(),
    );

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

    notifier.notify_complete(&summary);
    // Separate from `notifier` by design: this is what triggers the
    // frontend's list re-fetch, same event `start_scan` fires after a
    // manual scan.
    catalog_notifier.notify_changed();

    if let (Some(watch), Some(change)) = (watch, &watch_change) {
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
