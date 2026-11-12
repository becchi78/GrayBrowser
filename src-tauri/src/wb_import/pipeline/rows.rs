//! The registering stage of the `.wb` import: each `movie` row in turn is
//! converted, checked and written, and what became of it is tallied for the
//! summary, the import log and the thumbnail linking that follows.

use std::collections::HashSet;

use gb_core::path_restore::last_segment;
use gb_core::ports::dir_lister::DirLister;
use gb_core::ports::wb_source::WbMovieRow;
use gb_core::scan_pipeline::{self, NewVideo, ScannedFile, ValidationOutcome};
use gb_core::wb_import::{count_source_tags, wb_row_to_import_candidate, WbImportStage};
use gb_core::wb_import_log::{Availability, PathRestoration, RowLog, RowOutcome};
use gb_core::wb_skipped_payload::WbSkippedPayload;

use crate::db::{queries, Db};
use crate::events::WbImportNotifier;

use super::file_state::{resolve_file_state, restore_missing_path, CachingDirLister};
use super::notify_row_progress;
use super::share::ShareReachability;

/// What the rows added up to. Every row ends in exactly one of
/// `registered_*`, `skipped`, `skipped_invalid_name` and `failed_rows`, so the
/// summary's counts add up to `total`.
pub(super) struct RowTally {
    pub(super) registered_online: u32,
    pub(super) registered_offline: u32,
    pub(super) failed_rows: u32,
    pub(super) skipped: u32,
    pub(super) skipped_invalid_name: u32,
    pub(super) tags_assigned: u32,
    pub(super) ratings_from_star_tags: u32,
    /// (video_id, thumbnail_hash, file_path) of inserted rows with a thumbnail
    /// hash, the input to match_thumbnail_files. `file_path` lets
    /// `link_thumbnails` find each video's thumbnail subdirectory without
    /// another DB query. Already-registered rows are excluded: whatever first
    /// registered them already handled their thumbnail.
    pub(super) registered_movies: Vec<(String, String, String)>,
    /// Thumbnail hashes of inserted online rows. Their legacy JPGs are not
    /// migrated (a thumbnail is generated from the video), but the hashes keep
    /// those JPGs from being reported as unmatched.
    pub(super) online_hashes: Vec<String>,
    /// Hashes of rows refused for a machine-dependent file name: the JPG has
    /// nothing to attach to, and `link_thumbnails` drops it unreported.
    pub(super) invalid_name_hashes: HashSet<String>,
    /// Hashes of already-registered offline rows (re-import): the JPG was
    /// handled at first registration, so `link_thumbnails` drops it unreported.
    pub(super) already_registered_offline_hashes: HashSet<String>,
    /// Paths of rows newly registered by this run, online or offline, to become
    /// watch folders. Offline rows count so a stopped NAS folder stays
    /// reachable from the sidebar; skipped rows don't, so a re-import adds no
    /// duplicates.
    pub(super) registered_movie_paths: Vec<String>,
    /// One entry per row (the row loop pushes exactly one for every row),
    /// written after linking so each can state what became of its thumbnail.
    pub(super) row_logs: Vec<RowLog>,
}

/// Whether a finished row reports progress. A row that fails on a bad date or
/// a failed write does not, which is why the final count is sent separately.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowProgress {
    Report,
    Silent,
}

/// A row the database accepted, as [`RowTally::record_written`] needs it.
struct WrittenRow {
    id: String,
    file_path: String,
    status: &'static str,
    thumbnail_hash: Option<String>,
    rating_from_star_tags: bool,
}

/// Imports every row in order and ends the stage with the final count.
/// The caller announces the stage, so its start time can be read before.
pub(super) fn register_rows<N: WbImportNotifier, D: DirLister>(
    db: &Db,
    rows: &[WbMovieRow],
    wb_paths: &[String],
    shares: &ShareReachability,
    dir_lister: &D,
    notifier: &N,
    total: u32,
) -> RowTally {
    // Shared by every row so a parent folder is listed once per import, even
    // when it cannot be listed at all (see `CachingDirLister`).
    let dir_lister = CachingDirLister::new(dir_lister);
    let mut tally = RowTally::with_capacity(rows.len());

    let mut reported_rows: Option<u32> = None;
    for (idx, row) in rows.iter().enumerate() {
        let (entry, progress) =
            import_row(db, row, &wb_paths[idx], shares, &dir_lister, &mut tally);
        tally.row_logs.push(entry);
        if progress == RowProgress::Report {
            notify_row_progress(notifier, idx, total, &mut reported_rows);
        }
    }
    // Also covers an empty `.wb` (no row ever reports) so the phase still ends
    // with a count.
    if reported_rows != Some(total) {
        notifier.notify_progress(WbImportStage::Registering, total, Some(total));
    }
    tally
}

