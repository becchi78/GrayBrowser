//! The per-file half of a scan: validating one detected file's name and
//! reconciling it against the catalog (or registering it as new).

use std::fs::File;
use std::path::Path;
use std::time::UNIX_EPOCH;

use gb_core::paths;
use gb_core::reconcile::{self, DiscoveredFile, FileClassification, KnownFileMeta};
use gb_core::scan_pipeline::{self, ScannedFile, ValidationOutcome};

use super::FolderReachability;
use crate::adapters::long_path;
use crate::db::{queries, Db};
use crate::offline_sync::mark_folder_offline;
use crate::thumbnail;

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

impl ProcessOutcome {
    /// Whether the catalog changed, i.e. the frontend must be told and the
    /// thumbnail/metadata queues woken. The other outcomes never write.
    pub fn changed_catalog(&self) -> bool {
        matches!(
            self,
            Self::Registered | Self::Reconciled | Self::PathFollowed { .. }
        )
    }
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
pub(super) fn reconcile_known_path(
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
pub(super) fn register_new_path(
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

#[cfg(test)]
mod process_outcome_tests {
    use super::*;

    #[test]
    fn only_outcomes_that_write_to_the_catalog_count_as_changes() {
        let changed = [
            ProcessOutcome::Registered,
            ProcessOutcome::Reconciled,
            ProcessOutcome::PathFollowed {
                video_id: "v".into(),
            },
        ];
        let unchanged = [
            ProcessOutcome::Unchanged,
            ProcessOutcome::BlockedByCollision {
                video_id: "a".into(),
                colliding_video_id: "b".into(),
            },
            ProcessOutcome::SkippedInvalidName { detected_char: '?' },
            ProcessOutcome::SkippedUnreadable,
        ];
        assert!(changed.iter().all(ProcessOutcome::changed_catalog));
        assert!(!unchanged.iter().any(ProcessOutcome::changed_catalog));
    }
}
