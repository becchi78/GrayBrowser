//! Folder walking + real file I/O + DB glue around `gb_core::scan_pipeline`'s
//! pure validate/build decisions and `gb_core::reconcile`'s pure
//! classify/path-follow decisions.
//!
//! `process_detected_file` is the shared per-file entry point: reused as-is
//! by this module's `scan_folders` (manual full walk), the local realtime
//! watcher, and the NAS diff-scan/poll. Each caller passes a
//! `FolderReachability` so a moved file can be told apart from a copy.

use std::path::Path;
use std::time::Duration;

use gb_core::ports::path_existence::PathExistence;
use gb_core::reconcile;
use gb_core::scan_pipeline;
use walkdir::WalkDir;

use crate::adapters::long_path;
use crate::adapters::path_existence::RealPathExistence;
use crate::db::{queries, Db};
use crate::reachability::ReachabilityRegistry;

mod process;

pub(crate) use process::hash_file;
pub use process::{mtime_from_metadata, process_detected_file, ProcessOutcome};

/// How recent a probe of a watch folder may be for `FolderReachability` to
/// reuse it. A scan that registers many copies of one file would otherwise
/// probe the same folder once per file; this keeps that to one probe per few
/// seconds while still noticing a folder that went away since the last scan.
const MOVE_CHECK_MIN_AGE: Duration = Duration::from_secs(3);

/// Answers "is this watch folder gone right now?" for the move detection in
/// `register_new_path`. Kept as a trait so the scan paths can be exercised
/// with a fake probe behind a real `ReachabilityRegistry`.
pub trait FolderReachability: Send + Sync {
    /// True only when the folder is confirmed unreachable. May block for as
    /// long as a probe of a dead share takes, so callers must not hold the DB
    /// writer lock around it.
    fn is_confirmed_unreachable(&self, folder: &str) -> bool;
}

impl FolderReachability for ReachabilityRegistry {
    fn is_confirmed_unreachable(&self, folder: &str) -> bool {
        // Not `refresh` (forced): that would probe on every call. A recent
        // answer is trusted; otherwise this probes. `is_reachable` then reads
        // the settled answer, waiting for a probe already in flight rather
        // than acting on its provisional "unreachable" display flag.
        self.refresh_if_stale(folder, MOVE_CHECK_MIN_AGE);
        !self.is_reachable(folder)
    }
}

/// Treats every folder as reachable: a duplicate is always registered as a new
/// row and no folder is probed. For callers and tests that have no registry
/// and are not exercising moves out of vanished folders.
#[cfg(any(test, feature = "testing"))]
pub struct AssumeReachable;

#[cfg(any(test, feature = "testing"))]
impl FolderReachability for AssumeReachable {
    fn is_confirmed_unreachable(&self, _folder: &str) -> bool {
        false
    }
}

#[derive(Default, serde::Serialize)]
pub struct ScanSummary {
    pub scanned: u32,
    pub registered: u32,
    /// A known path whose content changed (quick_hash/mtime/file_size
    /// updated) and/or which reconnected from `offline` to `online` at its
    /// existing path. Always represents an actual DB write.
    pub reconciled: u32,
    /// A known path whose mtime+file_size still matched -- no DB write at
    /// all (quick_hash was not recomputed; see `reconcile_known_path`'s doc
    /// comment for the accepted staleness trade-off this implies).
    pub unchanged: u32,
    pub skipped: u32,
    /// A known online video confirmed missing this scan (via
    /// `reconcile_missing_videos`) and flipped to offline. This includes
    /// videos the broken-enumeration guard first withheld but that were then
    /// checked one by one and reported not found by the OS
    /// (`confirm_suppressed_candidates`). Videos still held back after that
    /// are *not* counted here (they're WARN-logged instead, matching NAS
    /// polling's convention) -- this field only reflects confirmed
    /// transitions.
    pub went_offline: u32,
    /// A path-follow match (`ProcessOutcome::PathFollowed`): an `offline` row
    /// matched by quick_hash+file_size at a new path and reactivated there
    /// (`file_path` rewritten, `id`/tags/thumbnail preserved). Kept separate
    /// from `reconciled` (same-path reconnection/content-change) since the
    /// two have different causes and are useful to tell apart when
    /// investigating scan results.
    pub reactivated: u32,
    /// A path-follow candidate existed but its target path already belonged
    /// to a different `online` row (`ProcessOutcome::BlockedByCollision`).
    /// No DB write happens for this file at all -- `file_path`'s UNIQUE
    /// constraint makes a fresh insert at an already-claimed path
    /// structurally impossible, so it is *not* also counted in
    /// `registered`. The file is left unregistered this pass and picked up
    /// again on the next scan/poll. This field exists purely so a
    /// collision's occurrence is visible from the summary itself, not only
    /// from the WARN log.
    pub collisions: u32,
    /// Watch folders a manual scan did not walk because they were known to be
    /// unreachable. `scan_folders` itself never fills this (it does not know
    /// about reachability); the caller that filtered them out does, so the
    /// screen can say why fewer folders were scanned than are registered.
    pub unreachable_folders: Vec<String>,
}

