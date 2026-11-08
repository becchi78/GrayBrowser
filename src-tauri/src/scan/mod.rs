//! Folder walking + real file I/O + DB glue around `gb_core::scan_pipeline`'s
//! pure validate/build decisions and `gb_core::reconcile`'s pure
//! classify/path-follow decisions.
//!
//! `process_detected_file` is the shared per-file entry point: reused as-is
//! by this module's `scan_folders` (manual full walk), the local realtime
//! watcher, and the NAS diff-scan/poll. Each caller passes a
//! `FolderReachability` so a moved file can be told apart from a copy.

use std::fs::File;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use gb_core::paths;
use gb_core::ports::path_existence::PathExistence;
use gb_core::reconcile::{self, DiscoveredFile, FileClassification, KnownFileMeta};
use gb_core::scan_pipeline::{self, ScannedFile, ValidationOutcome};
use walkdir::WalkDir;

use crate::adapters::long_path;
use crate::adapters::path_existence::RealPathExistence;
use crate::db::{queries, Db};
use crate::offline_sync::mark_folder_offline;
use crate::reachability::ReachabilityRegistry;
use crate::thumbnail;

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
pub struct AssumeReachable;

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

/// The outcome of processing one already-detected file. Shared by all three
/// detection paths (manual scan, realtime watch, NAS poll).
pub enum ProcessOutcome {
    Registered,
    Reconciled,
    Unchanged,
    /// A path-follow match against an `offline` row. `process_detected_file`'s
    /// "unknown path" branch looks up offline candidates by
    /// quick_hash+file_size before falling back to registering a brand-new
    /// row.
    PathFollowed {
        video_id: String,
    },
    /// The path-follow UNIQUE-collision guard. A low-frequency but
    /// genuinely *live* branch, not dead code -- see the doc comment on
    /// `register_new_path`'s collision check for exactly when this can be
    /// constructed.
    BlockedByCollision {
        video_id: String,
        colliding_video_id: String,
    },
    SkippedInvalidName {
        detected_char: char,
    },
    /// File couldn't be opened or hashed (lock, permission, vanished
    /// between discovery and here). Skipped and retried on the next
    /// scan/poll, never fatal to the batch. Not counted in `ScanSummary`
    /// (`skipped` means specifically "machine-dependent filename").
    SkippedUnreadable,
}

/// Processes one already-detected file end to end: validates its name, then
/// reconciles it against any existing DB row or registers it as new.
///
/// Caller contract: `entry_path` is already known to be a video-extension
/// file (`scan_pipeline::is_video_file`) and `file_size`/`mtime` have
/// already been read from the filesystem -- both stay the caller's
/// responsibility since each of the three detection paths gets them from a
/// different place (a `WalkDir` entry here, a stat after a `notify` event in
/// the realtime watcher, an already-collected diff-scan listing in the NAS
/// poll), and pre-filtering by extension before ever touching the DB is
/// cheaper for event-driven callers that see non-video events too.
///
/// `thumbnails_root` is the `thumbnails/` directory (never a resolved
/// per-video subdirectory) -- passed down only as far as `register_new_path`
/// actually needs it (to move a reactivated video's cached thumbnails from
/// its old resolved location to its new one). Resolved once by the command
/// layer (`crate::paths::app_data_dir`) and threaded down explicitly rather
/// than re-resolved here, matching this codebase's "OS-dependent path
/// resolution stays at the edge" convention.
///
/// `reachability` is consulted only for a new path whose content matches an
/// `online` row elsewhere (see `register_new_path`).
pub fn process_detected_file(
    db: &Db,
    thumbnails_root: &Path,
    entry_path: &Path,
    file_size: u64,
    mtime: i64,
    reachability: &dyn FolderReachability,
) -> anyhow::Result<ProcessOutcome> {
    let file_path = entry_path.to_string_lossy().to_string();
    let file_name = entry_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| file_path.clone());
    let scanned_file = ScannedFile {
        file_path: file_path.clone(),
        file_name,
        file_size,
        mtime,
    };

    // Validation runs unconditionally, first -- before the known/unknown-path
    // branch below, not just before hashing. No exception is carved out for
    // "already-registered paths don't need re-validating": the same check
    // must apply identically regardless of detection path, and keeping it
    // unconditional here means that holds structurally rather than by
    // convention.
    if let ValidationOutcome::Invalid { detected_char } = scan_pipeline::validate(&scanned_file) {
        let skipped = scan_pipeline::build_skipped_file(&scanned_file, detected_char);
        log::warn!(
            "skipped machine-dependent filename: {} (char: {detected_char})",
            skipped.file_name
        );
        let conn = db.writer.lock().unwrap();
        queries::upsert_skipped_file_found_on_disk(&conn, &skipped)
            .inspect_err(|e| log::error!("failed to record skipped file {file_path}: {e}"))?;
        return Ok(ProcessOutcome::SkippedInvalidName { detected_char });
    }

    let known = {
        let conn = db.read_pool.get()?;
        queries::find_video_by_path(&conn, &file_path)?
    };

    match known {
        Some(known) => reconcile_known_path(db, thumbnails_root, &scanned_file, &known),
        None => register_new_path(db, thumbnails_root, &scanned_file, reachability),
    }
}

/// `entry_path` already has a `videos` row (`known`). Reconciles content
/// changes and/or an `offline` -> `online` reconnection at this same path;
/// never inserts a second row.
fn reconcile_known_path(
    db: &Db,
    thumbnails_root: &Path,
    scanned_file: &ScannedFile,
    known: &queries::VideoRow,
) -> anyhow::Result<ProcessOutcome> {
    let discovered = DiscoveredFile {
        file_size: scanned_file.file_size,
        mtime: scanned_file.mtime,
    };
    let known_meta = KnownFileMeta {
        file_size: known.file_size as u64,
        mtime: known.mtime,
    };

    match reconcile::classify_discovered_file(&discovered, Some(&known_meta)) {
        FileClassification::Unchanged => {
            // Known, accepted limitation: content replaced while its mtime is
            // preserved (e.g. `robocopy /COPY:DAT`, a backup restore) is not
            // detected, so the stored quick_hash goes stale. Rehashing every
            // unchanged file would defeat the cheap mtime/size filter. A stale
            // quick_hash can make a later move of this file fail to
            // auto-follow, or match the wrong offline row; strict verification
            // is left to full_hash-based duplicate detection. The partial-copy
            // counterpart is noted in `watch::process_changed_file`.
            if known.status == "offline" {
                let conn = db.writer.lock().unwrap();
                queries::update_video_status(&conn, &known.id, "online")?;
                Ok(ProcessOutcome::Reconciled)
            } else {
                Ok(ProcessOutcome::Unchanged)
            }
        }
        FileClassification::NeedsRehash => {
            // Content matching some *other* offline row can appear at a path
            // that already has its own online row (`known`). Such a file
            // never reaches `register_new_path`, because `find_video_by_path`
            // already resolved `known`. So the rehashed quick_hash+file_size
            // is checked against offline candidates here (the same lookup
            // `register_new_path` uses), and a coincidental match is recorded
            // in `path_collisions` as a duplicate candidate instead of
            // leaving that offline row without a trace. The offline row is
            // not reactivated; only `known` is reconciled in place, and the
            // user resolves the pair via `dedup::detect_duplicate_groups`.
            // `register_new_path`'s `BlockedByCollision` is a different case:
            // a *path* already claimed by another online row.
            let quick_hash = match hash_file(&scanned_file.file_path, scanned_file.file_size) {
                Ok(h) => h,
                Err(e) => {
                    log::warn!(
                        "leaving {} as-is: failed to compute quick_hash: {e}",
                        scanned_file.file_path
                    );
                    return Ok(ProcessOutcome::SkippedUnreadable);
                }
            };
            let quick_hash_str = quick_hash.to_string();
            let conn = db.writer.lock().unwrap();

            record_rehash_coincidences(&conn, scanned_file, known, &quick_hash_str);
            apply_rehashed_metadata(&conn, thumbnails_root, scanned_file, known, &quick_hash_str)?;
            Ok(ProcessOutcome::Reconciled)
        }
        // `known` is `Some(_)` here, and classify_discovered_file's only
        // NewCandidate-producing branch requires `known: None` -- this arm
        // has no live call path, not just an unlikely one.
        FileClassification::NewCandidate => {
            unreachable!("classify_discovered_file never returns NewCandidate when `known` is Some")
        }
    }
}

