//! NAS/network-drive startup diff-scan + periodic polling. Reuses the pure
//! decision logic from `gb_core::reconcile` (including the
//! broken-enumeration guard) and the shared per-file entry point
//! `crate::scan::process_detected_file` -- this
//! module is glue: walk the folder, classify cheaply, delegate real work,
//! apply the missing-video verdict.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use gb_core::reconcile::{
    self, DiscoveredFile, EnumerationResult, FileClassification, KnownFileMeta, KnownOnlineVideo,
};
use gb_core::scan_pipeline;
use walkdir::WalkDir;

use crate::adapters::long_path;
use crate::db::{queries, Db};
use crate::events::CatalogNotifier;
use crate::scan::{self, mtime_from_metadata};

/// How often `start_nas_polling`'s loop checks its stop flag while waiting
/// for the next full `interval` to elapse -- bounds `NasPollHandle::stop`'s
/// worst-case latency to about this long, independent of how large
/// `interval` (e.g. the default 600s) is.
const STOP_CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// One diff-scan pass over `folder`: reachability check (doubles as the NAS
/// connectivity check -- no separate ping), a walk classifying each video
/// file via the same cheap mtime/file_size filter NAS polling exists for,
/// and a `decide_missing_video_ids` pass to flip genuinely-gone videos
/// offline without ever touching ones under a directory that merely failed
/// to enumerate this cycle, or ones caught by the broken-enumeration guard
/// (see `gb_core::reconcile::decide_missing_video_ids`'s doc comment).
///
/// **Path-representation invariant:**
/// `decide_missing_video_ids` excludes a known video from the "missing" set
/// only if its `file_path` textually starts with (case-insensitively) one
/// of `inaccessible_dirs`'s entries. `\\?\` prefixing lets the walk
/// itself reach past `MAX_PATH` on a deep NAS share, but every path is
/// stripped back to plain form (`long_path::strip_long_path_prefix`)
/// immediately after coming out of `WalkDir` -- both `discovered_paths` and
/// `inaccessible_dirs` are still built from plain, unprefixed strings here,
/// matching how `file_path` is written to the DB. The `\\?\` prefix exists
/// only transiently, applied right at `WalkDir::new`'s root and never
/// carried into anything that gets compared or persisted.
///
/// **Cooperative cancellation:** `stop_flag` is checked at three
/// points -- before starting the walk (the point after the one operation
/// that measurably blocks for a long time against an unreachable NAS, ~21s
/// observed against a network path with nothing listening), during the walk
/// (cheap per-entry check, defends against a very large/slow folder), and
/// before the final DB write. A walk aborted mid-way skips the
/// missing-video determination entirely for this cycle rather than running
/// it against an incomplete `discovered_paths` -- doing otherwise would
/// reintroduce exactly the mass-offline risk that `decide_missing_video_ids`'s
/// broken-enumeration guard exists to prevent. There is no way to
/// interrupt the initial `std::fs::read_dir` call itself once it's blocked
/// inside the OS -- that is a structural limit of cooperative
/// (flag-checking) cancellation, accepted in favor of not
/// adding a second timeout-supervision thread.
pub fn run_nas_diff_scan(
    db: &Db,
    thumbnails_root: &Path,
    folder: &str,
    stop_flag: &AtomicBool,
    notifier: &dyn CatalogNotifier,
) -> anyhow::Result<()> {
    if std::fs::read_dir(long_path::to_long_path(Path::new(folder))).is_err() {
        log::warn!("NAS root unreachable this cycle, skipping: {folder}");
        return Ok(());
    }

    if stop_flag.load(Ordering::SeqCst) {
        log::info!("NAS diff scan for {folder} aborted before walking (stop requested)");
        return Ok(());
    }

    let known_rows = queries::list_online_videos_under(&db.read_pool, folder)?;
    let known_by_path: HashMap<&str, &queries::VideoRow> = known_rows
        .iter()
        .map(|r| (r.file_path.as_str(), r))
        .collect();

    let mut discovered_paths = Vec::new();
    let mut inaccessible_dirs = Vec::new();
    let mut aborted = false;

    for entry in WalkDir::new(long_path::to_long_path(Path::new(folder))) {
        if stop_flag.load(Ordering::SeqCst) {
            aborted = true;
            break;
        }
        match entry {
            Ok(e) => {
                if !e.file_type().is_file() {
                    continue;
                }
                let file_name = e.file_name().to_string_lossy().to_string();
                if !scan_pipeline::is_video_file(&file_name) {
                    continue;
                }
                // Stripped back to plain form immediately (see this fn's
                // doc comment) -- path_str is used both as the
                // known_by_path lookup key (must match the DB's plain
                // file_path exactly) and as the discovered_paths entry fed
                // into decide_missing_video_ids's prefix match.
                let entry_path = long_path::strip_long_path_prefix(e.path());
                let path_str = entry_path.to_string_lossy().to_string();

                let metadata = match e.metadata() {
                    Ok(m) => m,
                    Err(err) => {
                        log::warn!("skipping {path_str} during NAS diff scan: {err}");
                        continue;
                    }
                };
                let file_size = metadata.len();
                let mtime = match mtime_from_metadata(&metadata) {
                    Ok(t) => t,
                    Err(err) => {
                        log::warn!("skipping {path_str} during NAS diff scan: {err}");
                        continue;
                    }
                };
                // Discovered *after* a successful stat, so a file that
                // vanished between the walk and here isn't counted as
                // "still present" -- it'll simply be absent from
                // discovered_paths and picked up by decide_missing_video_ids
                // like any other genuinely-gone file.
                discovered_paths.push(path_str.clone());

                let known_meta = known_by_path.get(path_str.as_str()).map(|r| KnownFileMeta {
                    file_size: r.file_size as u64,
                    mtime: r.mtime,
                });
                let classification = reconcile::classify_discovered_file(
                    &DiscoveredFile { file_size, mtime },
                    known_meta.as_ref(),
                );
                // Unchanged: the whole point of the cheap filter is to stop
                // here without ever calling process_detected_file (which
                // would do its own DB lookup per file) -- this is the
                // two-stage diff's whole point.
                if !matches!(classification, FileClassification::Unchanged) {
                    match scan::process_detected_file(
                        db,
                        thumbnails_root,
                        &entry_path,
                        file_size,
                        mtime,
                    ) {
                        Ok(
                            scan::ProcessOutcome::Registered
                            | scan::ProcessOutcome::Reconciled
                            | scan::ProcessOutcome::PathFollowed { .. },
                        ) => notifier.notify_changed(),
                        // Same reasoning as watch::handle_watch_event: the
                        // remaining outcomes (Unchanged shouldn't occur here
                        // given the classification check above,
                        // SkippedInvalidName/SkippedUnreadable/
                        // BlockedByCollision) never write, so nothing to
                        // notify the frontend about.
                        Ok(_) => {}
                        Err(err) => {
                            log::error!("failed to process NAS file {path_str}: {err}");
                        }
                    }
                }
            }
            Err(err) => {
                if let Some(path) = err.path() {
                    let path = long_path::strip_long_path_prefix(path);
                    inaccessible_dirs.push(format!("{}\\", path.to_string_lossy()));
                }
                log::warn!("failed to access a path during NAS diff scan: {err}");
            }
        }
    }

    if aborted {
        // discovered_paths is necessarily incomplete here -- running
        // missing-video determination against it would risk exactly the
        // mass-offline failure the broken-enumeration guard exists to
        // prevent, so this cycle's verdict is skipped entirely rather than
        // computed from partial data.
        log::info!(
            "NAS diff scan for {folder} aborted mid-walk (stop requested); skipping this cycle's missing-video check"
        );
        return Ok(());
    }

    if stop_flag.load(Ordering::SeqCst) {
        log::info!("NAS diff scan for {folder} aborted before writing (stop requested)");
        return Ok(());
    }

    let known_online: Vec<KnownOnlineVideo> = known_rows
        .iter()
        .map(|r| KnownOnlineVideo {
            video_id: r.id.clone(),
            file_path: r.file_path.clone(),
        })
        .collect();
    let diff = EnumerationResult {
        root_reachable: true,
        inaccessible_dirs,
        discovered_paths,
    };
    let decision = reconcile::decide_missing_video_ids(&known_online, &diff);
    if let Some(guard) = &decision.suppressed {
        log::warn!(
            "NAS diff scan for {folder}: holding {} video(s) online this cycle (known={}, \
             discovered={}, reason={:?}) -- enumeration looks broken, not treating this as \
             genuine deletion",
            guard.candidate_count,
            guard.known_online_count,
            guard.discovered_count,
            guard.reason
        );
    }
    if !decision.missing_ids.is_empty() {
        {
            let conn = db.writer.lock().unwrap();
            for id in &decision.missing_ids {
                queries::update_video_status(&conn, id, "offline")?;
            }
        }
        notifier.notify_changed();
        log::info!(
            "NAS diff scan: {} video(s) went offline under {folder}",
            decision.missing_ids.len()
        );
    }
    Ok(())
}