/// `scan_folders_with_existence` asking the real filesystem about withheld
/// missing-video candidates -- what every production caller wants.
pub fn scan_folders(
    folders: &[String],
    db: &Db,
    thumbnails_root: &Path,
    reachability: &dyn FolderReachability,
) -> anyhow::Result<ScanSummary> {
    scan_folders_with_existence(
        folders,
        db,
        thumbnails_root,
        reachability,
        &RealPathExistence,
    )
}

/// The manual full walk. `existence` answers the per-path follow-up for
/// missing-video candidates the broken-enumeration guard withheld (see
/// `reconcile_missing_videos`); it is a parameter so a test can make the OS
/// claim a file still exists, which a real tempdir cannot fake.
pub fn scan_folders_with_existence(
    folders: &[String],
    db: &Db,
    thumbnails_root: &Path,
    reachability: &dyn FolderReachability,
    existence: &dyn PathExistence,
) -> anyhow::Result<ScanSummary> {
    let mut summary = ScanSummary::default();
    log::info!("scan started for {} folder(s)", folders.len());

    for folder in folders {
        // Collected across this *one* folder's walk only, and consumed by
        // reconcile_missing_videos only after the inner loop below fully
        // completes -- calling it any earlier (e.g. per-entry, or after only
        // some folders in a multi-folder scan) would run missing-video
        // determination against a partial listing and risk exactly the
        // mass-offline failure the broken-enumeration guard exists to
        // prevent. Never merged across folders either: folder A's known
        // rows must only ever be judged against folder A's own discovered
        // set (list_online_videos_under(folder) inside
        // reconcile_missing_videos already scopes the "known" side the same
        // way, per-folder).
        let mut discovered_paths = Vec::new();
        let mut inaccessible_dirs = Vec::new();

        // The root is prefixed so the walk itself succeeds past MAX_PATH on
        // a deep watch folder; every path handed back by a `WalkDir`
        // entry then inherits that prefix (walkdir joins child names onto
        // whatever root it was given), so it's stripped back to plain form
        // immediately below -- discovered_paths/inaccessible_dirs and the
        // DB's file_path column must always agree on the unprefixed
        // representation (see gb_core::reconcile's is_under_folder prefix match).
        for entry in WalkDir::new(long_path::to_long_path(Path::new(folder))) {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    if let Some(path) = e.path() {
                        let path = long_path::strip_long_path_prefix(path);
                        inaccessible_dirs.push(format!("{}\\", path.to_string_lossy()));
                    }
                    log::warn!("failed to access a path during scan: {e}");
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }

            let file_name = entry.file_name().to_string_lossy().to_string();
            if !scan_pipeline::is_video_file(&file_name) {
                continue;
            }

            let entry_path = long_path::strip_long_path_prefix(entry.path());
            let file_path = entry_path.to_string_lossy().to_string();
            // A file that vanishes or becomes unreadable between the walk
            // and here is treated the same as a hash failure below (WARN +
            // skip, not an aborted scan). It is also simply absent from
            // discovered_paths, same as any other genuinely-missing file --
            // reconcile_missing_videos (after this folder's walk completes)
            // is what decides whether that means "offline" or "held" for
            // it, not this per-entry loop.
            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    log::warn!("skipping {file_path}: failed to read metadata: {e}");
                    continue;
                }
            };
            let file_size = metadata.len();
            let mtime = match mtime_from_metadata(&metadata) {
                Ok(secs) => secs,
                Err(e) => {
                    log::warn!("skipping {file_path}: {e}");
                    continue;
                }
            };
            discovered_paths.push(file_path.clone());
            summary.scanned += 1;

            match process_detected_file(
                db,
                thumbnails_root,
                &entry_path,
                file_size,
                mtime,
                reachability,
            )? {
                ProcessOutcome::Registered => summary.registered += 1,
                ProcessOutcome::Reconciled => summary.reconciled += 1,
                ProcessOutcome::Unchanged => summary.unchanged += 1,
                ProcessOutcome::SkippedInvalidName { .. } => summary.skipped += 1,
                ProcessOutcome::SkippedUnreadable => {}
                ProcessOutcome::PathFollowed { .. } => summary.reactivated += 1,
                // The WARN log is already emitted on the
                // process_detected_file (register_new_path) side. Because of
                // file_path's UNIQUE constraint, no DB write happens at all
                // for this branch (the colliding path is already occupied,
                // so a new registration is structurally impossible) -- not
                // counted in `registered`, only in `collisions` for
                // visibility.
                ProcessOutcome::BlockedByCollision { .. } => summary.collisions += 1,
            }
        }

        summary.went_offline +=
            reconcile_missing_videos(db, folder, discovered_paths, inaccessible_dirs, existence)?;
    }

    log::info!(
        "scan complete: scanned={} registered={} reconciled={} unchanged={} skipped={} went_offline={} \
         reactivated={} collisions={}",
        summary.scanned,
        summary.registered,
        summary.reconciled,
        summary.unchanged,
        summary.skipped,
        summary.went_offline,
        summary.reactivated,
        summary.collisions
    );
    Ok(summary)
}

