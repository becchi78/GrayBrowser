//! Folder walking + real file I/O + DB glue around `gb_core::scan_pipeline`'s
//! pure validate/build decisions and `gb_core::reconcile`'s pure
//! classify/path-follow decisions.
//!
//! `process_detected_file` is the shared per-file entry point: reused as-is
//! by this module's `scan_folders` (manual full walk), the local realtime
//! watcher, and the NAS diff-scan/poll without any signature change expected
//! across those additions.

use std::fs::File;
use std::path::Path;
use std::time::UNIX_EPOCH;

use gb_core::reconcile::{self, DiscoveredFile, FileClassification, KnownFileMeta};
use gb_core::scan_pipeline::{self, ScannedFile, ValidationOutcome};
use walkdir::WalkDir;

use crate::adapters::long_path;
use crate::db::{queries, Db};
use crate::thumbnail;

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
    /// `reconcile_missing_videos`) and flipped to offline. Videos held back
    /// by `decide_missing_video_ids`'s broken-enumeration guard are *not*
    /// counted here (they're WARN-logged instead, matching NAS polling's
    /// convention) -- this field only reflects confirmed transitions.
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
pub fn process_detected_file(
    db: &Db,
    thumbnails_root: &Path,
    entry_path: &Path,
    file_size: u64,
    mtime: i64,
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
        queries::upsert_skipped_file(&conn, &skipped)
            .inspect_err(|e| log::error!("failed to record skipped file {file_path}: {e}"))?;
        return Ok(ProcessOutcome::SkippedInvalidName { detected_char });
    }

    let known = {
        let conn = db.read_pool.get()?;
        queries::find_video_by_path(&conn, &file_path)?
    };

    match known {
        Some(known) => reconcile_known_path(db, &scanned_file, &known),
        None => register_new_path(db, thumbnails_root, &scanned_file),
    }
}