/// Handle returned by `start_nas_polling`. `stop()` sets a shared flag and
/// returns immediately -- it never waits for the polling thread to actually
/// exit. That thread notices the flag at the next checkpoint
/// inside `run_nas_diff_scan`, or (worst case, if it's blocked inside the
/// one call `run_nas_diff_scan` can't interrupt) once that call returns on
/// its own; either way it is not this call's problem to wait for. This is
/// what keeps `RealtimeWatchManager::reconfigure` (called synchronously
/// from the `pick_watch_folders` command handler) from blocking the UI for
/// as long as an unreachable NAS's OS-level timeout.
pub struct NasPollHandle {
    stop_flag: Arc<AtomicBool>,
}

impl NasPollHandle {
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }
}

/// Spawns a background thread that runs `run_nas_diff_scan` once
/// immediately (this doubles as the startup diff-scan -- there's no
/// separate startup-only code path) and then every `interval`
/// thereafter, until `NasPollHandle::stop` is called. The spawned thread's
/// `JoinHandle` is intentionally not retained: nothing here ever joins it
/// (see `NasPollHandle::stop`'s doc comment), so holding onto it would serve
/// no purpose. Dropping a `JoinHandle` merely detaches the thread; it keeps
/// running independently and its resources are reclaimed by the OS when it
/// finishes on its own.
pub fn start_nas_polling<N: CatalogNotifier + 'static>(
    db: Db,
    thumbnails_root: PathBuf,
    folder: String,
    interval: Duration,
    notifier: Arc<N>,
) -> NasPollHandle {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop_flag);

    thread::spawn(move || {
        if let Err(err) =
            run_nas_diff_scan(&db, &thumbnails_root, &folder, &flag, notifier.as_ref())
        {
            log::error!("NAS diff scan failed for {folder}: {err}");
        }

        let mut elapsed = Duration::ZERO;
        while !flag.load(Ordering::SeqCst) {
            thread::sleep(STOP_CHECK_INTERVAL);
            elapsed += STOP_CHECK_INTERVAL;
            if elapsed >= interval {
                elapsed = Duration::ZERO;
                if flag.load(Ordering::SeqCst) {
                    break;
                }
                if let Err(err) =
                    run_nas_diff_scan(&db, &thumbnails_root, &folder, &flag, notifier.as_ref())
                {
                    log::error!("NAS diff scan failed for {folder}: {err}");
                }
            }
        }
    });

    NasPollHandle { stop_flag }
}