/// Logs what the missing-video decision withheld or double-checked, so the
/// manual scan and the NAS poll report it in the same shape. `scan_label`
/// names the caller; `hold_qualifier` is spliced after "online" in the
/// "holding" line (the poll says " this cycle").
pub(crate) fn log_missing_video_decision(
    scan_label: &str,
    folder: &str,
    hold_qualifier: &str,
    decision: &reconcile::MissingVideoDecision,
) {
    if let Some(check) = &decision.individually_checked {
        if check.confirmed_missing_count > 0 {
            log::info!(
                "{scan_label} for {folder}: {} of {} withheld video(s) confirmed not found \
                 one by one (guard reason={:?}), marking them offline",
                check.confirmed_missing_count,
                check.checked_count,
                check.reason
            );
        }
    }
    if let Some(guard) = &decision.suppressed {
        log::warn!(
            "{scan_label} for {folder}: holding {} video(s) online{hold_qualifier} (known={}, \
             discovered={}, reason={:?}) -- enumeration looks broken, not treating this as \
             genuine deletion",
            guard.candidate_count,
            guard.known_online_count,
            guard.discovered_count,
            guard.reason
        );
    }
}

/// Missing-video determination for one folder, run once that folder's
/// `WalkDir` walk has fully completed. This is the manual-scan escape hatch
/// for videos the NAS-polling ratio guard can't self-resolve on its own --
/// a user-triggered full rescan reuses the exact same guarded logic, just
/// against a local walk instead of a network one. Reuses
/// `gb_core::reconcile::decide_missing_video_ids` (broken-enumeration guard
/// included, unchanged here) -- a local `WalkDir`
/// walk carries the same "enumeration might have partially failed" risk a
/// network listing does (locked directories, permission errors, AV scanner
/// interference), so it gets the same protection.
///
/// When the guard withholds only a few candidates,
/// `confirm_suppressed_candidates` then asks `existence` about each one and
/// lets through only those the OS reports as not found -- otherwise a folder
/// holding a single video could never see it go offline. That check cannot
/// tell "this file was moved" from "the whole folder is gone": a deleted
/// parent folder, an unassigned drive letter, an unknown host and an unknown
/// share all come back from the OS as NotFound. So it must never see the
/// candidates of a folder that is gone, and it doesn't: a missing or
/// unreachable root makes `WalkDir` fail on the root itself, with the root's
/// path attached, so `"<folder>\"` lands in `inaccessible_dirs` and
/// `is_under_folder` removes every known video of the folder from the
/// candidates before the guard or the check runs (quietly, no WARN). Ahead
/// of that, `scan_cmds::partition_by_reachability` already leaves folders
/// known to be unreachable out of the manual scan. Whatever remains withheld
/// after the check is WARN-logged and stays online.
fn reconcile_missing_videos(
    db: &Db,
    folder: &str,
    discovered_paths: Vec<String>,
    inaccessible_dirs: Vec<String>,
    existence: &dyn PathExistence,
) -> anyhow::Result<u32> {
    let known_rows = queries::list_online_videos_under(&db.read_pool, folder)?;
    let known_online: Vec<reconcile::KnownOnlineVideo> = known_rows
        .iter()
        .map(|r| reconcile::KnownOnlineVideo {
            video_id: r.id.clone(),
            file_path: r.file_path.clone(),
        })
        .collect();
    let diff = reconcile::EnumerationResult {
        root_reachable: true,
        inaccessible_dirs,
        discovered_paths,
    };
    let decision = reconcile::confirm_suppressed_candidates(
        reconcile::decide_missing_video_ids(&known_online, &diff),
        existence,
    );
    log_missing_video_decision("manual scan", folder, "", &decision);
    if !decision.missing_ids.is_empty() {
        let conn = db.writer.lock().unwrap();
        for id in &decision.missing_ids {
            queries::update_video_status(&conn, id, "offline")?;
        }
    }
    Ok(decision.missing_ids.len() as u32)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use tests::move_fixture;