/// Records, in `path_collisions`, every `offline` row whose quick_hash+size
/// now matches the freshly rehashed content of `known`. Detection only: no
/// row is reactivated. Failures are logged, never propagated. The caller
/// holds the writer lock and passes its connection.
fn record_rehash_coincidences(
    conn: &rusqlite::Connection,
    scanned_file: &ScannedFile,
    known: &queries::VideoRow,
    quick_hash_str: &str,
) {
    // Detection only (no reactivation): look up any `offline` row
    // whose quick_hash+file_size now matches the freshly computed
    // content. Each match is stored via `record_path_collision`.
    // `path_collisions` holds "an offline candidate and an online row
    // whose content may coincide, pending full_hash confirmation",
    // which fits this case too, and `dedup::detect_duplicate_groups`
    // reads every row in it uniformly.
    match queries::find_offline_candidates_by_quick_hash_and_size(
        conn,
        quick_hash_str,
        scanned_file.file_size as i64,
    ) {
        Ok(candidates) => {
            // Self-collision guard: `known` may still be `offline` here
            // (reconnecting; its status is flipped only after this check) and
            // its hash may equal its own stored values when only the
            // mtime drifted, so the query can match `known` itself.
            // Unfiltered, that records a self-pair that
            // `dedup::detect_duplicate_groups` would show as a
            // "duplicate of itself". `register_new_path` has no
            // `known` row, so it never needs this filter.
            for candidate in candidates.iter().filter(|c| c.id != known.id) {
                if let Err(e) = queries::record_path_collision(
                    conn,
                    &candidate.id,
                    &known.id,
                    &scanned_file.file_path,
                ) {
                    log::warn!(
                        "failed to record path collision from a rehash coincidence \
                         (video_id={}, colliding_video_id={}): {e}",
                        candidate.id,
                        known.id
                    );
                }
            }
        }
        Err(e) => {
            log::warn!(
                "failed to check {} against offline candidates after rehash: {e}",
                scanned_file.file_path
            );
        }
    }
}

/// Writes the rehashed size/mtime/quick_hash to `known` and flips it back
/// online if it was offline. A content change also resets the derived data
/// and removes the stale thumbnails. The caller holds the writer lock and
/// passes its connection; the transaction stays inside this function.
fn apply_rehashed_metadata(
    conn: &rusqlite::Connection,
    thumbnails_root: &Path,
    scanned_file: &ScannedFile,
    known: &queries::VideoRow,
    quick_hash_str: &str,
) -> anyhow::Result<()> {
    // An mtime-only change (same size and quick_hash) leaves the
    // derived data valid, so it must not throw away good thumbnails.
    let content_changed =
        known.file_size != scanned_file.file_size as i64 || known.quick_hash != quick_hash_str;
    if content_changed {
        // One transaction so the new hash is never committed without
        // the invalidation: the hash is what makes the next scan see
        // the file as unchanged, so a lost reset would never be
        // retried.
        let tx = conn.unchecked_transaction()?;
        queries::update_video_scan_metadata(
            &tx,
            &known.id,
            scanned_file.file_size as i64,
            scanned_file.mtime,
            quick_hash_str,
        )?;
        queries::reset_video_derived_data(&tx, &known.id)?;
        tx.commit()?;
        // Files go only after the DB commit: if the commit failed,
        // deleting them would leave a row that still claims
        // thumbnails exist.
        let watch_folders = match queries::get_watch_folders(conn) {
            Ok(folders) => folders,
            Err(e) => {
                log::warn!(
                    "failed to read watch_folders while removing stale thumbnails for \
                     video_id={}: {e}",
                    known.id
                );
                Vec::new()
            }
        };
        thumbnail::paths::remove_video_thumbnails(
            thumbnails_root,
            &watch_folders,
            &known.id,
            &known.file_path,
        );
    } else {
        queries::update_video_scan_metadata(
            conn,
            &known.id,
            scanned_file.file_size as i64,
            scanned_file.mtime,
            quick_hash_str,
        )?;
    }
    if known.status == "offline" {
        queries::update_video_status(conn, &known.id, "online")?;
    }
    Ok(())
}

/// `entry_path` has no existing `videos` row for this exact path.
/// Path-follow: matches by quick_hash+file_size against `offline` rows
/// first -- only falls back to a brand-new row when no such candidate
/// exists, or when the candidate's target path collides with an
/// already-`online` row.
///
/// With no `offline` candidate, an `online` row of the same content elsewhere
/// is either a real copy or the same file whose old folder was renamed or
/// unplugged without that being noticed yet (nothing re-checks a reachable
/// local folder on its own). Its folder is checked on the spot: if it is
/// confirmed gone, its rows are flipped `offline` first so the move is
/// followed; if it is reachable, the file is a copy and is registered as such.
fn register_new_path(
    db: &Db,
    thumbnails_root: &Path,
    scanned_file: &ScannedFile,
    reachability: &dyn FolderReachability,
) -> anyhow::Result<ProcessOutcome> {
    let quick_hash = match hash_file(&scanned_file.file_path, scanned_file.file_size) {
        Ok(h) => h,
        Err(e) => {
            log::warn!(
                "skipping {}: failed to compute quick_hash: {e}",
                scanned_file.file_path
            );
            return Ok(ProcessOutcome::SkippedUnreadable);
        }
    };
    let quick_hash_str = quick_hash.to_string();

    // Before the writer lock: a probe of a dead share can block for tens of
    // seconds, and every other writer would wait behind it.
    flip_vanished_lookalikes_offline(db, scanned_file, &quick_hash_str, reachability)?;

    // Read (candidate lookup + collision check), decision, and write all
    // happen inside one db.writer.lock() acquisition -- this is what
    // serializes path-follow against the other two detection paths
    // (realtime watch, NAS poll) sharing the same `Db`, not just against
    // concurrent manual scans. Without it, two paths could both see "no
    // collision" for the same offline candidate and both attempt to claim
    // it.
    let conn = db.writer.lock().unwrap();

    let candidate_rows = queries::find_offline_candidates_by_quick_hash_and_size(
        &conn,
        &quick_hash_str,
        scanned_file.file_size as i64,
    )?;
    let candidates: Vec<reconcile::OfflineCandidate> = candidate_rows
        .iter()
        .map(|r| reconcile::OfflineCandidate {
            video_id: r.id.clone(),
            file_path: r.file_path.clone(),
        })
        .collect();
    // `BlockedByCollision` is reachable despite the caller's
    // `find_video_by_path`: that check uses `db.read_pool` outside the writer
    // lock, and `hash_file` does disk I/O before the lock is taken. Another
    // detection path (NAS poll / realtime watch) can register an online row
    // at this exact path in that window, e.g. with nested watch folders that
    // observe the same file. Rare, but the check-then-write below runs under
    // the writer lock, so the collision is handled without a panic and
    // without touching either row.
    let collision = match candidates.first() {
        Some(candidate) => queries::is_path_used_by_online_video(
            &conn,
            &scanned_file.file_path,
            &candidate.video_id,
        )?,
        None => None,
    };

    match reconcile::decide_path_follow(&candidates, collision) {
        reconcile::PathFollowDecision::Reactivate { video_id } => {
            // `candidates.first()` is exactly the offline row
            // `decide_path_follow` picked (see its doc comment), so its
            // `file_path` is this video's pre-rewrite path.
            let old_file_path = candidates.first().map(|c| c.file_path.as_str());
            Ok(reactivate_at_new_path(
                &conn,
                thumbnails_root,
                scanned_file,
                video_id,
                old_file_path,
            ))
        }
        reconcile::PathFollowDecision::BlockedByCollision {
            video_id,
            colliding_video_id,
        } => {
            // The discovered file's own path already belongs to
            // `colliding_video_id` (an online row) -- `file_path` is
            // UNIQUE, so a fresh INSERT at this exact path could never
            // succeed even if attempted (it would either violate the
            // constraint or, via insert_video's `ON CONFLICT(file_path) DO
            // NOTHING`, silently no-op). There is nothing else to write:
            // `video_id` (the offline candidate) stays offline, the
            // colliding online row is left untouched, and this file is
            // simply left unregistered for this pass -- picked up again
            // identically on the next scan/poll.

            // Persist this pair as a duplicate-detection candidate, using
            // the same `db.writer.lock()` acquisition already held for this
            // function -- no separate lock. A failure here is logged only,
            // not propagated: the collision is still visible via this
            // scan's WARN log and `ScanSummary.collisions` count below, and
            // will be recorded again (idempotently, via
            // `record_path_collision`'s upsert) the next time this same
            // file is scanned/polled, so it's delayed, not lost.
            if let Err(e) = queries::record_path_collision(
                &conn,
                &video_id,
                &colliding_video_id,
                &scanned_file.file_path,
            ) {
                log::warn!(
                    "failed to record path collision (video_id={video_id}, \
                     colliding_video_id={colliding_video_id}): {e}"
                );
            }

            log::warn!(
                "path-follow for {} blocked: offline video_id={video_id} would collide with \
                 already-online video_id={colliding_video_id} -- leaving {video_id} offline; \
                 this file cannot be registered at a path already claimed by \
                 {colliding_video_id} and will be retried on the next scan/poll",
                scanned_file.file_path
            );
            Ok(ProcessOutcome::BlockedByCollision {
                video_id,
                colliding_video_id,
            })
        }
        reconcile::PathFollowDecision::NoMatch => {
            insert_new_video(&conn, scanned_file, quick_hash)?;
            Ok(ProcessOutcome::Registered)
        }
    }
}