/// One row, from its `.wb` values to its database write. The log entry is
/// returned rather than stored so that every row leaves exactly one.
fn import_row<D: DirLister>(
    db: &Db,
    row: &WbMovieRow,
    wb_path: &str,
    shares: &ShareReachability,
    dir_lister: &CachingDirLister<'_, D>,
    tally: &mut RowTally,
) -> (RowLog, RowProgress) {
    let mut entry = RowLog {
        movie_id: row.movie_id,
        name: row.movie_name.clone(),
        wb_path: wb_path.to_string(),
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
            tally.failed_rows += 1;
            entry.outcome = RowOutcome::Failed {
                reason: format!("日付が不正: {e}"),
            };
            return (entry, RowProgress::Silent);
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
    let restored_path = restore_missing_path(&candidate.movie_path, share_reachable, dir_lister);
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
    if let ValidationOutcome::Invalid { detected_char } = scan_pipeline::validate(&scanned_file) {
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
        tally.skipped_invalid_name += 1;
        entry.outcome = RowOutcome::InvalidName { detected_char };
        if let Some(hash) = candidate.thumbnail_hash {
            tally.invalid_name_hashes.insert(hash);
        }
        return (entry, RowProgress::Report);
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
                tally.failed_rows += 1;
                entry.outcome = RowOutcome::Failed {
                    reason: format!("データベースへの書き込みに失敗: {e}"),
                };
                return (entry, RowProgress::Silent);
            }
        }
    };

    tally.record_written(
        outcome,
        WrittenRow {
            id,
            file_path,
            status: file_state.status,
            thumbnail_hash: candidate.thumbnail_hash,
            rating_from_star_tags: candidate.rating_from_star_tags,
        },
        &mut entry,
    );
    (entry, RowProgress::Report)
}

impl RowTally {
    fn with_capacity(rows: usize) -> Self {
        Self {
            registered_online: 0,
            registered_offline: 0,
            failed_rows: 0,
            skipped: 0,
            skipped_invalid_name: 0,
            tags_assigned: 0,
            ratings_from_star_tags: 0,
            registered_movies: Vec::new(),
            online_hashes: Vec::new(),
            invalid_name_hashes: HashSet::new(),
            already_registered_offline_hashes: HashSet::new(),
            registered_movie_paths: Vec::new(),
            row_logs: Vec::with_capacity(rows),
        }
    }

    /// Counts a row the database accepted, and keeps its thumbnail hash in
    /// the set that decides how linking treats its legacy JPG.
    fn record_written(
        &mut self,
        outcome: queries::WbImportOutcome,
        row: WrittenRow,
        entry: &mut RowLog,
    ) {
        match outcome {
            queries::WbImportOutcome::Inserted {
                tags_assigned: row_tags_assigned,
            } => {
                if row.status == "offline" {
                    self.registered_offline += 1;
                } else {
                    self.registered_online += 1;
                }
                self.tags_assigned += row_tags_assigned as u32;
                if row.rating_from_star_tags {
                    self.ratings_from_star_tags += 1;
                }
                self.registered_movie_paths.push(row.file_path.clone());
                entry.outcome = RowOutcome::Registered;
                entry.video_id = Some(row.id.clone());
                // Legacy-JPG thumbnail migration (link_thumbnails, run after
                // every row) is only for offline rows: their source video isn't
                // available to generate a fresh thumbnail from, so the old JPG
                // is the best available image. An online row's video file *is*
                // available, so `run_wb_import` hands it to the existing
                // thumbnail pipeline instead (10%-in frame extraction, the
                // same as a normal scan) once the row loop finishes --
                // preferred over the old low-res JPG whenever the real
                // video can produce a fresh one.
                if let Some(hash) = row.thumbnail_hash {
                    if row.status == "offline" {
                        self.registered_movies.push((row.id, hash, row.file_path));
                    } else {
                        self.online_hashes.push(hash);
                    }
                }
            }
            queries::WbImportOutcome::Skipped => {
                self.skipped += 1;
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
                if let Some(hash) = row.thumbnail_hash {
                    if row.status != "offline" {
                        self.online_hashes.push(hash);
                    } else {
                        self.already_registered_offline_hashes.insert(hash);
                    }
                }
            }
        }
    }
}