/// `entry_path` already has a `videos` row (`known`). Reconciles content
/// changes and/or an `offline` -> `online` reconnection at this same path;
/// never inserts a second row.
fn reconcile_known_path(
    db: &Db,
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
            // Known, accepted limitation: matching on mtime+file_size alone
            // means content replaced while its mtime is deliberately
            // preserved (e.g. `robocopy /COPY:DAT`, restoring a backup that
            // keeps original timestamps) is *not* detected -- the DB's
            // quick_hash silently goes stale until something else forces a
            // rehash. This is intentional (recomputing quick_hash on every
            // unchanged file would defeat the whole point of the cheap
            // mtime/size filter), not an oversight. It matters most for
            // path-follow, which matches by quick_hash+file_size: a stale
            // quick_hash here can make a later move of *this* file fail to
            // auto-follow, or in principle match the wrong offline row.
            // Strict same-content verification is deferred to full_hash-based
            // duplicate detection.
            if known.status == "offline" {
                let conn = db.writer.lock().unwrap();
                queries::update_video_status(&conn, &known.id, "online")?;
                Ok(ProcessOutcome::Reconciled)
            } else {
                Ok(ProcessOutcome::Unchanged)
            }
        }
        FileClassification::NeedsRehash => {
            // This is where a collision scenario (経路Y) actually lands, not
            // `register_new_path`'s `BlockedByCollision` (経路X): content
            // matching some *other* offline row (R1) can appear at a path
            // (P2) that's already `known`'s own row (R2, online) here. Since
            // `find_video_by_path` in `process_detected_file` already
            // resolved `known = Some(R2)` before this function was ever
            // called, that's exactly what routes this case here instead of
            // to `register_new_path`'s candidate/collision check. R2's
            // newly-computed quick_hash+file_size is checked against
            // `offline` candidates (the same lookup `register_new_path`
            // uses) before/alongside the metadata write below, so a
            // coincidental match with some other offline row R1 is surfaced
            // as a duplicate candidate via `path_collisions`, instead of
            // leaving R1 offline forever with no trace. R1 itself is not
            // reactivated here -- this function only ever reconciles
            // `known` (R2) in place -- but the pair becomes visible to
            // `dedup::detect_duplicate_groups` for the user to resolve
            // (delete one side, or nothing further happens automatically).
            // See `register_new_path`'s `BlockedByCollision` branch for
            // 経路X (a structurally different case: a *path*, not a content
            // hash, already claimed by another online row).
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

            // Look up any `offline` row whose quick_hash+file_size now
            // coincidentally matches R2's freshly-computed content -- the
            // same query `register_new_path` uses for path-follow, reused
            // here for detection only (no reactivation: `known`/R2 keeps
            // its own path and id, only its stale metadata is refreshed
            // below). Each match is persisted into `path_collisions` via
            // `record_path_collision`: that table was originally named for
            // 経路X's UNIQUE-collision case (a *path* contested between two
            // rows), but its actual structure -- "an offline candidate and
            // an online row whose content may coincide, pending full_hash
            // confirmation" -- is exactly what a 経路Y coincidental rehash
            // match is too, so reusing it (rather than adding a parallel
            // table) is the right call; `dedup::detect_duplicate_groups`
            // already reads every row in `path_collisions` uniformly,
            // regardless of which route produced it.
            match queries::find_offline_candidates_by_quick_hash_and_size(
                &conn,
                &quick_hash_str,
                scanned_file.file_size as i64,
            ) {
                Ok(candidates) => {
                    // Self-collision guard: `known` (R2) is itself still
                    // `status='offline'` at this point whenever it's being
                    // reconnected from offline in the same pass (the
                    // `update_video_status` call below hasn't run yet), so
                    // if R2's freshly-computed quick_hash+file_size happen
                    // to still equal its own *stale* stored values (content
                    // unchanged, only mtime drifted -- e.g. a backup
                    // restore/reconnect that preserves bytes but not
                    // timestamps), this query matches `known` against
                    // itself. Left unfiltered, that would record a
                    // `path_collisions` row pairing `known.id` with itself,
                    // which `dedup::detect_duplicate_groups` would then
                    // surface as a nonsensical "duplicate of itself" group.
                    // `register_new_path`'s identical query never needs this
                    // filter: it has no `known` row at all (that's what
                    // routes here vs. there), so its own row can never be a
                    // candidate for its own path-follow lookup.
                    for candidate in candidates.iter().filter(|c| c.id != known.id) {
                        if let Err(e) = queries::record_path_collision(
                            &conn,
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

            queries::update_video_scan_metadata(
                &conn,
                &known.id,
                scanned_file.file_size as i64,
                scanned_file.mtime,
                &quick_hash_str,
            )?;
            if known.status == "offline" {
                queries::update_video_status(&conn, &known.id, "online")?;
            }
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

/// `entry_path` has no existing `videos` row for this exact path.
/// Path-follow: matches by quick_hash+file_size against `offline` rows
/// first -- only falls back to a brand-new row when no such candidate
/// exists, or when the candidate's target path collides with an
/// already-`online` row.
fn register_new_path(
    db: &Db,
    thumbnails_root: &Path,
    scanned_file: &ScannedFile,
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
    // When can this actually return `Some` (i.e. when is `BlockedByCollision`
    // a live branch, not dead code)? `process_detected_file`'s own
    // `find_video_by_path` already ruled out any row -- online or offline --
    // at this exact `file_path` *before* `register_new_path` was called. If
    // nothing else could write to the DB in between, that alone would make
    // an online collision here provably impossible (any row at this path
    // would have already been `known`, routing to `reconcile_known_path`
    // instead).
    //
    // But that earlier check runs against `db.read_pool` -- *not* under
    // `db.writer.lock()` -- and `hash_file` above does real disk I/O before
    // this function ever reaches the lock acquired a few lines up. That gap
    // is a genuine window, not a closed one: if another thread (this app
    // genuinely runs NAS-poll and realtime-watch callbacks concurrently)
    // registers a brand-new online row at this *exact* `file_path` during
    // that window, this check -- running after we've since acquired
    // `db.writer.lock()` -- will see it and correctly report a collision.
    // In practice that requires two detection paths to be racing over the
    // identical path, which shouldn't happen for an ordinary single watched
    // folder, but can if the user registers nested/overlapping watch
    // folders (e.g. both `C:\Videos` and `C:\Videos\2024`) that both end up
    // observing the same physical file. So: low-frequency, but a real,
    // reachable race, not merely a defensive stub -- and once we're inside
    // `db.writer.lock()`, the check-then-write below (this call through
    // `decide_path_follow` to whichever branch runs) is atomic, so this
    // holds regardless of how this state was reached (no panic, R1 stays
    // offline, R2 untouched).
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
            match queries::update_video_path_and_status(
                &conn,
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
                    //
                    // `candidates.first()` is exactly the offline row
                    // `decide_path_follow` picked (see its doc comment), so
                    // its `file_path` is this video's pre-rewrite path --
                    // reading it back from the DB again here isn't needed.
                    if let Some(candidate) = candidates.first() {
                        let watch_folders = match queries::get_watch_folders(&conn) {
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
                            &candidate.file_path,
                            &scanned_file.file_path,
                        ) {
                            log::warn!(
                                "failed to fully move cached thumbnails for reactivated \
                                 video_id={video_id} from {} to {}",
                                candidate.file_path,
                                scanned_file.file_path
                            );
                        }
                    }
                    Ok(ProcessOutcome::PathFollowed { video_id })
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
                    Ok(ProcessOutcome::SkippedUnreadable)
                }
            }
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

pub fn scan_folders(
    folders: &[String],
    db: &Db,
    thumbnails_root: &Path,
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
        // representation (see gb_core::reconcile's is_under prefix match).
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

            match process_detected_file(db, thumbnails_root, &entry_path, file_size, mtime)? {
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
            reconcile_missing_videos(db, folder, discovered_paths, inaccessible_dirs)?;
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
/// interference), so it gets the same protection. A folder that's entirely
/// gone (the watch folder itself no longer exists) naturally resolves
/// safely without a separate reachability pre-check: `WalkDir` surfaces
/// that as an error on/under the root, which either lands every known video
/// in `inaccessible_dirs` (quietly excluded, no WARN) or leaves
/// `discovered_paths` empty (the `NothingDiscovered` guard, WARN-logged) --
/// either way, nothing gets marked offline.
fn reconcile_missing_videos(
    db: &Db,
    folder: &str,
    discovered_paths: Vec<String>,
    inaccessible_dirs: Vec<String>,
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
    let decision = reconcile::decide_missing_video_ids(&known_online, &diff);
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
    /// can never actually be registered as a separate row at the same path
    /// R2 already occupies (an insert there is structurally impossible, not
    /// merely undesirable) -- the DB ends up with exactly the original two
    /// rows, unchanged, not three.
    #[test]
    fn register_new_path_leaves_both_rows_untouched_when_the_target_path_collides_with_an_online_row(
    ) {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();

        // R1: an offline candidate for path-follow.
        let old_dir = tempfile::tempdir().unwrap();
        let old_path = old_dir.path().join("old.mp4");
        fs::write(&old_path, b"shared content").unwrap();
        let old_folders = vec![old_dir.path().to_string_lossy().to_string()];
        scan_folders(&old_folders, &db, thumbs_root.path()).unwrap();
        let r1_id: String = {
            let conn = db.writer.lock().unwrap();
            conn.execute("UPDATE videos SET status = 'offline'", [])
                .unwrap();
            conn.query_row("SELECT id FROM videos", [], |r| r.get(0))
                .unwrap()
        };

        // R2: an online row already claiming the path we're about to target.
        let new_dir = tempfile::tempdir().unwrap();
        let new_path = new_dir.path().join("new.mp4");
        fs::write(&new_path, b"different content, a different length").unwrap();
        let new_folders = vec![new_dir.path().to_string_lossy().to_string()];
        scan_folders(&new_folders, &db, thumbs_root.path()).unwrap();
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
        // R1's -- the physical trigger for a path-follow match -- while its
        // path is still R2's already-registered online path.
        fs::write(&new_path, b"shared content").unwrap();
        let metadata = fs::metadata(&new_path).unwrap();
        let scanned_file = ScannedFile {
            file_path: new_path.to_string_lossy().to_string(),
            file_name: "new.mp4".to_string(),
            file_size: metadata.len(),
            mtime: mtime_from_metadata(&metadata).unwrap(),
        };

        let outcome = register_new_path(&db, thumbs_root.path(), &scanned_file).unwrap();
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

    /// The true 経路Y scenario (see `reconcile_known_path`'s `NeedsRehash`
    /// doc comment): R2 is a `known` (already-registered) row whose on-disk
    /// content is overwritten so that its *new* quick_hash+file_size happen
    /// to coincide with an unrelated `offline` row R1's. `reconcile_known_path`
    /// rehashes R2 in place as always (never reactivates R1 -- that's not
    /// this function's job), but must now also persist the pair to
    /// `path_collisions` so `dedup::detect_duplicate_groups` can surface it
    /// as a duplicate candidate, instead of silently leaving R1 offline
    /// forever with no trace of the coincidence.
    #[test]
    fn reconcile_known_path_records_a_path_collision_when_the_rehash_coincidentally_matches_an_offline_row(
    ) {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();

        // R1: an unrelated offline row, content "shared content".
        let old_dir = tempfile::tempdir().unwrap();
        let old_path = old_dir.path().join("old.mp4");
        fs::write(&old_path, b"shared content").unwrap();
        let old_folders = vec![old_dir.path().to_string_lossy().to_string()];
        scan_folders(&old_folders, &db, thumbs_root.path()).unwrap();
        let r1_id: String = {
            let conn = db.writer.lock().unwrap();
            conn.execute("UPDATE videos SET status = 'offline'", [])
                .unwrap();
            conn.query_row("SELECT id FROM videos", [], |r| r.get(0))
                .unwrap()
        };

        // R2: a known, online row -- registered with different content, so
        // its initial quick_hash/file_size do *not* match R1's.
        let new_dir = tempfile::tempdir().unwrap();
        let new_path = new_dir.path().join("new.mp4");
        fs::write(&new_path, b"original R2 content, a different length").unwrap();
        let new_folders = vec![new_dir.path().to_string_lossy().to_string()];
        scan_folders(&new_folders, &db, thumbs_root.path()).unwrap();
        let r2_id: String = {
            let conn = db.writer.lock().unwrap();
            conn.query_row(
                "SELECT id FROM videos WHERE file_path = ?1",
                [new_path.to_string_lossy().to_string()],
                |r| r.get(0),
            )
            .unwrap()
        };

        // Overwrite R2's file in place (same path, still `known`) with bytes
        // identical to R1's -- the physical trigger for both NeedsRehash
        // (file_size differs from R2's own stored metadata) and the
        // coincidental-match check (new quick_hash+file_size equal R1's).
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

        let outcome = reconcile_known_path(&db, &scanned_file, &known).unwrap();
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
        scan_folders(&folders, &db, thumbs_root.path()).unwrap();

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

        let outcome = reconcile_known_path(&db, &scanned_file, &known).unwrap();
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
        scan_folders(&folders, &db, thumbs_root.path()).unwrap();

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

        let outcome = reconcile_known_path(&db, &scanned_file, &known).unwrap();
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
        scan_folders(std::slice::from_ref(&old_folder), &db, thumbs_root.path()).unwrap();
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

        scan_folders(std::slice::from_ref(&new_folder), &db, thumbs_root.path()).unwrap();

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
}