/// When `scanned_file` has no `offline` candidate, checks the watch folders of
/// the `online` rows with the same content and flips the rows under each
/// folder confirmed unreachable to `offline`, so the caller's candidate lookup
/// then finds them. Rows under a reachable folder are left alone (a real
/// copy). Written directly rather than left to the unreachable hook, which
/// runs on another thread and may not have applied yet; the update only
/// touches `online` rows, so doing both is harmless.
fn flip_vanished_lookalikes_offline(
    db: &Db,
    scanned_file: &ScannedFile,
    quick_hash_str: &str,
    reachability: &dyn FolderReachability,
) -> anyhow::Result<()> {
    let (lookalikes, watch_folders) = {
        let conn = db.read_pool.get()?;
        let file_size = scanned_file.file_size as i64;
        if !queries::find_offline_candidates_by_quick_hash_and_size(
            &conn,
            quick_hash_str,
            file_size,
        )?
        .is_empty()
        {
            return Ok(());
        }
        let lookalikes = queries::find_online_lookalikes_at_other_paths(
            &conn,
            quick_hash_str,
            file_size,
            &scanned_file.file_path,
        )?;
        if lookalikes.is_empty() {
            return Ok(());
        }
        (lookalikes, queries::get_watch_folders(&conn)?)
    };

    let mut checked: Vec<&str> = Vec::new();
    for row in &lookalikes {
        let Some(folder) = paths::owning_folder(&row.file_path, &watch_folders) else {
            continue;
        };
        if checked.contains(&folder) {
            continue;
        }
        checked.push(folder);
        if !reachability.is_confirmed_unreachable(folder) {
            continue;
        }
        match mark_folder_offline(db, folder) {
            Ok(changed) => log::info!(
                "{changed} video(s) under {folder} went offline: folder unreachable while \
                 checking a possible move to {}",
                scanned_file.file_path
            ),
            Err(e) => log::error!("failed to mark videos under {folder} offline: {e:#}"),
        }
    }
    Ok(())
}

/// Moves the offline row `video_id` to the discovered path and brings it
/// online, then moves its cached thumbnails along. The caller holds the
/// writer lock (the same acquisition as its collision pre-check) and passes
/// its connection, so the check and this write stay atomic.
fn reactivate_at_new_path(
    conn: &rusqlite::Connection,
    thumbnails_root: &Path,
    scanned_file: &ScannedFile,
    video_id: String,
    old_file_path: Option<&str>,
) -> ProcessOutcome {
    match queries::update_video_path_and_status(
        conn,
        &video_id,
        &scanned_file.file_path,
        &scanned_file.file_name,
        "online",
    ) {
        Ok(()) => {
            // Best-effort: a reactivated video's cached thumbnails
            // (if any were ever generated) live under a subdirectory
            // resolved from its *old* file_path -- now that the path
            // has moved, they must move with it, or a later
            // `get_thumbnails`/`list_videos_missing_thumbnails` call
            // would look in the wrong (new) directory and find
            // nothing, needlessly regenerating what already exists.
            // A failure here never affects `video_id`'s already-
            // committed DB write above: thumbnails are regenerable,
            // so this is logged, not propagated.
            if let Some(old_file_path) = old_file_path {
                let watch_folders = match queries::get_watch_folders(conn) {
                    Ok(folders) => folders,
                    Err(e) => {
                        log::warn!(
                            "failed to read watch_folders while moving thumbnails for \
                             reactivated video_id={video_id}: {e}"
                        );
                        Vec::new()
                    }
                };
                if !thumbnail::paths::move_video_thumbnails(
                    thumbnails_root,
                    &watch_folders,
                    &video_id,
                    old_file_path,
                    &scanned_file.file_path,
                ) {
                    log::warn!(
                        "failed to fully move cached thumbnails for reactivated \
                         video_id={video_id} from {} to {}",
                        old_file_path,
                        scanned_file.file_path
                    );
                }
            }
            ProcessOutcome::PathFollowed { video_id }
        }
        Err(e) => {
            // Final defense: the pre-check above ran inside this
            // same lock acquisition, so this is not an ordinary
            // TOCTOU race --
            // it means some other constraint slipped past
            // is_path_used_by_online_video. Log and leave the
            // offline row untouched rather than propagate/panic;
            // the file is picked up again on the next scan/poll.
            log::warn!(
                "path-follow write for video_id={video_id} to {} failed despite the \
                 pre-check (UNIQUE constraint?): {e}",
                scanned_file.file_path
            );
            ProcessOutcome::SkippedUnreadable
        }
    }
}

fn insert_new_video(
    conn: &rusqlite::Connection,
    scanned_file: &ScannedFile,
    quick_hash: u64,
) -> anyhow::Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    let new_video = scan_pipeline::build_new_video(scanned_file, quick_hash, id);
    queries::insert_video(conn, &new_video)
        .inspect_err(|e| log::error!("failed to insert video {}: {e}", scanned_file.file_path))?;
    Ok(())
}

/// `pub(crate)` (not private) so the `.wb` import pipeline
/// (`crate::wb_import::pipeline`) can reuse this exact quick_hash-computation
/// logic -- long-path conversion + `File::open` + `gb_core::hash::quick_hash`
/// -- for online `.wb` rows instead of re-implementing it.
pub(crate) fn hash_file(file_path: &str, file_size: u64) -> anyhow::Result<u64> {
    let mut file = File::open(long_path::to_long_path(Path::new(file_path)))?;
    Ok(gb_core::hash::quick_hash(&mut file, file_size)?)
}

/// Converts `std::fs::Metadata::modified()` to Unix seconds, shared by
/// `scan_folders` (this module) and the realtime watcher
/// (`crate::watch::handle_watch_event`) so both detection paths derive
/// `mtime` the same way. `Err` covers both "the platform can't report an
/// mtime at all" and "the mtime predates 1970" -- both cases the caller
/// treats identically (WARN + skip this file, a catch-and-continue policy).
pub fn mtime_from_metadata(metadata: &std::fs::Metadata) -> Result<i64, String> {
    let modified = metadata
        .modified()
        .map_err(|e| format!("failed to read mtime: {e}"))?;
    modified
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .map_err(|e| format!("mtime predates the Unix epoch: {e}"))
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
    if let Some(check) = &decision.individually_checked {
        if check.confirmed_missing_count > 0 {
            log::info!(
                "manual scan for {folder}: {} of {} withheld video(s) confirmed not found \
                 one by one (guard reason={:?}), marking them offline",
                check.confirmed_missing_count,
                check.checked_count,
                check.reason
            );
        }
    }
    if let Some(guard) = &decision.suppressed {
        log::warn!(
            "manual scan for {folder}: holding {} video(s) online (known={}, discovered={}, \
             reason={:?}) -- enumeration looks broken, not treating this as genuine deletion",
            guard.candidate_count,
            guard.known_online_count,
            guard.discovered_count,
            guard.reason
        );
    }
    if !decision.missing_ids.is_empty() {
        let conn = db.writer.lock().unwrap();
        for id in &decision.missing_ids {
            queries::update_video_status(&conn, id, "offline")?;
        }
    }
    Ok(decision.missing_ids.len() as u32)
}

/// A catalog with folders to register videos in and move them between, behind a
/// registry whose probe the test controls. Shared by the tests of every
/// detection path.
#[cfg(test)]
pub(crate) mod move_fixture {
    use super::*;
    use crate::db::test_support::init_temp_db;
    use crate::events::NoopReachabilityNotifier;
    use gb_core::testing::fake_folder_probe::FakeFolderProbe;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;

    pub const CONTENT: &[u8] = b"the same video bytes";

    pub struct MoveFixture {
        pub db: Db,
        pub thumbs: tempfile::TempDir,
        pub dirs: Vec<tempfile::TempDir>,
        pub probe: Arc<FakeFolderProbe>,
        pub registry: ReachabilityRegistry,
        pub watch_folders: Vec<String>,
        _db_dir: tempfile::TempDir,
    }

    impl MoveFixture {
        pub fn new(folder_count: usize) -> Self {
            let (db_dir, db) = init_temp_db();
            let dirs: Vec<_> = (0..folder_count)
                .map(|_| tempfile::tempdir().unwrap())
                .collect();
            let watch_folders: Vec<String> = dirs
                .iter()
                .map(|d| d.path().to_string_lossy().to_string())
                .collect();
            queries::set_watch_folders(&db.writer.lock().unwrap(), &watch_folders).unwrap();
            let probe = Arc::new(FakeFolderProbe::default());
            let registry =
                ReachabilityRegistry::new(probe.clone(), Arc::new(NoopReachabilityNotifier));
            Self {
                db,
                thumbs: tempfile::tempdir().unwrap(),
                dirs,
                probe,
                registry,
                watch_folders,
                _db_dir: db_dir,
            }
        }

        pub fn folder(&self, i: usize) -> &str {
            &self.watch_folders[i]
        }

        /// Writes `name` into folder `i` and registers it through a manual scan
        /// that never consults reachability, so the registry stays unprobed.
        pub fn register_at(&self, i: usize, name: &str) -> PathBuf {
            let path = self.write_at(i, name);
            scan_folders(
                &[self.folder(i).to_string()],
                &self.db,
                self.thumbs.path(),
                &AssumeReachable,
            )
            .unwrap();
            path
        }

        pub fn write_at(&self, i: usize, name: &str) -> PathBuf {
            let path = self.dirs[i].path().join(name);
            fs::write(&path, CONTENT).unwrap();
            path
        }

        pub fn id_at(&self, path: &Path) -> String {
            let conn = self.db.read_pool.get().unwrap();
            queries::find_video_by_path(&conn, &path.to_string_lossy())
                .unwrap()
                .unwrap()
                .id
        }

        pub fn rows(&self) -> Vec<(String, String)> {
            let conn = self.db.read_pool.get().unwrap();
            let mut stmt = conn
                .prepare("SELECT file_path, status FROM videos ORDER BY file_path")
                .unwrap();
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows
        }

        pub fn tag(&self, id: &str, name: &str) {
            let conn = self.db.writer.lock().unwrap();
            conn.execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", [name])
                .unwrap();
            conn.execute(
                "INSERT INTO video_tags (video_id, tag_id)
                 SELECT ?1, id FROM tags WHERE name = ?2",
                [id, name],
            )
            .unwrap();
        }

        pub fn tag_count(&self, id: &str) -> i64 {
            let conn = self.db.read_pool.get().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM video_tags WHERE video_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
        }

        /// Where `id`'s first thumbnail slot lives for a file at `path`.
        pub fn thumbnail_slot(&self, id: &str, path: &Path) -> PathBuf {
            thumbnail::paths::video_thumbnail_dir(
                self.thumbs.path(),
                &self.watch_folders,
                &path.to_string_lossy(),
            )
            .join(format!("{id}_0.webp"))
        }

        pub fn put_thumbnail(&self, id: &str, path: &Path) {
            let slot = self.thumbnail_slot(id, path);
            fs::create_dir_all(slot.parent().unwrap()).unwrap();
            fs::write(slot, "thumb").unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::init_temp_db;
    use std::fs;

    /// A throwaway `thumbnails/` root for tests that don't care about its
    /// contents -- none of the scenarios below reactivate a video that
    /// actually has cached thumbnails to move, so this only needs to be a
    /// valid, empty directory.
    fn temp_thumbs_root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// This scenario -- a path-follow target that's already claimed by
    /// a *different*, currently-online row -- can never actually be reached
    /// through the public `scan_folders`/`process_detected_file` entry
    /// point: `process_detected_file` looks up `find_video_by_path(new_path)`
    /// *before* `register_new_path` runs, and if any row (online or offline)
    /// already has that exact `file_path`, it's routed to
    /// `reconcile_known_path` instead -- never reaching
    /// `register_new_path`'s candidate/collision check at all. So the
    /// collision guard is a final-defense layer against a state that a real
    /// `WalkDir`-driven scan/watch/poll cannot produce given this module's
    /// current query semantics, not an ordinary hit path -- `register_new_path`
    /// is called directly here (bypassing `find_video_by_path`'s guard) to
    /// construct it anyway and prove the fallback doesn't panic and leaves
    /// both existing rows exactly as they were.
    ///
    /// Also proves a corollary of `file_path` being `UNIQUE`: the "new file"
    /// can never actually be registered as a separate row at the path the
    /// online row (`r2_id`) already occupies (an insert there is structurally
    /// impossible, not merely undesirable) -- the DB ends up with exactly the
    /// original two rows, unchanged, not three.
    #[test]
    fn register_new_path_leaves_both_rows_untouched_when_the_target_path_collides_with_an_online_row(
    ) {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();

        // `r1_id`: an offline candidate for path-follow.
        let old_dir = tempfile::tempdir().unwrap();
        let old_path = old_dir.path().join("old.mp4");
        fs::write(&old_path, b"shared content").unwrap();
        let old_folders = vec![old_dir.path().to_string_lossy().to_string()];
        scan_folders(&old_folders, &db, thumbs_root.path(), &AssumeReachable).unwrap();
        let r1_id: String = {
            let conn = db.writer.lock().unwrap();
            conn.execute("UPDATE videos SET status = 'offline'", [])
                .unwrap();
            conn.query_row("SELECT id FROM videos", [], |r| r.get(0))
                .unwrap()
        };

        // `r2_id`: an online row already claiming the path we're about to target.
        let new_dir = tempfile::tempdir().unwrap();
        let new_path = new_dir.path().join("new.mp4");
        fs::write(&new_path, b"different content, a different length").unwrap();
        let new_folders = vec![new_dir.path().to_string_lossy().to_string()];
        scan_folders(&new_folders, &db, thumbs_root.path(), &AssumeReachable).unwrap();
        let r2_id: String = {
            let conn = db.writer.lock().unwrap();
            conn.query_row(
                "SELECT id FROM videos WHERE file_path = ?1",
                [new_path.to_string_lossy().to_string()],
                |r| r.get(0),
            )
            .unwrap()
        };

        // Overwrite new.mp4's bytes so its quick_hash+file_size now match
        // the offline row's -- the physical trigger for a path-follow match --
        // while its path is still the online row's already-registered path.
        fs::write(&new_path, b"shared content").unwrap();
        let metadata = fs::metadata(&new_path).unwrap();
        let scanned_file = ScannedFile {
            file_path: new_path.to_string_lossy().to_string(),
            file_name: "new.mp4".to_string(),
            file_size: metadata.len(),
            mtime: mtime_from_metadata(&metadata).unwrap(),
        };

        let outcome =
            register_new_path(&db, thumbs_root.path(), &scanned_file, &AssumeReachable).unwrap();
        match outcome {
            ProcessOutcome::BlockedByCollision {
                video_id,
                colliding_video_id,
            } => {
                assert_eq!(video_id, r1_id);
                assert_eq!(colliding_video_id, r2_id);
            }
            _ => panic!("expected BlockedByCollision"),
        }

        let conn = db.writer.lock().unwrap();
        let (r1_status, r1_path): (String, String) = conn
            .query_row(
                "SELECT status, file_path FROM videos WHERE id = ?1",
                [&r1_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            r1_status, "offline",
            "R1 must stay offline, not be rewritten"
        );
        assert_eq!(r1_path, old_path.to_string_lossy().to_string());

        let (r2_status, r2_path): (String, String) = conn
            .query_row(
                "SELECT status, file_path FROM videos WHERE id = ?1",
                [&r2_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(r2_status, "online", "R2 must be untouched");
        assert_eq!(r2_path, new_path.to_string_lossy().to_string());

        let total_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            total_count, 2,
            "file_path is UNIQUE -- a third row at R2's exact path is structurally \
             impossible, so no insert happens at all; the file is left unregistered \
             this pass"
        );

        // The collision must also be persisted to `path_collisions`
        // (queries::record_path_collision), not merely WARN-logged -- this
        // is what makes it visible to `dedup::detect_duplicate_groups`
        // later.
        let collisions = queries::list_path_collisions(&db.read_pool).unwrap();
        assert_eq!(
            collisions.len(),
            1,
            "the collision must be recorded exactly once"
        );
        assert_eq!(collisions[0].video_id, r1_id);
        assert_eq!(collisions[0].colliding_video_id, r2_id);
        assert_eq!(
            collisions[0].attempted_path,
            new_path.to_string_lossy().to_string()
        );
    }

    /// A `known` (already-registered) row (`r2_id`) has its on-disk content
    /// overwritten so that its *new* quick_hash+file_size coincide with an
    /// unrelated `offline` row's (`r1_id`; see `reconcile_known_path`'s
    /// `NeedsRehash` arm). `reconcile_known_path` rehashes the known row in
    /// place (never reactivating the offline one) and must also persist the
    /// pair to `path_collisions` so `dedup::detect_duplicate_groups` can
    /// surface it as a duplicate candidate, instead of silently leaving the
    /// offline row with no trace of the coincidence.
    #[test]
    fn reconcile_known_path_records_a_path_collision_when_the_rehash_coincidentally_matches_an_offline_row(
    ) {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();

        // `r1_id`: an unrelated offline row, content "shared content".
        let old_dir = tempfile::tempdir().unwrap();
        let old_path = old_dir.path().join("old.mp4");
        fs::write(&old_path, b"shared content").unwrap();
        let old_folders = vec![old_dir.path().to_string_lossy().to_string()];
        scan_folders(&old_folders, &db, thumbs_root.path(), &AssumeReachable).unwrap();
        let r1_id: String = {
            let conn = db.writer.lock().unwrap();
            conn.execute("UPDATE videos SET status = 'offline'", [])
                .unwrap();
            conn.query_row("SELECT id FROM videos", [], |r| r.get(0))
                .unwrap()
        };

        // `r2_id`: a known, online row -- registered with different content,
        // so its initial quick_hash/file_size do *not* match `r1_id`'s.
        let new_dir = tempfile::tempdir().unwrap();
        let new_path = new_dir.path().join("new.mp4");
        fs::write(&new_path, b"original R2 content, a different length").unwrap();
        let new_folders = vec![new_dir.path().to_string_lossy().to_string()];
        scan_folders(&new_folders, &db, thumbs_root.path(), &AssumeReachable).unwrap();
        let r2_id: String = {
            let conn = db.writer.lock().unwrap();
            conn.query_row(
                "SELECT id FROM videos WHERE file_path = ?1",
                [new_path.to_string_lossy().to_string()],
                |r| r.get(0),
            )
            .unwrap()
        };

        // Overwrite the known row's file in place (same path) with bytes
        // identical to the offline row's -- the physical trigger for both
        // NeedsRehash (file_size differs from the stored metadata) and the
        // coincidental-match check (new quick_hash+file_size equal the
        // offline row's).
        fs::write(&new_path, b"shared content").unwrap();
        let metadata = fs::metadata(&new_path).unwrap();
        let scanned_file = ScannedFile {
            file_path: new_path.to_string_lossy().to_string(),
            file_name: "new.mp4".to_string(),
            file_size: metadata.len(),
            mtime: mtime_from_metadata(&metadata).unwrap(),
        };
        let known = {
            let conn = db.read_pool.get().unwrap();
            queries::find_video_by_path(&conn, &scanned_file.file_path)
                .unwrap()
                .unwrap()
        };

        let outcome = reconcile_known_path(&db, thumbs_root.path(), &scanned_file, &known).unwrap();
        assert!(
            matches!(outcome, ProcessOutcome::Reconciled),
            "R2 is always just reconciled in place, never reactivated/replaced"
        );

        let conn = db.writer.lock().unwrap();
        let (r2_status, r2_path, r2_quick_hash): (String, String, String) = conn
            .query_row(
                "SELECT status, file_path, quick_hash FROM videos WHERE id = ?1",
                [&r2_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(r2_status, "online", "R2 keeps its own online status");
        assert_eq!(r2_path, new_path.to_string_lossy().to_string());

        let r1_status: String = conn
            .query_row("SELECT status FROM videos WHERE id = ?1", [&r1_id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            r1_status, "offline",
            "reconcile_known_path never reactivates R1 -- that's out of scope for this arm"
        );

        let collisions = queries::list_path_collisions(&db.read_pool).unwrap();
        assert_eq!(
            collisions.len(),
            1,
            "the coincidental rehash match must be recorded exactly once"
        );
        assert_eq!(
            collisions[0].video_id, r1_id,
            "video_id is the offline candidate"
        );
        assert_eq!(
            collisions[0].colliding_video_id, r2_id,
            "colliding_video_id is the known/online row that got rehashed"
        );
        assert_eq!(collisions[0].attempted_path, r2_path);
        assert_eq!(
            r2_quick_hash,
            gb_core::hash::quick_hash(&mut File::open(&new_path).unwrap(), metadata.len())
                .unwrap()
                .to_string(),
            "R2's stored quick_hash must reflect its new content"
        );
    }

    /// Self-collision regression: `known` itself can be `status='offline'`
    /// at the moment `reconcile_known_path` runs its `NeedsRehash` arm -- this
    /// happens whenever a row is reconnecting from offline to online at its
    /// *own* unchanged path (e.g. a backup restore or drive reconnect that
    /// preserves file bytes but not `mtime`), since `update_video_status`'s
    /// online flip only happens *after* the coincidental-match lookup, not
    /// before. If the freshly-recomputed quick_hash+file_size still equal
    /// `known`'s own stale stored values (content genuinely unchanged, only
    /// `mtime` drifted), `find_offline_candidates_by_quick_hash_and_size`
    /// matches `known` against itself. Unfiltered, that would record a
    /// `path_collisions` row pairing `known.id` with itself -- a nonsensical
    /// "duplicate of itself" that `dedup::detect_duplicate_groups` would
    /// then surface to the user. This must never happen.
    #[test]
    fn reconcile_known_path_does_not_record_a_self_collision_when_the_rehash_matches_its_own_stale_row(
    ) {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("video.mp4");
        fs::write(&path, b"shared content").unwrap();
        let folders = vec![dir.path().to_string_lossy().to_string()];
        scan_folders(&folders, &db, thumbs_root.path(), &AssumeReachable).unwrap();

        let video_id: String = {
            let conn = db.writer.lock().unwrap();
            conn.query_row("SELECT id FROM videos", [], |r| r.get(0))
                .unwrap()
        };

        // Simulate the row having gone offline (e.g. drive disconnect) and
        // its stored mtime now disagreeing with the file's real mtime on
        // disk (e.g. the drive/backup preserved content but not
        // timestamps), *without* touching the file's actual bytes or the
        // stored quick_hash/file_size -- both still match what a fresh
        // rehash of the unchanged file will compute.
        {
            let conn = db.writer.lock().unwrap();
            conn.execute(
                "UPDATE videos SET status = 'offline', mtime = mtime - 1000",
                [],
            )
            .unwrap();
        }

        let metadata = fs::metadata(&path).unwrap();
        let scanned_file = ScannedFile {
            file_path: path.to_string_lossy().to_string(),
            file_name: "video.mp4".to_string(),
            file_size: metadata.len(),
            mtime: mtime_from_metadata(&metadata).unwrap(),
        };
        let known = {
            let conn = db.read_pool.get().unwrap();
            queries::find_video_by_path(&conn, &scanned_file.file_path)
                .unwrap()
                .unwrap()
        };
        assert_eq!(
            known.status, "offline",
            "precondition: known must still be offline when NeedsRehash is classified, since \
             classify_discovered_file's mtime mismatch is what routes here in the first place"
        );

        let outcome = reconcile_known_path(&db, thumbs_root.path(), &scanned_file, &known).unwrap();
        assert!(
            matches!(outcome, ProcessOutcome::Reconciled),
            "reconnecting at the same path with unchanged content is still an ordinary reconcile"
        );

        let conn = db.writer.lock().unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM videos WHERE id = ?1",
                [&video_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "online", "the row must be flipped back online");

        let collisions = queries::list_path_collisions(&db.read_pool).unwrap();
        assert!(
            collisions.is_empty(),
            "a row must never be recorded as colliding with itself in path_collisions"
        );
    }

    /// Regression companion to the test above: an ordinary `NeedsRehash`
    /// reconciliation whose new content does *not* coincide with any
    /// `offline` row's quick_hash+file_size must not write anything to
    /// `path_collisions`.
    #[test]
    fn reconcile_known_path_does_not_record_a_collision_when_no_offline_row_matches_the_rehash() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("video.mp4");
        fs::write(&path, b"original content").unwrap();
        let folders = vec![dir.path().to_string_lossy().to_string()];
        scan_folders(&folders, &db, thumbs_root.path(), &AssumeReachable).unwrap();

        // Change the content to something with no matching offline
        // candidate anywhere in the DB (there isn't even another row).
        fs::write(&path, b"changed content, nothing else shares this hash").unwrap();
        let metadata = fs::metadata(&path).unwrap();
        let scanned_file = ScannedFile {
            file_path: path.to_string_lossy().to_string(),
            file_name: "video.mp4".to_string(),
            file_size: metadata.len(),
            mtime: mtime_from_metadata(&metadata).unwrap(),
        };
        let known = {
            let conn = db.read_pool.get().unwrap();
            queries::find_video_by_path(&conn, &scanned_file.file_path)
                .unwrap()
                .unwrap()
        };

        let outcome = reconcile_known_path(&db, thumbs_root.path(), &scanned_file, &known).unwrap();
        assert!(matches!(outcome, ProcessOutcome::Reconciled));

        let collisions = queries::list_path_collisions(&db.read_pool).unwrap();
        assert!(
            collisions.is_empty(),
            "an ordinary rehash with no offline-candidate match must not write to \
             path_collisions"
        );
    }

    /// Regression test for `register_new_path`'s `Reactivate` branch: a
    /// path-follow reactivation must not just rewrite `file_path` in the
    /// DB, it must also move the video's already-generated cached
    /// thumbnails from the subdirectory resolved for its *old* file_path to
    /// the one resolved for its *new* file_path (`thumbnail::paths::
    /// move_video_thumbnails`) -- otherwise a later `get_thumbnails` call
    /// would look in the (now wrong) new location and find nothing, even
    /// though the thumbnails were already generated and simply never moved.
    #[test]
    fn reactivating_a_video_via_path_follow_moves_its_cached_thumbnails_to_the_new_folders_subdir()
    {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();

        let old_dir = tempfile::tempdir().unwrap();
        let new_dir = tempfile::tempdir().unwrap();
        let old_path = old_dir.path().join("old.mp4");
        fs::write(&old_path, b"shared content for path-follow").unwrap();

        let old_folder = old_dir.path().to_string_lossy().to_string();
        let new_folder = new_dir.path().to_string_lossy().to_string();
        let watch_folders = vec![old_folder.clone(), new_folder.clone()];
        {
            let conn = db.writer.lock().unwrap();
            queries::set_watch_folders(&conn, &watch_folders).unwrap();
        }

        // Register the video under old_dir, then simulate it going offline
        // (e.g. the drive holding old_dir was disconnected).
        scan_folders(
            std::slice::from_ref(&old_folder),
            &db,
            thumbs_root.path(),
            &AssumeReachable,
        )
        .unwrap();
        let video_id: String = {
            let conn = db.writer.lock().unwrap();
            conn.execute("UPDATE videos SET status = 'offline'", [])
                .unwrap();
            conn.query_row("SELECT id FROM videos", [], |r| r.get(0))
                .unwrap()
        };

        // Simulate 6 already-generated thumbnail files sitting in the
        // subdirectory resolved for the video's *old* file_path.
        let old_video_dir = thumbnail::paths::video_thumbnail_dir(
            thumbs_root.path(),
            &watch_folders,
            &old_path.to_string_lossy(),
        );
        std::fs::create_dir_all(&old_video_dir).unwrap();
        for i in 0..thumbnail::worker::THUMBNAILS_PER_VIDEO {
            std::fs::write(
                old_video_dir.join(format!("{video_id}_{i}.webp")),
                format!("slot-{i}"),
            )
            .unwrap();
        }

        // The file reappears at a new path (under a different registered
        // folder) with identical content -- a path-follow match.
        let new_path = new_dir.path().join("new.mp4");
        fs::write(&new_path, b"shared content for path-follow").unwrap();

        scan_folders(
            std::slice::from_ref(&new_folder),
            &db,
            thumbs_root.path(),
            &AssumeReachable,
        )
        .unwrap();

        let conn = db.writer.lock().unwrap();
        let (status, file_path): (String, String) = conn
            .query_row(
                "SELECT status, file_path FROM videos WHERE id = ?1",
                [&video_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "online", "the video must be reactivated online");
        assert_eq!(
            file_path,
            new_path.to_string_lossy().to_string(),
            "the video's id must be reused (reactivated), not re-registered"
        );
        drop(conn);

        let new_video_dir = thumbnail::paths::video_thumbnail_dir(
            thumbs_root.path(),
            &watch_folders,
            &new_path.to_string_lossy(),
        );
        for i in 0..thumbnail::worker::THUMBNAILS_PER_VIDEO {
            assert!(
                !old_video_dir.join(format!("{video_id}_{i}.webp")).exists(),
                "slot {i} must no longer exist in the old folder's thumbnail subdirectory"
            );
            assert_eq!(
                std::fs::read_to_string(new_video_dir.join(format!("{video_id}_{i}.webp")))
                    .unwrap(),
                format!("slot-{i}"),
                "slot {i} must exist, with its original content, in the new folder's \
                 thumbnail subdirectory"
            );
        }
    }

    /// Registers `content` at a fresh path, marks the row as fully derived
    /// (thumbnails ready, duration probed) and writes its 6 slot files plus
    /// a neighbouring video's 6 slot files in the same directory. Returns
    /// `(video_id, path, video_dir, thumbs_root)` once set up.
    fn setup_video_with_derived_data(
        db: &Db,
        content: &[u8],
    ) -> (
        String,
        std::path::PathBuf,
        std::path::PathBuf,
        tempfile::TempDir,
        tempfile::TempDir,
    ) {
        let thumbs_root = temp_thumbs_root();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("video.mp4");
        fs::write(&path, content).unwrap();
        let folders = vec![dir.path().to_string_lossy().to_string()];
        {
            let conn = db.writer.lock().unwrap();
            queries::set_watch_folders(&conn, &folders).unwrap();
        }
        scan_folders(&folders, db, thumbs_root.path(), &AssumeReachable).unwrap();
        let video_id: String = {
            let conn = db.writer.lock().unwrap();
            conn.execute(
                "UPDATE videos SET thumbnail_ready = 1, thumbnail_attempts = 2,
                 duration = 5, probed_at = CURRENT_TIMESTAMP, metadata_attempts = 1",
                [],
            )
            .unwrap();
            conn.query_row("SELECT id FROM videos", [], |r| r.get(0))
                .unwrap()
        };
        let video_dir = thumbnail::paths::video_thumbnail_dir(
            thumbs_root.path(),
            &folders,
            &path.to_string_lossy(),
        );
        fs::create_dir_all(&video_dir).unwrap();
        for i in 0..thumbnail::worker::THUMBNAILS_PER_VIDEO {
            fs::write(video_dir.join(format!("{video_id}_{i}.webp")), "mine").unwrap();
            fs::write(video_dir.join(format!("other-video_{i}.webp")), "other").unwrap();
        }
        (video_id, path, video_dir, thumbs_root, dir)
    }

    fn derived_state(db: &Db, id: &str) -> (i64, i64, Option<i64>, Option<String>, i64) {
        let conn = db.writer.lock().unwrap();
        conn.query_row(
            "SELECT thumbnail_ready, thumbnail_attempts, duration, probed_at, metadata_attempts
             FROM videos WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap()
    }

    fn rescan_with_current_file(db: &Db, thumbs_root: &Path, path: &Path, mtime_bump: i64) {
        let metadata = fs::metadata(path).unwrap();
        process_detected_file(
            db,
            thumbs_root,
            path,
            metadata.len(),
            mtime_from_metadata(&metadata).unwrap() + mtime_bump,
            &AssumeReachable,
        )
        .unwrap();
    }

    #[test]
    fn a_size_change_invalidates_derived_data_and_removes_only_that_videos_thumbnails() {
        let (_db_dir, db) = init_temp_db();
        let (id, path, video_dir, thumbs_root, _dir) =
            setup_video_with_derived_data(&db, b"truncated head");

        fs::write(&path, b"the complete file, now much longer than before").unwrap();
        rescan_with_current_file(&db, thumbs_root.path(), &path, 0);

        assert_eq!(derived_state(&db, &id), (0, 0, None, None, 0));
        for i in 0..thumbnail::worker::THUMBNAILS_PER_VIDEO {
            assert!(!video_dir.join(format!("{id}_{i}.webp")).exists());
            assert!(
                video_dir.join(format!("other-video_{i}.webp")).exists(),
                "another video's thumbnails must be left alone"
            );
        }
    }

    #[test]
    fn a_quick_hash_change_at_the_same_size_invalidates_derived_data() {
        let (_db_dir, db) = init_temp_db();
        let (id, path, video_dir, thumbs_root, _dir) =
            setup_video_with_derived_data(&db, b"aaaaaaaaaaaaaaaa");

        fs::write(&path, b"bbbbbbbbbbbbbbbb").unwrap();
        rescan_with_current_file(&db, thumbs_root.path(), &path, 100);

        assert_eq!(derived_state(&db, &id), (0, 0, None, None, 0));
        assert!(!video_dir.join(format!("{id}_0.webp")).exists());
    }

    #[test]
    fn an_mtime_only_change_keeps_derived_data_and_thumbnails() {
        let (_db_dir, db) = init_temp_db();
        let (id, path, video_dir, thumbs_root, _dir) =
            setup_video_with_derived_data(&db, b"unchanged content");

        rescan_with_current_file(&db, thumbs_root.path(), &path, 100);

        let (ready, attempts, duration, probed_at, meta_attempts) = derived_state(&db, &id);
        assert_eq!(
            (ready, attempts, duration, meta_attempts),
            (1, 2, Some(5), 1)
        );
        assert!(probed_at.is_some());
        for i in 0..thumbnail::worker::THUMBNAILS_PER_VIDEO {
            assert!(video_dir.join(format!("{id}_{i}.webp")).exists());
        }
    }

    /// Registers `content` under `old_dir`, flips it offline, and returns
    /// `(video_id, old_path)` -- the starting point for path-follow tests.
    fn register_then_take_offline(
        db: &Db,
        thumbs_root: &Path,
        old_dir: &Path,
        content: &[u8],
    ) -> (String, std::path::PathBuf) {
        let old_path = old_dir.join("old.mp4");
        fs::write(&old_path, content).unwrap();
        let folders = vec![old_dir.to_string_lossy().to_string()];
        scan_folders(&folders, db, thumbs_root, &AssumeReachable).unwrap();
        let conn = db.writer.lock().unwrap();
        conn.execute("UPDATE videos SET status = 'offline'", [])
            .unwrap();
        let id = conn
            .query_row("SELECT id FROM videos", [], |r| r.get(0))
            .unwrap();
        (id, old_path)
    }

    fn scanned_file_of(path: &Path) -> ScannedFile {
        let metadata = fs::metadata(path).unwrap();
        ScannedFile {
            file_path: path.to_string_lossy().to_string(),
            file_name: path.file_name().unwrap().to_string_lossy().to_string(),
            file_size: metadata.len(),
            mtime: mtime_from_metadata(&metadata).unwrap(),
        }
    }

    fn known_row_of(db: &Db, scanned_file: &ScannedFile) -> queries::VideoRow {
        let conn = db.read_pool.get().unwrap();
        queries::find_video_by_path(&conn, &scanned_file.file_path)
            .unwrap()
            .unwrap()
    }

    #[test]
    fn a_thumbnail_move_failure_does_not_undo_the_path_follow_reactivation() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let old_dir = tempfile::tempdir().unwrap();
        let new_dir = tempfile::tempdir().unwrap();
        let watch_folders = vec![
            old_dir.path().to_string_lossy().to_string(),
            new_dir.path().to_string_lossy().to_string(),
        ];
        {
            let conn = db.writer.lock().unwrap();
            queries::set_watch_folders(&conn, &watch_folders).unwrap();
        }
        let (video_id, old_path) =
            register_then_take_offline(&db, thumbs_root.path(), old_dir.path(), b"moved content");

        let old_video_dir = thumbnail::paths::video_thumbnail_dir(
            thumbs_root.path(),
            &watch_folders,
            &old_path.to_string_lossy(),
        );
        fs::create_dir_all(&old_video_dir).unwrap();
        fs::write(old_video_dir.join(format!("{video_id}_0.webp")), "slot-0").unwrap();

        let new_path = new_dir.path().join("new.mp4");
        fs::write(&new_path, b"moved content").unwrap();
        let new_video_dir = thumbnail::paths::video_thumbnail_dir(
            thumbs_root.path(),
            &watch_folders,
            &new_path.to_string_lossy(),
        );
        // A directory squatting on the destination slot makes the rename fail.
        fs::create_dir_all(new_video_dir.join(format!("{video_id}_0.webp"))).unwrap();

        let outcome = register_new_path(
            &db,
            thumbs_root.path(),
            &scanned_file_of(&new_path),
            &AssumeReachable,
        )
        .unwrap();
        assert!(matches!(
        outcome,
        ProcessOutcome::PathFollowed { video_id: ref id } if *id == video_id
                ));

        let conn = db.writer.lock().unwrap();
        let (status, file_path): (String, String) = conn
            .query_row(
                "SELECT status, file_path FROM videos WHERE id = ?1",
                [&video_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "online");
        assert_eq!(file_path, new_path.to_string_lossy().to_string());
        assert!(
            old_video_dir.join(format!("{video_id}_0.webp")).exists(),
            "the thumbnail that failed to move stays where it was"
        );
    }

    #[test]
    fn a_failed_path_follow_write_leaves_the_offline_row_untouched_and_is_skipped() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let old_dir = tempfile::tempdir().unwrap();
        let new_dir = tempfile::tempdir().unwrap();
        let (video_id, old_path) =
            register_then_take_offline(&db, thumbs_root.path(), old_dir.path(), b"moved content");
        let new_path = new_dir.path().join("new.mp4");
        fs::write(&new_path, b"moved content").unwrap();
        {
            let conn = db.writer.lock().unwrap();
            conn.execute_batch(
                "CREATE TRIGGER block_path_update BEFORE UPDATE OF file_path ON videos
                 BEGIN SELECT RAISE(ABORT, 'blocked'); END;",
            )
            .unwrap();
        }

        let outcome = register_new_path(
            &db,
            thumbs_root.path(),
            &scanned_file_of(&new_path),
            &AssumeReachable,
        )
        .unwrap();
        assert!(matches!(outcome, ProcessOutcome::SkippedUnreadable));

        let conn = db.writer.lock().unwrap();
        let (status, file_path): (String, String) = conn
            .query_row(
                "SELECT status, file_path FROM videos WHERE id = ?1",
                [&video_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "offline");
        assert_eq!(file_path, old_path.to_string_lossy().to_string());
    }

    #[test]
    fn an_unreadable_new_file_is_skipped_without_touching_the_db() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let dir = tempfile::tempdir().unwrap();
        let scanned_file = ScannedFile {
            file_path: dir
                .path()
                .join("vanished.mp4")
                .to_string_lossy()
                .to_string(),
            file_name: "vanished.mp4".to_string(),
            file_size: 10,
            mtime: 1,
        };
        let outcome =
            register_new_path(&db, thumbs_root.path(), &scanned_file, &AssumeReachable).unwrap();
        assert!(matches!(outcome, ProcessOutcome::SkippedUnreadable));
        let conn = db.writer.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn a_new_path_with_no_offline_match_is_registered_as_a_new_row() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fresh.mp4");
        fs::write(&path, b"fresh content").unwrap();
        let outcome = register_new_path(
            &db,
            thumbs_root.path(),
            &scanned_file_of(&path),
            &AssumeReachable,
        )
        .unwrap();
        assert!(matches!(outcome, ProcessOutcome::Registered));
        let conn = db.writer.lock().unwrap();
        let (status, count): (String, i64) = conn
            .query_row("SELECT status, COUNT(*) FROM videos", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((status.as_str(), count), ("online", 1));
    }

    #[test]
    fn reconcile_known_path_reports_unchanged_for_an_online_row_with_matching_metadata() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("video.mp4");
        fs::write(&path, b"stable content").unwrap();
        scan_folders(
            &[dir.path().to_string_lossy().to_string()],
            &db,
            thumbs_root.path(),
            &AssumeReachable,
        )
        .unwrap();
        let scanned_file = scanned_file_of(&path);
        let known = known_row_of(&db, &scanned_file);
        let outcome = reconcile_known_path(&db, thumbs_root.path(), &scanned_file, &known).unwrap();
        assert!(matches!(outcome, ProcessOutcome::Unchanged));
    }

    #[test]
    fn reconcile_known_path_brings_an_unchanged_offline_row_back_online() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let dir = tempfile::tempdir().unwrap();
        let (video_id, path) =
            register_then_take_offline(&db, thumbs_root.path(), dir.path(), b"stable content");
        let scanned_file = scanned_file_of(&path);
        let known = known_row_of(&db, &scanned_file);
        let outcome = reconcile_known_path(&db, thumbs_root.path(), &scanned_file, &known).unwrap();
        assert!(matches!(outcome, ProcessOutcome::Reconciled));
        let conn = db.writer.lock().unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM videos WHERE id = ?1",
                [&video_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "online");
    }

    #[test]
    fn reconcile_known_path_leaves_the_row_as_is_when_the_rehash_cannot_read_the_file() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let dir = tempfile::tempdir().unwrap();
        let (video_id, path) =
            register_then_take_offline(&db, thumbs_root.path(), dir.path(), b"original");
        let mut scanned_file = scanned_file_of(&path);
        scanned_file.mtime += 100;
        let known = known_row_of(&db, &scanned_file);
        fs::remove_file(&path).unwrap();
        let outcome = reconcile_known_path(&db, thumbs_root.path(), &scanned_file, &known).unwrap();
        assert!(matches!(outcome, ProcessOutcome::SkippedUnreadable));
        let conn = db.writer.lock().unwrap();
        let (status, mtime): (String, i64) = conn
            .query_row(
                "SELECT status, mtime FROM videos WHERE id = ?1",
                [&video_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "offline");
        assert_eq!(Some(mtime), known.mtime);
    }

    #[test]
    fn reconcile_known_path_updates_metadata_and_goes_online_when_an_offline_row_changed_content() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let dir = tempfile::tempdir().unwrap();
        let (video_id, path) =
            register_then_take_offline(&db, thumbs_root.path(), dir.path(), b"original");
        fs::write(&path, b"original, now longer").unwrap();
        let scanned_file = scanned_file_of(&path);
        let known = known_row_of(&db, &scanned_file);
        let outcome = reconcile_known_path(&db, thumbs_root.path(), &scanned_file, &known).unwrap();
        assert!(matches!(outcome, ProcessOutcome::Reconciled));
        let conn = db.writer.lock().unwrap();
        let (status, size, mtime): (String, i64, i64) = conn
            .query_row(
                "SELECT status, file_size, mtime FROM videos WHERE id = ?1",
                [&video_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "online");
        assert_eq!(size, scanned_file.file_size as i64);
        assert_eq!(mtime, scanned_file.mtime);
    }

    use super::move_fixture::MoveFixture;

    fn process(fx: &MoveFixture, path: &Path, reach: &dyn FolderReachability) -> ProcessOutcome {
        let metadata = fs::metadata(path).unwrap();
        process_detected_file(
            &fx.db,
            fx.thumbs.path(),
            path,
            metadata.len(),
            mtime_from_metadata(&metadata).unwrap(),
            reach,
        )
        .unwrap()
    }

    fn path_str(p: &Path) -> String {
        p.to_string_lossy().to_string()
    }

    /// A video is registered in folder A, A then disappears (renamed or
    /// unplugged) while the registry still caches it as reachable, and the file
    /// shows up in B. No hook has run yet.
    #[test]
    fn a_move_out_of_a_vanished_folder_is_followed_before_any_hook_ran() {
        let fx = MoveFixture::new(2);
        let old = fx.register_at(0, "old.mp4");
        let id = fx.id_at(&old);
        fx.tag(&id, "keep");
        fx.put_thumbnail(&id, &old);
        fx.probe.set_reachable(fx.folder(0), false);
        let new = fx.dirs[1].path().join("new.mp4");
        fs::rename(&old, &new).unwrap();

        let outcome = process(&fx, &new, &fx.registry);

        assert!(matches!(outcome, ProcessOutcome::PathFollowed { video_id } if video_id == id));
        assert_eq!(fx.rows(), vec![(path_str(&new), "online".to_string())]);
        assert_eq!(fx.id_at(&new), id);
        assert_eq!(fx.tag_count(&id), 1);
        assert!(fx.thumbnail_slot(&id, &new).exists());
        assert!(!fx.thumbnail_slot(&id, &old).exists());
    }

    #[test]
    fn a_move_is_followed_when_the_unreachable_hook_already_flipped_the_rows() {
        let fx = MoveFixture::new(2);
        let old = fx.register_at(0, "old.mp4");
        let id = fx.id_at(&old);
        {
            let db = fx.db.clone();
            fx.registry.set_on_unreachable(move |folder| {
                crate::offline_sync::mark_folder_offline(&db, folder).unwrap();
            });
        }
        fx.probe.set_reachable(fx.folder(0), false);
        assert!(!fx.registry.is_reachable(fx.folder(0)));
        let new = fx.dirs[1].path().join("new.mp4");
        fs::rename(&old, &new).unwrap();

        let outcome = process(&fx, &new, &fx.registry);

        assert!(matches!(outcome, ProcessOutcome::PathFollowed { video_id } if video_id == id));
        assert_eq!(fx.rows(), vec![(path_str(&new), "online".to_string())]);
        assert_eq!(
            fx.probe.call_count_for(fx.folder(0)),
            1,
            "an offline candidate needs no further check"
        );
    }

    #[test]
    fn a_copy_of_a_file_in_a_reachable_folder_is_a_new_row() {
        let fx = MoveFixture::new(2);
        let old = fx.register_at(0, "old.mp4");
        let copy = fx.write_at(1, "copy.mp4");

        let outcome = process(&fx, &copy, &fx.registry);

        assert!(matches!(outcome, ProcessOutcome::Registered));
        let mut expected = vec![
            (path_str(&copy), "online".to_string()),
            (path_str(&old), "online".to_string()),
        ];
        expected.sort();
        assert_eq!(fx.rows(), expected);
    }

    #[test]
    fn only_the_row_in_the_unreachable_folder_is_followed() {
        let fx = MoveFixture::new(3);
        let in_a = fx.register_at(0, "a.mp4");
        let in_c = fx.register_at(2, "c.mp4");
        let id_a = fx.id_at(&in_a);
        fx.probe.set_reachable(fx.folder(0), false);
        let new = fx.write_at(1, "new.mp4");

        let outcome = process(&fx, &new, &fx.registry);

        assert!(matches!(outcome, ProcessOutcome::PathFollowed { video_id } if video_id == id_a));
        let mut expected = vec![
            (path_str(&in_c), "online".to_string()),
            (path_str(&new), "online".to_string()),
        ];
        expected.sort();
        assert_eq!(fx.rows(), expected);
    }

    #[test]
    fn rows_in_reachable_folders_are_not_touched_by_the_check() {
        let fx = MoveFixture::new(3);
        let in_a = fx.register_at(0, "a.mp4");
        let in_c = fx.register_at(2, "c.mp4");
        let new = fx.write_at(1, "new.mp4");

        let outcome = process(&fx, &new, &fx.registry);

        assert!(matches!(outcome, ProcessOutcome::Registered));
        let rows = fx.rows();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|(_, status)| status == "online"));
        assert!(rows.iter().any(|(p, _)| *p == path_str(&in_a)));
        assert!(rows.iter().any(|(p, _)| *p == path_str(&in_c)));
    }

    #[test]
    fn a_burst_of_copies_probes_each_reachable_folder_only_once() {
        let fx = MoveFixture::new(2);
        fx.register_at(0, "old.mp4");
        for i in 0..6 {
            let copy = fx.write_at(1, &format!("copy{i}.mp4"));
            assert!(matches!(
                process(&fx, &copy, &fx.registry),
                ProcessOutcome::Registered
            ));
        }

        assert_eq!(fx.probe.call_count_for(fx.folder(0)), 1);
        assert_eq!(fx.probe.call_count_for(fx.folder(1)), 1);
        assert_eq!(fx.rows().len(), 7);
    }

    #[test]
    fn a_lookalike_outside_every_watch_folder_is_left_alone() {
        let fx = MoveFixture::new(2);
        let new = fx.write_at(1, "new.mp4");
        let stray = tempfile::tempdir().unwrap();
        fs::write(stray.path().join("stray.mp4"), move_fixture::CONTENT).unwrap();
        scan_folders(
            &[path_str(stray.path())],
            &fx.db,
            fx.thumbs.path(),
            &AssumeReachable,
        )
        .unwrap();

        let outcome = process(&fx, &new, &fx.registry);

        assert!(matches!(outcome, ProcessOutcome::Registered));
        assert_eq!(fx.rows().len(), 2);
        assert!(
            fx.probe.calls().is_empty(),
            "no owning folder, nothing to probe"
        );
    }

    #[test]
    fn a_manual_scan_follows_a_move_out_of_a_vanished_folder() {
        let fx = MoveFixture::new(2);
        let old = fx.register_at(0, "old.mp4");
        let id = fx.id_at(&old);
        fx.probe.set_reachable(fx.folder(0), false);
        let new = fx.dirs[1].path().join("new.mp4");
        fs::rename(&old, &new).unwrap();

        let summary = scan_folders(
            &[fx.folder(1).to_string()],
            &fx.db,
            fx.thumbs.path(),
            &fx.registry,
        )
        .unwrap();

        assert_eq!((summary.reactivated, summary.registered), (1, 0));
        assert_eq!(fx.id_at(&new), id);
        assert_eq!(fx.rows().len(), 1);
    }

    /// The only video of a folder moved somewhere no watch folder covers:
    /// the walk finds nothing (the `NothingDiscovered` guard), but the
    /// one-by-one check gets NotFound from the real filesystem, so the old
    /// row still goes offline.
    #[test]
    fn a_manual_scan_offlines_the_only_video_moved_out_of_its_folder() {
        let fx = MoveFixture::new(1);
        let old = fx.register_at(0, "only.mp4");
        let elsewhere = tempfile::tempdir().unwrap();
        fs::rename(&old, elsewhere.path().join("only.mp4")).unwrap();

        let summary = scan_folders(
            &[fx.folder(0).to_string()],
            &fx.db,
            fx.thumbs.path(),
            &fx.registry,
        )
        .unwrap();

        assert_eq!(summary.went_offline, 1);
        assert_eq!(fx.rows(), vec![(path_str(&old), "offline".to_string())]);
    }

    /// The per-path check answers NotFound for every file of a deleted
    /// folder, so a folder that is gone must be kept away from it. With the
    /// real filesystem, a watch folder deleted outright leaves all of its
    /// videos online, for every count up to the check's limit.
    #[test]
    fn a_manual_scan_keeps_the_videos_of_a_deleted_watch_folder_online() {
        for count in [
            1,
            3,
            gb_core::reconcile::MAX_CANDIDATES_FOR_INDIVIDUAL_CHECK,
        ] {
            let fx = MoveFixture::new(1);
            for i in 1..count {
                fx.write_at(0, &format!("v{i}.mp4"));
            }
            // Registers every file written so far in one scan.
            fx.register_at(0, "v0.mp4");
            assert_eq!(fx.rows().len(), count);
            fs::remove_dir_all(fx.dirs[0].path()).unwrap();

            let summary = scan_folders(
                &[fx.folder(0).to_string()],
                &fx.db,
                fx.thumbs.path(),
                &fx.registry,
            )
            .unwrap();

            assert_eq!(summary.went_offline, 0, "count={count}");
            assert!(
                fx.rows().iter().all(|(_, status)| status == "online"),
                "count={count}"
            );
        }
    }

    /// Stand-in for a listing that missed a file which is in fact still
    /// there: the walk finds nothing, and the OS says the path exists, so
    /// the guard's hold stands.
    #[test]
    fn a_manual_scan_keeps_the_only_video_online_when_the_os_says_it_exists() {
        use gb_core::testing::fake_path_existence::FakePathExistence;

        let fx = MoveFixture::new(1);
        let old = fx.register_at(0, "only.mp4");
        let elsewhere = tempfile::tempdir().unwrap();
        fs::rename(&old, elsewhere.path().join("only.mp4")).unwrap();
        let existence = FakePathExistence::with_paths(&[&path_str(&old)]);

        let summary = scan_folders_with_existence(
            &[fx.folder(0).to_string()],
            &fx.db,
            fx.thumbs.path(),
            &fx.registry,
            &existence,
        )
        .unwrap();

        assert_eq!(summary.went_offline, 0);
        assert_eq!(fx.rows(), vec![(path_str(&old), "online".to_string())]);
    }
}
