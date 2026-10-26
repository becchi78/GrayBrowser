//! `.wb` import pipeline: reads every `movie` row via a `WbSourceAdapter`,
//! converts+writes each one through
//! `gb_core::wb_import`/`db::queries::import_wb_video`, then links legacy
//! thumbnails via `gb_core::wb_import::match_thumbnail_files`. Runs on a
//! background thread (`run_wb_import`) so the command layer never blocks on
//! it -- a ~3072-row real library must not freeze the UI.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use gb_core::path_restore::{last_segment, restore_path};
use gb_core::ports::dir_lister::DirLister;
use gb_core::ports::ffmpeg::{CropRect, FfmpegAdapter};
use gb_core::ports::wb_source::WbSourceAdapter;
use gb_core::scan_pipeline::NewVideo;
use gb_core::wb_import::{
    count_clamped_scores, count_source_tags, match_thumbnail_files, wb_row_to_import_candidate,
};
use gb_core::wb_thumbnail_split::{parse_thum_size, split_plan, ThumbLayout};

use crate::adapters::long_path;
use crate::db::{queries, Db};
use crate::events::{CatalogNotifier, WatchReconfigurer, WbImportNotifier, WbImportSummary};
use crate::reachability::ReachabilityRegistry;
use crate::thumbnail::reassign::reassign_thumbnails_after_folders_added;
use crate::thumbnail::worker::THUMBNAILS_PER_VIDEO;
use crate::thumbnail::{self, ThumbnailQueueHandle};
use crate::wb_import::thumbnail_scan;

/// Matches `thumbnail::worker::THUMBNAIL_QUALITY` -- both are ffmpeg's
/// `-quality` for a low-bitrate-leaning WebP catalog thumbnail; there's no
/// reason for legacy-imported thumbnails to look different from freshly
/// generated ones.
const WB_THUMBNAIL_QUALITY: u8 = 55;

/// How often `notifier.notify_progress` fires while importing movie rows --
/// often enough for a responsive progress indicator across the real
/// ~3072-row library, without emitting an event (and its IPC round trip) on
/// every single row.
const PROGRESS_NOTIFY_INTERVAL: u32 = 50;

/// `profile` key holding the legacy thumbnail's frame grid.
const THUM_SIZE_KEY: &str = "thum_size";

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
pub fn run_wb_import<S, F, N, C, W, D>(
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
) where
    S: WbSourceAdapter + 'static,
    F: FfmpegAdapter + 'static,
    N: WbImportNotifier + 'static,
    C: CatalogNotifier + 'static,
    W: WatchReconfigurer + 'static,
    D: DirLister + 'static,
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

/// Resolved per-row scan-metadata used to build each row's `NewVideo`.
struct ResolvedFileState {
    status: &'static str,
    quick_hash: String,
    mtime: i64,
    file_size: u64,
}

/// Determines whether a `.wb` row's `movie_path` exists on this machine
/// right now, and if so, computes its real quick_hash/mtime/file_size so an
/// imported row is immediately scan-consistent instead of waiting for a
/// future full scan to backfill it.
///
/// Every failure mode -- the path doesn't exist, its metadata can't be read,
/// its mtime can't be represented, or hashing it fails (locked/permission
/// error) -- converges on the same `"offline"` placeholder
/// (`quick_hash=""`, `mtime=0`, `file_size=0`). This is a deliberate design
/// choice, not merely "the error path was easiest": that placeholder is
/// inert for the drive-still-attached-at-the-same-path reconnection case,
/// and the moment a real scan/watch/poll rediscovers *this exact path*
/// again, `gb_core::reconcile::classify_discovered_file` sees
/// `mtime=0`/`file_size=0` mismatch the real values and classifies it
/// `NeedsRehash` -- so the placeholder self-heals into a real quick_hash
/// without any dedicated "wb-imported, needs backfill" bookkeeping. The
/// known limitation this accepts: quick_hash-based auto-follow to a *new*
/// path after a drive-letter change cannot work for a row still carrying
/// this placeholder, since it never had a real quick_hash to match against
/// in the first place.
fn resolve_file_state(movie_path: &str) -> ResolvedFileState {
    let offline = || ResolvedFileState {
        status: "offline",
        quick_hash: String::new(),
        mtime: 0,
        file_size: 0,
    };

    let long_movie_path = long_path::to_long_path(Path::new(movie_path));
    let metadata = match fs::metadata(&long_movie_path) {
        Ok(m) => m,
        Err(_) => return offline(),
    };
    let file_size = metadata.len();
    let mtime = match crate::scan::mtime_from_metadata(&metadata) {
        Ok(secs) => secs,
        Err(e) => {
            log::warn!("wb import: {movie_path}: {e}; treating as offline placeholder");
            return offline();
        }
    };
    match crate::scan::hash_file(movie_path, file_size) {
        Ok(hash) => ResolvedFileState {
            status: "online",
            quick_hash: hash.to_string(),
            mtime,
            file_size,
        },
        Err(e) => {
            log::warn!(
                "wb import: {movie_path}: failed to compute quick_hash: {e}; \
                 treating as offline placeholder"
            );
            offline()
        }
    }
}

/// Restores a stored path containing `?` when -- and only when -- that path
/// does not exist as written. Rows that exist, or have no `?`, are left
/// exactly as before (no listing happens for them).
fn restore_missing_path(movie_path: &str, lister: &impl DirLister) -> Option<String> {
    if !movie_path.contains('?') {
        return None;
    }
    if fs::metadata(long_path::to_long_path(Path::new(movie_path))).is_ok() {
        return None;
    }
    restore_path(movie_path, lister)
}

/// Remembers each folder's listing -- including "could not be listed" -- for
/// one import. Listing a stopped NAS share blocks for about 20 seconds, so
/// without this every row under that share would pay the wait again.
struct CachingDirLister<'a, D: DirLister> {
    inner: &'a D,
    listings: Mutex<HashMap<String, Option<Vec<String>>>>,
}

impl<'a, D: DirLister> CachingDirLister<'a, D> {
    fn new(inner: &'a D) -> Self {
        Self {
            inner,
            listings: Mutex::new(HashMap::new()),
        }
    }
}

impl<D: DirLister> DirLister for CachingDirLister<'_, D> {
    fn list_names(&self, dir: &str) -> Option<Vec<String>> {
        // The lock is held across the inner call on purpose: this runs on the
        // import thread only, so there is no contention to avoid, and it
        // keeps two callers from both paying for the same slow listing.
        let mut listings = self.listings.lock().unwrap();
        if let Some(cached) = listings.get(dir) {
            return cached.clone();
        }
        let names = self.inner.list_names(dir);
        listings.insert(dir.to_string(), names.clone());
        names
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
fn import_all<S, F, N, C, W, D>(
    db: &Db,
    paths: &WbImportPaths,
    wb_source: &S,
    ffmpeg: &F,
    notifier: &N,
    catalog_notifier: &C,
    watch: Option<&W>,
    dir_lister: &D,
) where
    S: WbSourceAdapter,
    F: FfmpegAdapter,
    N: WbImportNotifier,
    C: CatalogNotifier,
    W: WatchReconfigurer,
    D: DirLister,
{
    log::info!("wb import started: wb_path={}", paths.wb_path.display());

    let rows = match wb_source.read_movies() {
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
            notifier.notify_failed(&reason);
            return;
        }
    };

    let thumb_layout = read_thumb_layout(wb_source);

    let total = rows.len() as u32;
    let clamped_scores = count_clamped_scores(&rows) as u32;
    let source_tag_count = count_source_tags(&rows) as u32;

    let mut registered = 0u32;
    let mut skipped = 0u32;
    let mut tags_assigned = 0u32;
    let mut ratings_from_star_tags = 0u32;
    // (video_id, thumbnail_hash, file_path) for every row that was actually
    // inserted and carries a thumbnail hash -- the input to
    // match_thumbnail_files below. `file_path` is carried alongside so
    // `link_thumbnails` can resolve each video's own registered-folder
    // thumbnail subdirectory without a second DB round trip. Rows that were
    // skipped (already registered) are deliberately excluded: their
    // thumbnail, if any, was already linked (or not) by whatever earlier
    // import/scan first registered them.
    let mut registered_movies: Vec<(String, String, String)> = Vec::new();
    // Thumbnail hashes of inserted online rows. Their legacy JPGs are not
    // migrated (a fresh thumbnail is generated from the video), but the
    // hashes are needed so those JPGs are not reported as unmatched.
    let mut online_hashes: Vec<String> = Vec::new();
    // `movie_path` of every newly registered online row: the folders to
    // register as watch folders. Skipped rows are excluded (not registered by
    // this run) and so are offline rows, whose folder is typically a detached
    // drive that would only show up as an unreachable watch folder.
    let mut online_movie_paths: Vec<String> = Vec::new();
    // Shared by every row so a parent folder is listed once per import, even
    // when it cannot be listed at all (see `CachingDirLister`).
    let dir_lister = CachingDirLister::new(dir_lister);

    for (idx, row) in rows.iter().enumerate() {
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
                continue;
            }
        };

        // The real path on disk: `movie_path` as stored, or -- when it holds
        // `?` for characters the old WhiteBrowser could not convert and the
        // stored path therefore does not exist -- the path restored from the
        // parent folder's contents. Everything downstream (DB row, watch
        // folders, thumbnail location) must use this one, never the stored
        // `?` form.
        let restored_path = restore_missing_path(&candidate.movie_path, &dir_lister);
        let file_path = restored_path
            .clone()
            .unwrap_or_else(|| candidate.movie_path.clone());
        let file_state = resolve_file_state(&file_path);
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
                    continue;
                }
            }
        };

        match outcome {
            queries::WbImportOutcome::Inserted {
                tags_assigned: row_tags_assigned,
            } => {
                registered += 1;
                tags_assigned += row_tags_assigned as u32;
                if candidate.rating_from_star_tags {
                    ratings_from_star_tags += 1;
                }
                if file_state.status != "offline" {
                    online_movie_paths.push(file_path.clone());
                }
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
                // An already-registered online row still owns its legacy
                // JPG. Without collecting the hash here, a re-import would
                // report that JPG as unmatched (needs manual attention).
                // Offline rows stay out of `registered_movies`: their
                // thumbnail was settled by whichever import first
                // registered them. Duplicate hashes are harmless because
                // matching only tests membership.
                if let Some(hash) = candidate.thumbnail_hash {
                    if file_state.status != "offline" {
                        online_hashes.push(hash);
                    }
                }
            }
        }

        let processed = (idx + 1) as u32;
        if processed.is_multiple_of(PROGRESS_NOTIFY_INTERVAL) || processed == total {
            notifier.notify_progress(processed, total);
        }
    }

    // Must precede `link_thumbnails` and the thumbnail queue: both decide
    // where a thumbnail is stored from the watch folders in force at that
    // moment, so adding the folders first puts new thumbnails straight into
    // their final directory.
    let watch_change = if watch.is_some() {
        register_import_folders(db, &online_movie_paths)
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
    let link_outcome = link_thumbnails(
        &paths.thumbnail_folder,
        &paths.thumbnails_dir,
        &watch_folders,
        &registered_movies,
        &online_hashes,
        ffmpeg,
        thumb_layout.as_ref(),
    );

    // The list view only fetches thumbnail images for rows whose
    // `thumbnail_ready` is set, so slot files on disk alone leave the row
    // stuck on "generating". One lock for the whole batch keeps the writer
    // free for other callers; a single failure is logged and does not
    // change the linked count, since the files really are in place.
    if !link_outcome.linked_video_ids.is_empty() {
        let conn = db.writer.lock().unwrap();
        for video_id in &link_outcome.linked_video_ids {
            if let Err(e) = queries::mark_thumbnail_ready(&conn, video_id) {
                log::warn!("wb import: failed to mark thumbnail ready for video {video_id}: {e}");
            }
        }
    }
    let thumbnails_linked = link_outcome.linked;
    let thumbnails_failed = link_outcome.failed;
    let thumbnails_unmatched = link_outcome.unmatched;
    let thumbnails_skipped_online = link_outcome.skipped_online;

    let summary = WbImportSummary {
        registered,
        skipped,
        clamped_scores,
        tags_assigned,
        source_tag_count,
        ratings_from_star_tags,
        thumbnails_linked,
        thumbnails_failed,
        thumbnails_unmatched,
        thumbnails_skipped_online,
        watch_folders_added,
    };

    log::info!(
        "wb import complete: registered={} skipped={} clamped_scores={} tags_assigned={} \
         source_tag_count={} ratings_from_star_tags={} thumbnails_linked={} \
         thumbnails_failed={} thumbnails_unmatched={} thumbnails_skipped_online={} \
         watch_folders_added={:?}",
        summary.registered,
        summary.skipped,
        summary.clamped_scores,
        summary.tags_assigned,
        summary.source_tag_count,
        summary.ratings_from_star_tags,
        summary.thumbnails_linked,
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

/// The legacy thumbnail frame grid, when the `.wb` states one unambiguously:
/// every skin that sets `thum_size` agrees and the value parses. Anything
/// else (none, conflicting, unreadable) yields `None`, because cutting
/// images by a guessed grid is worse than importing them whole.
fn read_thumb_layout<S: WbSourceAdapter>(wb_source: &S) -> Option<ThumbLayout> {
    let mut values = match wb_source.read_profile_value(THUM_SIZE_KEY) {
        Ok(values) => values,
        Err(e) => {
            log::warn!("wb import: could not read {THUM_SIZE_KEY}: {e}; not splitting thumbnails");
            return None;
        }
    };
    values.sort();
    values.dedup();
    match values.as_slice() {
        [] => None,
        [value] => {
            let layout = parse_thum_size(value);
            if layout.is_none() {
                log::warn!(
                    "wb import: unrecognized {THUM_SIZE_KEY} {value:?}; not splitting thumbnails"
                );
            }
            layout
        }
        _ => {
            log::warn!(
                "wb import: skins disagree on {THUM_SIZE_KEY} ({values:?}); \
                 not splitting thumbnails"
            );
            None
        }
    }
}

/// Outcome of [`register_import_folders`].
struct WatchFolderChange {
    before: Vec<String>,
    after: Vec<String>,
    added: Vec<String>,
}

/// Adds the parent folders of `online_movie_paths` to the stored watch-folder
/// list. `None` when nothing needed adding or the DB could not be updated --
/// registering folders is a convenience, so a failure is logged and the
/// import carries on without it. Read-merge-write happens under one writer
/// lock so a concurrent change to the list is not overwritten.
fn register_import_folders(db: &Db, online_movie_paths: &[String]) -> Option<WatchFolderChange> {
    if online_movie_paths.is_empty() {
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
    let added = gb_core::watch_folders::suggest_watch_folders(online_movie_paths, &before);
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

/// Result of [`link_thumbnails`]. `linked == linked_video_ids.len()`.
#[derive(Debug, Default)]
struct LinkOutcome {
    linked: u32,
    failed: u32,
    unmatched: u32,
    skipped_online: u32,
    /// Videos whose slot files were all written; the caller must flag these
    /// ready in the DB.
    linked_video_ids: Vec<String>,
}

/// Legacy-thumbnail linking pass (`match_thumbnail_files` +
/// `generate_thumbnail_for_video`'s atomic-rename pattern applied to
/// `FfmpegAdapter::convert_image_to_webp` instead of `extract_thumbnail`).
///
/// Converts the legacy JPG once, then duplicates the resulting WebP into
/// all `THUMBNAILS_PER_VIDEO` slots (`{video_id}_0.webp` ..
/// `{video_id}_{THUMBNAILS_PER_VIDEO - 1}.webp`). This is a deliberate
/// quality downgrade for offline videos: the normal worker
/// (`thumbnail::worker::generate_thumbnail_for_video`) extracts 6 distinct
/// frames from the source video, but a `.wb` legacy thumbnail is the only
/// image available when the source video file can't be reached, so there's
/// no way to produce 6 distinct frames -- every slot gets the same image
/// instead of leaving the video without any thumbnail at all (which is what
/// would happen if only slot 0 were written, since `get_thumbnails` /
/// `list_videos_missing_thumbnails` require all
/// `THUMBNAILS_PER_VIDEO` slots to exist).
///
/// When `layout` is given and describes the JPG (exact pixel size, exactly
/// `THUMBNAILS_PER_VIDEO` frames), the JPG is instead cut into its frames and
/// slot `i` receives frame `i`, matching what a normal scan produces. Any
/// other case -- no layout, a size that does not match, a probe failure --
/// falls back to the whole-image behavior above.
///
/// Returns a [`LinkOutcome`], where `linked`/`failed` count videos, not
/// individual slot files, and `unmatched`/`skipped_online` count legacy JPG
/// files. `online_hashes` separates JPGs of online videos (intentionally not
/// migrated) from truly unmatched ones.
///
/// This function only writes files and never touches the DB; the caller
/// marks `LinkOutcome::linked_video_ids` ready afterwards.
///
/// `thumbnails_root` is the `thumbnails/` root, not a per-video directory --
/// `watch_folders` (fetched once by the caller, `import_all`) is used
/// together with each matched video's own `file_path` (carried in
/// `registered_movies`) to resolve its registered-folder subdirectory
/// (`thumbnail::paths::video_thumbnail_dir`) before writing anything into
/// it.
fn link_thumbnails<F: FfmpegAdapter>(
    thumbnail_folder: &Path,
    thumbnails_root: &Path,
    watch_folders: &[String],
    registered_movies: &[(String, String, String)],
    online_hashes: &[String],
    ffmpeg: &F,
    layout: Option<&ThumbLayout>,
) -> LinkOutcome {
    let filenames = match thumbnail_scan::list_filenames(thumbnail_folder) {
        Ok(names) => names,
        Err(e) => {
            log::warn!(
                "wb import: failed to list legacy thumbnail folder {}: {e}; \
                 skipping thumbnail linking entirely",
                thumbnail_folder.display()
            );
            return LinkOutcome::default();
        }
    };

    // `match_thumbnail_files` only needs (video_id, thumbnail_hash) pairs --
    // `file_path` is looked back up per matched video via this map instead
    // of threading a third tuple element through `gb_core::wb_import`.
    let movies_for_matching: Vec<(String, String)> = registered_movies
        .iter()
        .map(|(id, hash, _)| (id.clone(), hash.clone()))
        .collect();
    let file_path_by_id: std::collections::HashMap<&str, &str> = registered_movies
        .iter()
        .map(|(id, _, file_path)| (id.as_str(), file_path.as_str()))
        .collect();

    let plan = match_thumbnail_files(&movies_for_matching, online_hashes, &filenames);
    let thumbnails_unmatched = plan.unmatched_filenames.len() as u32;
    let thumbnails_skipped_online = plan.skipped_online_filenames.len() as u32;
    if !plan.unmatched_filenames.is_empty() {
        log::warn!(
            "wb import: {} legacy thumbnail file(s) matched no imported video: {:?}",
            plan.unmatched_filenames.len(),
            plan.unmatched_filenames
        );
    }

    let mut linked_video_ids: Vec<String> = Vec::new();
    let mut thumbnails_failed = 0u32;
    let mut split_count = 0u32;
    for (video_id, filename) in plan.matched {
        // Defensive only: every `video_id` here came from `movies_for_matching`,
        // which is derived from the very same `registered_movies` this map
        // was built from, so a miss should never actually occur.
        let Some(&file_path) = file_path_by_id.get(video_id.as_str()) else {
            log::warn!(
                "wb import: internal inconsistency -- matched video {video_id} has no known \
                 file_path; skipping its legacy thumbnail link"
            );
            thumbnails_failed += 1;
            continue;
        };
        let video_dir =
            crate::thumbnail::paths::video_thumbnail_dir(thumbnails_root, watch_folders, file_path);
        if let Err(e) = fs::create_dir_all(&video_dir) {
            log::warn!(
                "wb import: failed to create thumbnail directory {} for video {video_id}: {e}",
                video_dir.display()
            );
            thumbnails_failed += 1;
            continue;
        }

        let src_path = thumbnail_folder.join(&filename);

        if let Some(rects) = layout.and_then(|l| split_rects(l, &src_path, &filename, ffmpeg)) {
            match write_split_slots(ffmpeg, &src_path, &video_dir, &video_id, &rects) {
                Ok(()) => {
                    split_count += 1;
                    linked_video_ids.push(video_id);
                }
                Err(e) => {
                    log::warn!(
                        "wb import: failed to split thumbnail for video {video_id} \
                         ({filename}): {e}"
                    );
                    remove_slot_files(&video_dir, &video_id);
                    thumbnails_failed += 1;
                }
            }
            continue;
        }
        // Shared scratch file for the single conversion pass below; distinct
        // from the per-slot `{video_id}_{i}.webp.tmp` names used by
        // `thumbnail::worker` so the two pipelines can never collide on a
        // tmp path.
        let shared_tmp_path = video_dir.join(format!("{video_id}.wb_legacy.webp.tmp"));

        if let Err(e) =
            ffmpeg.convert_image_to_webp(&src_path, &shared_tmp_path, WB_THUMBNAIL_QUALITY)
        {
            log::warn!(
                "wb import: thumbnail conversion failed for video {video_id} \
                 ({filename}): {e}"
            );
            let _ = fs::remove_file(&shared_tmp_path);
            thumbnails_failed += 1;
            continue;
        }

        // Duplicate the converted image into every slot. Fail-fast on the
        // first copy error (same rationale as
        // `thumbnail::worker::generate_thumbnail_for_video`'s slot
        // extraction loop): don't leave a partially populated set of slot
        // files behind, since `get_thumbnails` / `list_videos_missing_thumbnails`
        // treat "all slots present" as the only valid state.
        let mut copy_err: Option<std::io::Error> = None;
        for i in 0..THUMBNAILS_PER_VIDEO {
            let final_path = crate::thumbnail::paths::slot_path(&video_dir, &video_id, i);
            if let Err(e) = fs::copy(&shared_tmp_path, &final_path) {
                copy_err = Some(e);
                break;
            }
        }
        let _ = fs::remove_file(&shared_tmp_path);

        match copy_err {
            None => linked_video_ids.push(video_id),
            Some(e) => {
                log::warn!(
                    "wb import: failed to finalize thumbnail slot(s) for video {video_id} \
                     ({filename}): {e}"
                );
                remove_slot_files(&video_dir, &video_id);
                thumbnails_failed += 1;
            }
        }
    }

    if split_count > 0 {
        log::info!("wb import: split {split_count} legacy thumbnail(s) into frames");
    }

    LinkOutcome {
        linked: linked_video_ids.len() as u32,
        failed: thumbnails_failed,
        unmatched: thumbnails_unmatched,
        skipped_online: thumbnails_skipped_online,
        linked_video_ids,
    }
}

/// Crop rectangles for `src_path`, or `None` (with a log line) when the JPG
/// has to be imported whole.
fn split_rects<F: FfmpegAdapter>(
    layout: &ThumbLayout,
    src_path: &Path,
    filename: &str,
    ffmpeg: &F,
) -> Option<Vec<CropRect>> {
    let metadata = match ffmpeg.probe_metadata(src_path) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("wb import: could not read the size of {filename}: {e}; importing it whole");
            return None;
        }
    };
    let (Some(w), Some(h)) = (metadata.width, metadata.height) else {
        log::warn!("wb import: {filename} reports no size; importing it whole");
        return None;
    };
    let rects = split_plan(layout, w, h, THUMBNAILS_PER_VIDEO as u32);
    if rects.is_none() {
        log::warn!(
            "wb import: {filename} is {w}x{h}, which does not match {THUM_SIZE_KEY} \
             {layout:?}; importing it whole"
        );
    }
    rects
}

/// Writes frame `i` of `src_path` to slot `i`, each through its own
/// `{video_id}_{i}.webp.tmp` and a rename so a slot is never half written.
/// Stops at the first failure; the caller removes whatever was written.
fn write_split_slots<F: FfmpegAdapter>(
    ffmpeg: &F,
    src_path: &Path,
    video_dir: &Path,
    video_id: &str,
    rects: &[CropRect],
) -> Result<(), String> {
    for (i, rect) in rects.iter().enumerate() {
        let tmp_path = video_dir.join(format!("{video_id}_{i}.webp.tmp"));
        let final_path = crate::thumbnail::paths::slot_path(video_dir, video_id, i);
        ffmpeg
            .crop_image_to_webp(src_path, &tmp_path, *rect, WB_THUMBNAIL_QUALITY)
            .map_err(|e| format!("frame {i}: {e}"))?;
        fs::rename(&tmp_path, &final_path).map_err(|e| format!("frame {i}: {e}"))?;
    }
    Ok(())
}

/// Best-effort removal of every slot file (and split scratch file) of a
/// video, so a failure never leaves a partial set behind: readers treat
/// "all slots present" as the only valid state.
fn remove_slot_files(video_dir: &Path, video_id: &str) {
    for i in 0..THUMBNAILS_PER_VIDEO {
        let _ = fs::remove_file(crate::thumbnail::paths::slot_path(video_dir, video_id, i));
        let _ = fs::remove_file(video_dir.join(format!("{video_id}_{i}.webp.tmp")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gb_core::ports::ffmpeg::FfmpegError;
    use gb_core::ports::wb_source::WbMovieRow;
    use gb_core::testing::fake_dir_lister::FakeDirLister;
    use gb_core::testing::fake_ffmpeg::FakeFfmpegAdapter;
    use gb_core::testing::fake_wb_source::FakeWbSourceAdapter;

    use crate::db::test_support::init_temp_db;
    use crate::events::{FakeCatalogNotifier, FakeWatchReconfigurer, FakeWbImportNotifier};

    /// Import without registering watch folders -- the behavior the tests
    /// below that predate the option exercise. Shadows the production
    /// `import_all` brought in by `use super::*`.
    fn import_all<S, F, N, C>(
        db: &Db,
        paths: &WbImportPaths,
        wb_source: &S,
        ffmpeg: &F,
        notifier: &N,
        catalog_notifier: &C,
    ) where
        S: WbSourceAdapter,
        F: FfmpegAdapter,
        N: WbImportNotifier,
        C: CatalogNotifier,
    {
        super::import_all(
            db,
            paths,
            wb_source,
            ffmpeg,
            notifier,
            catalog_notifier,
            None::<&FakeWatchReconfigurer>,
            &FakeDirLister::default(),
        );
    }

    /// The subdirectory `link_thumbnails`/`video_thumbnail_dir` resolve
    /// every video in these tests to, since none of them register any
    /// `watch_folders`.
    fn unassigned_dir(thumbnails_root: &Path) -> PathBuf {
        thumbnails_root.join(gb_core::paths::THUMBNAIL_UNASSIGNED_SUBDIR)
    }

    fn row(overrides: impl FnOnce(&mut WbMovieRow)) -> WbMovieRow {
        let mut row = WbMovieRow {
            movie_id: 1,
            movie_name: "movie.mp4".to_string(),
            movie_path: "T:\\videos\\movie.mp4".to_string(),
            tag: String::new(),
            score: 0,
            hash: String::new(),
            kana: String::new(),
            roma: String::new(),
            file_date: "2011-05-04 12:00:00".to_string(),
            regist_date: "2011-05-04 12:00:00".to_string(),
            last_date: "2011-05-04 12:00:00".to_string(),
        };
        overrides(&mut row);
        row
    }

    fn no_op_ffmpeg() -> FakeFfmpegAdapter {
        FakeFfmpegAdapter {
            convert_result: Box::new(|_src| Ok(())),
            ..Default::default()
        }
    }

    struct Harness {
        _db_dir: tempfile::TempDir,
        db: Db,
        _thumb_folder_dir: tempfile::TempDir,
        _thumbs_out_dir: tempfile::TempDir,
        paths: WbImportPaths,
    }

    fn harness() -> Harness {
        let (db_dir, db) = init_temp_db();
        let thumb_folder_dir = tempfile::tempdir().unwrap();
        let thumbs_out_dir = tempfile::tempdir().unwrap();
        let paths = WbImportPaths {
            wb_path: PathBuf::from("test.wb"),
            thumbnail_folder: thumb_folder_dir.path().to_path_buf(),
            thumbnails_dir: thumbs_out_dir.path().to_path_buf(),
        };
        Harness {
            _db_dir: db_dir,
            db,
            _thumb_folder_dir: thumb_folder_dir,
            _thumbs_out_dir: thumbs_out_dir,
            paths,
        }
    }

    #[test]
    fn imports_online_and_offline_rows_and_reports_expected_counts() {
        let h = harness();

        // Online: a real temp file, movie_path points at it.
        let online_file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(online_file.path(), b"hello wb import").unwrap();
        let online_row = row(|r| {
            r.movie_id = 1;
            r.movie_name = "online.mp4".to_string();
            r.movie_path = online_file.path().to_string_lossy().to_string();
            r.score = 3;
        });

        // Offline: a path that does not exist on this machine.
        let offline_row = row(|r| {
            r.movie_id = 2;
            r.movie_name = "offline.mp4".to_string();
            r.movie_path = "Z:\\this\\path\\does\\not\\exist\\offline.mp4".to_string();
            r.score = 12; // clamp-triggering
        });

        // Invalid datetime: must be skipped without aborting the batch.
        let invalid_row = row(|r| {
            r.movie_id = 3;
            r.movie_name = "invalid.mp4".to_string();
            r.movie_path = "Z:\\invalid.mp4".to_string();
            r.regist_date = "not-a-date".to_string();
        });

        let wb_source = FakeWbSourceAdapter {
            movies: Ok(vec![online_row, offline_row, invalid_row]),
            ..Default::default()
        };
        let ffmpeg = no_op_ffmpeg();
        let notifier = FakeWbImportNotifier::default();
        let catalog_notifier = FakeCatalogNotifier::default();

        import_all(
            &h.db,
            &h.paths,
            &wb_source,
            &ffmpeg,
            &notifier,
            &catalog_notifier,
        );

        let complete_calls = notifier.complete_calls.lock().unwrap();
        assert_eq!(
            complete_calls.len(),
            1,
            "notify_complete must fire exactly once"
        );
        let summary = &complete_calls[0];

        assert_eq!(
            summary.registered, 2,
            "the online and offline rows both import"
        );
        assert_eq!(summary.skipped, 0);
        assert_eq!(summary.clamped_scores, 1, "only the score=12 row clamps");
        assert_eq!(catalog_notifier.calls(), 1);

        // The offline row must land as the documented placeholder.
        let conn = h.db.writer.lock().unwrap();
        let (status, quick_hash, mtime, file_size): (String, String, i64, i64) = conn
            .query_row(
                "SELECT status, quick_hash, mtime, file_size FROM videos \
                 WHERE file_path = 'Z:\\this\\path\\does\\not\\exist\\offline.mp4'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(status, "offline");
        assert_eq!(quick_hash, "");
        assert_eq!(mtime, 0);
        assert_eq!(file_size, 0);

        // The online row must have a real quick_hash/mtime/file_size.
        let (status, quick_hash, file_size): (String, String, i64) = conn
            .query_row(
                "SELECT status, quick_hash, file_size FROM videos WHERE file_name = 'online.mp4'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "online");
        assert_ne!(quick_hash, "", "an online row must get a real quick_hash");
        assert_eq!(file_size, "hello wb import".len() as i64);

        // The invalid-datetime row must never have reached the DB at all.
        let invalid_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM videos WHERE file_path = 'Z:\\invalid.mp4'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(invalid_count, 0);
    }

    /// `source_tag_count` must reflect the raw `.wb` source data
    /// (`gb_core::wb_import::count_source_tags`) independent of
    /// `tags_assigned` -- summing every row's tags regardless of whether
    /// that row's video was inserted (and its tags actually written) or
    /// skipped (already registered, tags left untouched).
    #[test]
    fn source_tag_count_reflects_every_rows_raw_tag_data_not_just_assigned_ones() {
        let h = harness();
        let wb_source = FakeWbSourceAdapter {
            movies: Ok(vec![
                row(|r| {
                    r.movie_id = 1;
                    r.movie_path = "Z:\\tagged.mp4".to_string();
                    r.tag = "foo\nbar".to_string();
                }),
                row(|r| {
                    r.movie_id = 2;
                    r.movie_path = "Z:\\untagged.mp4".to_string();
                    r.tag = String::new();
                }),
            ]),
            ..Default::default()
        };
        let ffmpeg = no_op_ffmpeg();
        let notifier = FakeWbImportNotifier::default();
        let catalog_notifier = FakeCatalogNotifier::default();

        import_all(
            &h.db,
            &h.paths,
            &wb_source,
            &ffmpeg,
            &notifier,
            &catalog_notifier,
        );

        let summary = notifier.complete_calls.lock().unwrap()[0].clone();
        assert_eq!(summary.tags_assigned, 2);
        assert_eq!(summary.source_tag_count, 2);
    }

    /// The "every row already registered" edge case: a repeat import writes
    /// no new `video_tags` (`tags_assigned == 0`), but `source_tag_count`
    /// still reports the source data's real tag count -- proving
    /// `tags_assigned == 0` here is *not* "the source had no tags", which is
    /// exactly the ambiguity `source_tag_count` exists to resolve for the
    /// frontend.
    #[test]
    fn source_tag_count_stays_nonzero_even_when_a_repeat_import_assigns_no_new_tags() {
        let h = harness();
        let wb_source = FakeWbSourceAdapter {
            movies: Ok(vec![row(|r| {
                r.movie_path = "Z:\\repeat_tagged.mp4".to_string();
                r.tag = "foo".to_string();
            })]),
            ..Default::default()
        };
        let ffmpeg = no_op_ffmpeg();

        import_all(
            &h.db,
            &h.paths,
            &wb_source,
            &ffmpeg,
            &FakeWbImportNotifier::default(),
            &FakeCatalogNotifier::default(),
        );

        let notifier_second = FakeWbImportNotifier::default();
        import_all(
            &h.db,
            &h.paths,
            &wb_source,
            &ffmpeg,
            &notifier_second,
            &FakeCatalogNotifier::default(),
        );
        let second_summary = notifier_second.complete_calls.lock().unwrap()[0].clone();
        assert_eq!(
            second_summary.tags_assigned, 0,
            "the repeat run's already-registered row writes no new video_tags"
        );
        assert_eq!(
            second_summary.source_tag_count, 1,
            "source_tag_count reflects the raw .wb data, not what got (re-)written"
        );
    }

    /// Star-only tags become a rating (and never a tag); mixed/hollow-star
    /// tags stay tags. A repeat import (all rows Skipped) counts nothing.
    #[test]
    fn star_only_tags_become_ratings_and_are_counted_for_inserted_rows_only() {
        let h = harness();
        let wb_source = FakeWbSourceAdapter {
            movies: Ok(vec![
                row(|r| {
                    r.movie_id = 1;
                    r.movie_path = "Z:\\star3.mp4".to_string();
                    r.tag = "★★★\nfoo".to_string();
                    r.score = 1;
                }),
                row(|r| {
                    r.movie_id = 2;
                    r.movie_path = "Z:\\mixed.mp4".to_string();
                    r.tag = "☆\n★お気に入り".to_string();
                    r.score = 4;
                }),
                row(|r| {
                    r.movie_id = 3;
                    r.movie_path = "Z:\\star_only.mp4".to_string();
                    r.tag = "★★★★★★★".to_string();
                    r.score = 0;
                }),
            ]),
            ..Default::default()
        };
        let ffmpeg = no_op_ffmpeg();
        let notifier = FakeWbImportNotifier::default();

        import_all(
            &h.db,
            &h.paths,
            &wb_source,
            &ffmpeg,
            &notifier,
            &FakeCatalogNotifier::default(),
        );

        let summary = notifier.complete_calls.lock().unwrap()[0].clone();
        assert_eq!(summary.registered, 3);
        assert_eq!(summary.ratings_from_star_tags, 2);
        // foo + ☆ + ★お気に入り; the star-only tags are not source tags.
        assert_eq!(summary.source_tag_count, 3);
        assert_eq!(summary.tags_assigned, 3);

        {
            let conn = h.db.writer.lock().unwrap();
            let rating_of = |name: &str| -> i64 {
                conn.query_row(
                    "SELECT rating FROM videos WHERE file_path = ?1",
                    [name],
                    |r| r.get(0),
                )
                .unwrap()
            };
            assert_eq!(rating_of(r"Z:\star3.mp4"), 3, "star tag beats score=1");
            assert_eq!(
                rating_of(r"Z:\mixed.mp4"),
                4,
                "no star-only tag: score wins"
            );
            assert_eq!(rating_of(r"Z:\star_only.mp4"), 5, "7 stars cap at 5");

            let star_tags: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM tags WHERE name LIKE '★%' AND name NOT LIKE '%お%'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(star_tags, 0, "star-only tags are not registered as tags");
            let tag_names: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM tags WHERE name IN ('foo', '☆', '★お気に入り')",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(tag_names, 3);
        }

        let notifier_second = FakeWbImportNotifier::default();
        import_all(
            &h.db,
            &h.paths,
            &wb_source,
            &ffmpeg,
            &notifier_second,
            &FakeCatalogNotifier::default(),
        );
        let second = notifier_second.complete_calls.lock().unwrap()[0].clone();
        assert_eq!(second.skipped, 3);
        assert_eq!(second.ratings_from_star_tags, 0);
    }

    #[test]
    fn a_repeat_import_of_the_same_source_is_fully_skipped() {
        let h = harness();
        let wb_source = FakeWbSourceAdapter {
            movies: Ok(vec![row(|r| {
                r.movie_path = "Z:\\repeat.mp4".to_string();
            })]),
            ..Default::default()
        };
        let ffmpeg = no_op_ffmpeg();

        let notifier_first = FakeWbImportNotifier::default();
        let catalog_notifier_first = FakeCatalogNotifier::default();
        import_all(
            &h.db,
            &h.paths,
            &wb_source,
            &ffmpeg,
            &notifier_first,
            &catalog_notifier_first,
        );
        assert_eq!(
            notifier_first.complete_calls.lock().unwrap()[0].registered,
            1
        );
        assert_eq!(notifier_first.complete_calls.lock().unwrap()[0].skipped, 0);

        let notifier_second = FakeWbImportNotifier::default();
        let catalog_notifier_second = FakeCatalogNotifier::default();
        import_all(
            &h.db,
            &h.paths,
            &wb_source,
            &ffmpeg,
            &notifier_second,
            &catalog_notifier_second,
        );
        let second_summary = notifier_second.complete_calls.lock().unwrap()[0].clone();
        assert_eq!(
            second_summary.registered, 0,
            "the second run must register nothing new"
        );
        assert_eq!(
            second_summary.skipped, 1,
            "the second run must skip the already-imported row"
        );

        let conn = h.db.writer.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 1,
            "no duplicate row must be created by the repeat import"
        );
    }

    #[test]
    fn links_matching_thumbnails_and_counts_unmatched_and_failed() {
        let h = harness();

        let matched_row = row(|r| {
            r.movie_id = 1;
            r.movie_path = "Z:\\matched.mp4".to_string();
            r.hash = "1e5e0fbf".to_string();
        });
        let failing_row = row(|r| {
            r.movie_id = 2;
            r.movie_path = "Z:\\failing.mp4".to_string();
            r.hash = "aaaaaaaa".to_string();
        });
        let wb_source = FakeWbSourceAdapter {
            movies: Ok(vec![matched_row, failing_row]),
            ..Default::default()
        };

        std::fs::write(
            h.paths.thumbnail_folder.join("MyVideo.mp4.#1e5e0fbf.jpg"),
            b"fake jpg",
        )
        .unwrap();
        std::fs::write(
            h.paths.thumbnail_folder.join("MyVideo.mp4.#aaaaaaaa.jpg"),
            b"fake jpg",
        )
        .unwrap();
        std::fs::write(
            h.paths.thumbnail_folder.join("no_match_at_all.jpg"),
            b"fake jpg",
        )
        .unwrap();

        let ffmpeg = FakeFfmpegAdapter {
            convert_result: Box::new(|src_path| {
                if src_path.to_string_lossy().contains("aaaaaaaa") {
                    Err(FfmpegError::NonZeroExit {
                        status: 1,
                        stderr: "simulated conversion failure".into(),
                    })
                } else {
                    Ok(())
                }
            }),
            ..Default::default()
        };
        let notifier = FakeWbImportNotifier::default();
        let catalog_notifier = FakeCatalogNotifier::default();

        import_all(
            &h.db,
            &h.paths,
            &wb_source,
            &ffmpeg,
            &notifier,
            &catalog_notifier,
        );

        let summary = notifier.complete_calls.lock().unwrap()[0].clone();
        assert_eq!(summary.registered, 2);
        assert_eq!(
            summary.thumbnails_linked, 1,
            "only the matching, succeeding hash links"
        );
        assert_eq!(
            summary.thumbnails_failed, 1,
            "the failing conversion must be counted"
        );
        assert_eq!(
            summary.thumbnails_unmatched, 1,
            "the filename with no matching hash must be counted"
        );

        let conn = h.db.writer.lock().unwrap();
        let matched_id: String = conn
            .query_row(
                "SELECT id FROM videos WHERE file_path = 'Z:\\matched.mp4'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let video_dir = unassigned_dir(&h.paths.thumbnails_dir);
        for i in 0..THUMBNAILS_PER_VIDEO {
            assert!(
                video_dir.join(format!("{matched_id}_{i}.webp")).exists(),
                "slot {i} must exist for the matched video"
            );
        }
        assert!(!video_dir
            .join(format!("{matched_id}.wb_legacy.webp.tmp"))
            .exists());

        let failing_id: String = conn
            .query_row(
                "SELECT id FROM videos WHERE file_path = 'Z:\\failing.mp4'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        for i in 0..THUMBNAILS_PER_VIDEO {
            assert!(
                !video_dir.join(format!("{failing_id}_{i}.webp")).exists(),
                "slot {i} must not exist for the failing video"
            );
        }
        assert!(!video_dir
            .join(format!("{failing_id}.wb_legacy.webp.tmp"))
            .exists());

        let ready = |id: &str| -> i64 {
            conn.query_row(
                "SELECT thumbnail_ready FROM videos WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(
            ready(&matched_id),
            1,
            "a linked video must be flagged ready"
        );
        assert_eq!(
            ready(&failing_id),
            0,
            "a conversion failure must leave thumbnail_ready unset"
        );
    }

    /// Only an offline row whose legacy JPG was linked gets `thumbnail_ready`;
    /// online rows (thumbnail comes from the real video) and offline rows with
    /// no JPG stay 0. A re-import must not disturb the already-set flag.
    #[test]
    fn only_linked_offline_rows_become_thumbnail_ready_and_reimport_keeps_it() {
        let h = harness();

        let online_file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(online_file.path(), b"hello ready flag").unwrap();
        let wb_source = FakeWbSourceAdapter {
            movies: Ok(vec![
                row(|r| {
                    r.movie_id = 1;
                    r.movie_path = online_file.path().to_string_lossy().to_string();
                    r.hash = "1e5e0fbf".to_string();
                }),
                row(|r| {
                    r.movie_id = 2;
                    r.movie_path = "Z:\\linked.mp4".to_string();
                    r.hash = "2f6f1cc0".to_string();
                }),
                row(|r| {
                    r.movie_id = 3;
                    r.movie_path = "Z:\\no_jpg.mp4".to_string();
                    r.hash = "3a7a2dd1".to_string();
                }),
            ]),
            ..Default::default()
        };
        for name in ["On.mp4.#1e5e0fbf.jpg", "Off.mp4.#2f6f1cc0.jpg"] {
            std::fs::write(h.paths.thumbnail_folder.join(name), b"fake jpg").unwrap();
        }
        let ffmpeg = no_op_ffmpeg();

        let ready_of = |path_like: &str| -> i64 {
            let conn = h.db.writer.lock().unwrap();
            conn.query_row(
                "SELECT thumbnail_ready FROM videos WHERE file_path LIKE ?1",
                [path_like],
                |r| r.get(0),
            )
            .unwrap()
        };

        for pass in 0..2 {
            import_all(
                &h.db,
                &h.paths,
                &wb_source,
                &ffmpeg,
                &FakeWbImportNotifier::default(),
                &FakeCatalogNotifier::default(),
            );
            assert_eq!(ready_of("Z:\\linked.mp4"), 1, "pass {pass}");
            assert_eq!(ready_of("Z:\\no_jpg.mp4"), 0, "pass {pass}");
            assert_eq!(
                ready_of(&online_file.path().to_string_lossy()),
                0,
                "pass {pass}: online rows are left to the thumbnail worker"
            );
        }
    }

    /// Follow-up to the test above: that test only exercises
    /// `ffmpeg.convert_image_to_webp` itself failing (the early-return
    /// branch before the copy loop). This test instead exercises the copy
    /// loop's own fail-fast + best-effort-cleanup branch: conversion
    /// succeeds, but `fs::copy` fails partway through duplicating the
    /// converted image into the `THUMBNAILS_PER_VIDEO` slots.
    ///
    /// Calls `link_thumbnails` directly rather than going through
    /// `import_all`: this function never touches the DB, so a hand-picked
    /// `video_id` can be used to pre-create a directory at slot 2's
    /// destination path ahead of time -- `fs::copy` always fails when its
    /// destination is an existing directory, which is what forces the copy
    /// loop to fail specifically at slot 2, after slots 0 and 1 have already
    /// been written for real.
    #[test]
    fn copy_failure_partway_through_the_slot_loop_cleans_up_and_stops() {
        let h = harness();
        let video_id = "fixed-video-id-for-copy-failure-test";
        let video_file_path = "Z:\\video-for-copy-failure-test.mp4";
        let video_dir = unassigned_dir(&h.paths.thumbnails_dir);
        std::fs::create_dir_all(&video_dir).unwrap();

        std::fs::write(
            h.paths.thumbnail_folder.join("MyVideo.mp4.#deadbeef.jpg"),
            b"fake jpg",
        )
        .unwrap();

        // Pre-create a directory at slot 2's destination so `fs::copy` fails
        // there specifically. Slots 0 and 1 must succeed first (their
        // destinations are untouched), proving the loop really got partway
        // through the six slots before hitting the failure.
        std::fs::create_dir(video_dir.join(format!("{video_id}_2.webp"))).unwrap();

        // Fail-fast side channel for slot 3 (the slot immediately after the
        // failure, so the very first one a broken/missing `break` would
        // reach): the best-effort cleanup pass below unconditionally
        // `fs::remove_file`s every slot 0..THUMBNAILS_PER_VIDEO regardless of
        // which slot actually failed, so merely asserting "slot 3's path
        // doesn't exist afterward" is true whether or not slot 3 was ever
        // copied to -- cleanup would delete it either way, making that
        // assertion alone unable to catch a missing `break`. To observe the
        // copy attempt itself (not just its cleaned-up-or-not-cleaned-up
        // aftermath), hard-link slot 3's destination to a marker file that
        // lives outside the six slot paths cleanup ever touches:
        // `fs::copy`'s Windows implementation overwrites an existing
        // destination in place (verified empirically: it does NOT
        // delete-and-recreate, so a hard link to that file sees the new
        // content too), while `fs::remove_file` on the slot path afterward
        // only detaches that one directory entry -- the marker keeps
        // whatever content the copy left behind, cleaned up or not.
        let marker_path = video_dir.join("fail_fast_side_channel_marker.bin");
        std::fs::write(&marker_path, b"untouched-original-marker-content").unwrap();
        std::fs::hard_link(&marker_path, video_dir.join(format!("{video_id}_3.webp"))).unwrap();

        let ffmpeg = no_op_ffmpeg();

        let outcome = link_thumbnails(
            &h.paths.thumbnail_folder,
            &h.paths.thumbnails_dir,
            &[],
            &[(
                video_id.to_string(),
                "deadbeef".to_string(),
                video_file_path.to_string(),
            )],
            &[],
            &ffmpeg,
            None,
        );

        assert_eq!(outcome.skipped_online, 0);
        assert_eq!(outcome.linked, 0, "a copy failure must not count as linked");
        assert!(
            outcome.linked_video_ids.is_empty(),
            "a copy failure must not be reported as linked, or its thumbnail_ready would be set"
        );
        assert_eq!(
            outcome.failed, 1,
            "a copy failure partway through the slot loop must still count as failed"
        );
        assert_eq!(outcome.unmatched, 0);

        // Slots 0 and 1 were written successfully, then must have been
        // cleaned back up by the best-effort cleanup pass.
        for i in 0..2 {
            assert!(
                !video_dir.join(format!("{video_id}_{i}.webp")).exists(),
                "slot {i} must have been cleaned up after the later copy failure at slot 2"
            );
        }

        // Slot 2's destination is still present, but only because it is the
        // directory the test pre-created as the failure trigger: cleanup
        // uses `fs::remove_file`, which cannot remove a directory, so
        // best-effort cleanup leaves it untouched by design (not a bug --
        // the test's own `tempfile::TempDir` removes it on drop).
        assert!(video_dir.join(format!("{video_id}_2.webp")).is_dir());

        // Slots 3..THUMBNAILS_PER_VIDEO's own destination paths must be gone
        // too (removed by the same best-effort cleanup pass as slots 0/1 --
        // this alone doesn't prove they were *never attempted*, see above).
        for i in 3..THUMBNAILS_PER_VIDEO {
            assert!(
                !video_dir.join(format!("{video_id}_{i}.webp")).exists(),
                "slot {i}'s path must not remain after cleanup"
            );
        }

        // The actual fail-fast proof: the marker file hard-linked to slot 3
        // must still read back its original content. If the missing `break`
        // regression were reintroduced, the loop would reach slot 3 after
        // slot 2's failure, `fs::copy` would overwrite the marker's content
        // in place, and this would read back the copy's ("fake webp bytes")
        // content instead -- even though slot 3's own path gets cleaned up
        // either way (checked above) and can't tell the two cases apart by
        // itself.
        assert_eq!(
            std::fs::read(&marker_path).unwrap(),
            b"untouched-original-marker-content",
            "slot 3 must never have been attempted after the slot-2 failure -- the loop must \
             stop immediately, not merely clean up whatever it went on to write"
        );

        // The shared scratch tmp file must not survive the failure either.
        assert!(!video_dir
            .join(format!("{video_id}.wb_legacy.webp.tmp"))
            .exists());
    }

    /// An *online* row's thumbnail must come from the real video file (the
    /// existing background thumbnail pipeline, kicked off by
    /// `run_wb_import` after `import_all` returns -- not exercised by this
    /// test, which calls `import_all` directly), never from the legacy
    /// JPG, even when a hash-matching legacy JPG exists. It must therefore
    /// never reach `ffmpeg.convert_image_to_webp` at all, and its legacy
    /// JPG must be counted as `thumbnails_skipped_online` -- not as
    /// `thumbnails_unmatched` (which means "needs manual attention") and not
    /// as `thumbnails_linked`.
    #[test]
    fn online_rows_are_excluded_from_legacy_thumbnail_matching() {
        let h = harness();

        let online_file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(online_file.path(), b"hello online thumbnail exclusion").unwrap();
        let online_row = row(|r| {
            r.movie_id = 1;
            r.movie_path = online_file.path().to_string_lossy().to_string();
            r.hash = "1e5e0fbf".to_string();
        });

        let wb_source = FakeWbSourceAdapter {
            movies: Ok(vec![online_row]),
            ..Default::default()
        };

        std::fs::write(
            h.paths.thumbnail_folder.join("MyVideo.mp4.#1e5e0fbf.jpg"),
            b"fake jpg",
        )
        .unwrap();

        let ffmpeg = no_op_ffmpeg();
        let notifier = FakeWbImportNotifier::default();
        let catalog_notifier = FakeCatalogNotifier::default();

        import_all(
            &h.db,
            &h.paths,
            &wb_source,
            &ffmpeg,
            &notifier,
            &catalog_notifier,
        );

        let summary = notifier.complete_calls.lock().unwrap()[0].clone();
        assert_eq!(summary.registered, 1);
        assert_eq!(summary.thumbnails_linked, 0);
        assert_eq!(summary.thumbnails_failed, 0);
        assert_eq!(
            summary.thumbnails_unmatched, 0,
            "an online row's legacy JPG is intentionally skipped, not unmatched"
        );
        assert_eq!(summary.thumbnails_skipped_online, 1);

        assert!(
            ffmpeg.calls.lock().unwrap().is_empty(),
            "ffmpeg must never be invoked for an online row's legacy thumbnail"
        );
    }

    /// Re-importing the same `.wb` skips every already-registered row, but an
    /// online row's legacy JPG must still be counted as skipped-online rather
    /// than falling into `thumbnails_unmatched`.
    #[test]
    fn reimport_keeps_online_legacy_thumbnail_out_of_unmatched() {
        let h = harness();

        let online_file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(online_file.path(), b"hello reimport online").unwrap();
        let online_row = row(|r| {
            r.movie_id = 1;
            r.movie_path = online_file.path().to_string_lossy().to_string();
            r.hash = "1e5e0fbf".to_string();
        });
        let wb_source = FakeWbSourceAdapter {
            movies: Ok(vec![online_row]),
            ..Default::default()
        };
        std::fs::write(
            h.paths.thumbnail_folder.join("MyVideo.mp4.#1e5e0fbf.jpg"),
            b"fake jpg",
        )
        .unwrap();

        let ffmpeg = no_op_ffmpeg();
        for pass in 0..2 {
            let notifier = FakeWbImportNotifier::default();
            let catalog_notifier = FakeCatalogNotifier::default();
            import_all(
                &h.db,
                &h.paths,
                &wb_source,
                &ffmpeg,
                &notifier,
                &catalog_notifier,
            );
            let summary = notifier.complete_calls.lock().unwrap()[0].clone();
            assert_eq!(summary.registered, 1 - pass, "pass {pass}");
            assert_eq!(summary.skipped, pass, "pass {pass}");
            assert_eq!(summary.thumbnails_unmatched, 0, "pass {pass}");
            assert_eq!(summary.thumbnails_skipped_online, 1, "pass {pass}");
        }
        assert!(ffmpeg.calls.lock().unwrap().is_empty());
    }

    /// Online, offline and unrelated legacy JPGs in one folder must land in
    /// three separate buckets.
    #[test]
    fn mixed_online_offline_and_unrelated_thumbnails_are_counted_separately() {
        let h = harness();

        let online_file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(online_file.path(), b"hello mixed thumbnails").unwrap();
        let online_row = row(|r| {
            r.movie_id = 1;
            r.movie_path = online_file.path().to_string_lossy().to_string();
            r.hash = "1e5e0fbf".to_string();
        });
        let offline_row = row(|r| {
            r.movie_id = 2;
            r.movie_name = "offline.mp4".to_string();
            r.movie_path = "Z:\\no\\such\\offline.mp4".to_string();
            r.hash = "2f6f1cc0".to_string();
        });

        let wb_source = FakeWbSourceAdapter {
            movies: Ok(vec![online_row, offline_row]),
            ..Default::default()
        };

        for name in [
            "On.mp4.#1e5e0fbf.jpg",
            "Off.mp4.#2f6f1cc0.jpg",
            "Other.mp4.#deadbeef.jpg",
        ] {
            std::fs::write(h.paths.thumbnail_folder.join(name), b"fake jpg").unwrap();
        }

        let ffmpeg = no_op_ffmpeg();
        let notifier = FakeWbImportNotifier::default();
        let catalog_notifier = FakeCatalogNotifier::default();

        import_all(
            &h.db,
            &h.paths,
            &wb_source,
            &ffmpeg,
            &notifier,
            &catalog_notifier,
        );

        let summary = notifier.complete_calls.lock().unwrap()[0].clone();
        assert_eq!(summary.registered, 2);
        assert_eq!(summary.thumbnails_linked, 1);
        assert_eq!(summary.thumbnails_failed, 0);
        assert_eq!(summary.thumbnails_skipped_online, 1);
        assert_eq!(summary.thumbnails_unmatched, 1);
        assert_eq!(
            ffmpeg.calls.lock().unwrap().len(),
            1,
            "only the offline row's JPG is converted"
        );
    }

    #[test]
    fn read_movies_failure_calls_notify_failed_instead_of_notify_complete() {
        let h = harness();
        let wb_source = FakeWbSourceAdapter::default(); // default `movies` is Err
        let ffmpeg = no_op_ffmpeg();
        let notifier = FakeWbImportNotifier::default();
        let catalog_notifier = FakeCatalogNotifier::default();

        import_all(
            &h.db,
            &h.paths,
            &wb_source,
            &ffmpeg,
            &notifier,
            &catalog_notifier,
        );

        assert!(
            notifier.complete_calls.lock().unwrap().is_empty(),
            "no summary exists to report -- the import never got past read_movies"
        );
        assert!(notifier.progress_calls.lock().unwrap().is_empty());
        let failed_calls = notifier.failed_calls.lock().unwrap();
        assert_eq!(
            failed_calls.len(),
            1,
            "notify_failed must fire exactly once so the frontend learns the import died"
        );
        assert!(
            !failed_calls[0].is_empty(),
            "the failure reason should be a non-empty human-readable message"
        );
        assert_eq!(catalog_notifier.calls(), 0);
    }

    // --- watch-folder registration ------------------------------------------

    /// A real directory holding one real (online) video file.
    struct OnlineDir {
        dir: tempfile::TempDir,
    }

    impl OnlineDir {
        fn new(file_name: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            std::fs::write(dir.path().join(file_name), b"video bytes").unwrap();
            Self { dir }
        }

        fn folder(&self) -> String {
            self.dir.path().to_string_lossy().to_string()
        }

        fn file(&self, name: &str) -> String {
            self.dir.path().join(name).to_string_lossy().to_string()
        }
    }

    fn import_registering(
        h: &Harness,
        rows: Vec<WbMovieRow>,
        reconfigurer: &FakeWatchReconfigurer,
    ) -> crate::events::WbImportSummary {
        let wb_source = FakeWbSourceAdapter {
            movies: Ok(rows),
            ..Default::default()
        };
        let notifier = FakeWbImportNotifier::default();
        super::import_all(
            &h.db,
            &h.paths,
            &wb_source,
            &no_op_ffmpeg(),
            &notifier,
            &FakeCatalogNotifier::default(),
            Some(reconfigurer),
            &FakeDirLister::default(),
        );
        let summary = notifier.complete_calls.lock().unwrap()[0].clone();
        summary
    }

    fn stored_watch_folders(h: &Harness) -> Vec<String> {
        let conn = h.db.read_pool.get().unwrap();
        queries::get_watch_folders(&conn).unwrap()
    }

    fn online_row(id: i64, path: String) -> WbMovieRow {
        row(|r| {
            r.movie_id = id;
            r.movie_name = "v.mp4".to_string();
            r.movie_path = path;
        })
    }

    fn video_id_of(h: &Harness, path: &str) -> String {
        h.db.writer
            .lock()
            .unwrap()
            .query_row("SELECT id FROM videos WHERE file_path = ?1", [path], |r| {
                r.get(0)
            })
            .unwrap()
    }

    #[test]
    fn only_parent_folders_of_newly_registered_online_rows_are_added() {
        let h = harness();
        let fresh = OnlineDir::new("a.mp4");
        let known = OnlineDir::new("b.mp4");
        let reconf = FakeWatchReconfigurer::default();

        // The "known" video is registered first (without registering
        // folders), so the second run sees it as Skipped.
        import_all(
            &h.db,
            &h.paths,
            &FakeWbSourceAdapter {
                movies: Ok(vec![online_row(1, known.file("b.mp4"))]),
                ..Default::default()
            },
            &no_op_ffmpeg(),
            &FakeWbImportNotifier::default(),
            &FakeCatalogNotifier::default(),
        );

        let summary = import_registering(
            &h,
            vec![
                online_row(1, known.file("b.mp4")),
                online_row(2, fresh.file("a.mp4")),
                online_row(3, r"Z:\gone\offline.mp4".to_string()),
            ],
            &reconf,
        );

        assert_eq!(summary.registered, 2);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.watch_folders_added, vec![fresh.folder()]);
        assert_eq!(stored_watch_folders(&h), vec![fresh.folder()]);
        assert_eq!(*reconf.calls.lock().unwrap(), vec![vec![fresh.folder()]]);
    }

    #[test]
    fn folders_already_covered_by_a_watch_folder_are_not_added() {
        let h = harness();
        let dir = OnlineDir::new("a.mp4");
        let parent = dir
            .dir
            .path()
            .parent()
            .unwrap()
            .to_string_lossy()
            .to_string();
        {
            let conn = h.db.writer.lock().unwrap();
            queries::set_watch_folders(&conn, std::slice::from_ref(&parent)).unwrap();
        }
        let reconf = FakeWatchReconfigurer::default();

        let summary = import_registering(&h, vec![online_row(1, dir.file("a.mp4"))], &reconf);

        assert!(summary.watch_folders_added.is_empty());
        assert_eq!(stored_watch_folders(&h), vec![parent]);
        assert!(
            reconf.calls.lock().unwrap().is_empty(),
            "nothing was added, so the watchers must not be restarted"
        );
    }

    #[test]
    fn opting_out_adds_nothing() {
        let h = harness();
        let dir = OnlineDir::new("a.mp4");
        let notifier = FakeWbImportNotifier::default();
        import_all(
            &h.db,
            &h.paths,
            &FakeWbSourceAdapter {
                movies: Ok(vec![online_row(1, dir.file("a.mp4"))]),
                ..Default::default()
            },
            &no_op_ffmpeg(),
            &notifier,
            &FakeCatalogNotifier::default(),
        );
        let summary = notifier.complete_calls.lock().unwrap()[0].clone();
        assert_eq!(summary.registered, 1);
        assert!(summary.watch_folders_added.is_empty());
        assert!(stored_watch_folders(&h).is_empty());
    }

    #[test]
    fn reimporting_does_not_add_the_folder_twice_or_reconfigure_again() {
        let h = harness();
        let dir = OnlineDir::new("a.mp4");
        let reconf = FakeWatchReconfigurer::default();
        let rows = || vec![online_row(1, dir.file("a.mp4"))];

        import_registering(&h, rows(), &reconf);
        let second = import_registering(&h, rows(), &reconf);

        assert!(second.watch_folders_added.is_empty());
        assert_eq!(stored_watch_folders(&h), vec![dir.folder()]);
        assert_eq!(reconf.calls.lock().unwrap().len(), 1);
    }

    // --- restoring `?` in stored paths ---------------------------------------

    const DASH: &str = "\u{2014}";

    fn import_with_lister(
        h: &Harness,
        rows: Vec<WbMovieRow>,
        lister: &FakeDirLister,
        reconfigurer: Option<&FakeWatchReconfigurer>,
    ) -> crate::events::WbImportSummary {
        let notifier = FakeWbImportNotifier::default();
        super::import_all(
            &h.db,
            &h.paths,
            &FakeWbSourceAdapter {
                movies: Ok(rows),
                ..Default::default()
            },
            &no_op_ffmpeg(),
            &notifier,
            &FakeCatalogNotifier::default(),
            reconfigurer,
            lister,
        );
        let summary = notifier.complete_calls.lock().unwrap()[0].clone();
        summary
    }

    /// A row as the old WhiteBrowser stored it: the dash replaced by `?` in
    /// both the path and the name.
    fn replaced_row(id: i64, folder: &str, stem: &str) -> WbMovieRow {
        row(|r| {
            r.movie_id = id;
            r.movie_name = format!("{stem}?.mp4");
            r.movie_path = format!("{folder}\\{stem}?.mp4");
        })
    }

    fn video_columns(h: &Harness) -> (String, String, String, String) {
        h.db.writer
            .lock()
            .unwrap()
            .query_row(
                "SELECT file_path, file_name, status, quick_hash FROM videos",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap()
    }

    #[test]
    fn a_question_mark_path_is_restored_and_registered_online_under_the_real_name() {
        let h = harness();
        let real = format!("a{DASH}.mp4");
        let dir = OnlineDir::new(&real);
        let lister = FakeDirLister::default().with_dir(&dir.folder(), &[&real]);

        let summary =
            import_with_lister(&h, vec![replaced_row(1, &dir.folder(), "a")], &lister, None);

        assert_eq!(summary.registered, 1);
        let (file_path, file_name, status, quick_hash) = video_columns(&h);
        assert_eq!(file_path, dir.file(&real));
        assert_eq!(file_name, real, "the name must not keep the `?`");
        assert_eq!(status, "online");
        assert_ne!(quick_hash, "");
    }

    #[test]
    fn several_candidates_leave_the_row_offline_with_the_stored_path() {
        let h = harness();
        let dir = OnlineDir::new("unrelated.mp4");
        let lister =
            FakeDirLister::default().with_dir(&dir.folder(), &[&format!("a{DASH}.mp4"), "a-.mp4"]);
        let row = replaced_row(1, &dir.folder(), "a");
        let stored = row.movie_path.clone();

        let summary = import_with_lister(&h, vec![row], &lister, None);

        assert_eq!(summary.registered, 1);
        let (file_path, file_name, status, quick_hash) = video_columns(&h);
        assert_eq!(file_path, stored);
        assert_eq!(file_name, "a?.mp4");
        assert_eq!(status, "offline");
        assert_eq!(quick_hash, "");
    }

    #[test]
    fn an_unlistable_folder_leaves_the_row_offline() {
        let h = harness();
        let dir = OnlineDir::new(&format!("a{DASH}.mp4"));
        let row = replaced_row(1, &dir.folder(), "a");
        let stored = row.movie_path.clone();

        import_with_lister(&h, vec![row], &FakeDirLister::default(), None);

        let (file_path, _, status, _) = video_columns(&h);
        assert_eq!(file_path, stored);
        assert_eq!(status, "offline");
    }

    #[test]
    fn a_folder_is_listed_once_per_import_however_many_rows_need_it() {
        let h = harness();
        let names: Vec<String> = ["a", "b", "c"]
            .iter()
            .map(|stem| format!("{stem}{DASH}.mp4"))
            .collect();
        let dir = OnlineDir::new(&names[0]);
        for name in &names[1..] {
            std::fs::write(dir.dir.path().join(name), b"video bytes").unwrap();
        }
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let lister = FakeDirLister::default().with_dir(&dir.folder(), &name_refs);

        let summary = import_with_lister(
            &h,
            vec![
                replaced_row(1, &dir.folder(), "a"),
                replaced_row(2, &dir.folder(), "b"),
                replaced_row(3, &dir.folder(), "c"),
            ],
            &lister,
            None,
        );

        assert_eq!(summary.registered, 3);
        assert_eq!(lister.call_count_for(&dir.folder()), 1);
    }

    #[test]
    fn a_folder_that_cannot_be_listed_is_not_retried_for_each_row() {
        let h = harness();
        let lister = FakeDirLister::default();
        let folder = "Z:\\gone";

        import_with_lister(
            &h,
            vec![
                replaced_row(1, folder, "a"),
                replaced_row(2, folder, "b"),
                replaced_row(3, folder, "c"),
            ],
            &lister,
            None,
        );

        assert_eq!(lister.call_count_for(folder), 1);
    }

    #[test]
    fn the_folder_of_a_restored_online_video_becomes_a_watch_folder() {
        let h = harness();
        let real = format!("a{DASH}.mp4");
        let dir = OnlineDir::new(&real);
        let lister = FakeDirLister::default().with_dir(&dir.folder(), &[&real]);
        let reconf = FakeWatchReconfigurer::default();

        let summary = import_with_lister(
            &h,
            vec![replaced_row(1, &dir.folder(), "a")],
            &lister,
            Some(&reconf),
        );

        assert_eq!(summary.watch_folders_added, vec![dir.folder()]);
        assert_eq!(stored_watch_folders(&h), vec![dir.folder()]);
    }

    #[test]
    fn reimporting_skips_the_restored_path() {
        let h = harness();
        let real = format!("a{DASH}.mp4");
        let dir = OnlineDir::new(&real);
        let lister = FakeDirLister::default().with_dir(&dir.folder(), &[&real]);
        let rows = || vec![replaced_row(1, &dir.folder(), "a")];

        import_with_lister(&h, rows(), &lister, None);
        let second = import_with_lister(&h, rows(), &lister, None);

        assert_eq!(second.registered, 0);
        assert_eq!(second.skipped, 1);
        let count: i64 =
            h.db.writer
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
                .unwrap();
        assert_eq!(count, 1);
    }

    /// Registering folders is a convenience: a failed save must neither abort
    /// the import nor be reported as added.
    #[test]
    fn a_failed_watch_folder_save_does_not_stop_the_import() {
        let h = harness();
        let dir = OnlineDir::new("a.mp4");
        {
            let conn = h.db.writer.lock().unwrap();
            conn.execute_batch(
                "CREATE TRIGGER fail_watch_folders BEFORE INSERT ON settings
                 WHEN NEW.key = 'watch_folders'
                 BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
            )
            .unwrap();
        }
        let reconf = FakeWatchReconfigurer::default();

        let summary = import_registering(&h, vec![online_row(1, dir.file("a.mp4"))], &reconf);

        assert_eq!(summary.registered, 1);
        assert!(summary.watch_folders_added.is_empty());
        assert!(stored_watch_folders(&h).is_empty());
        assert!(reconf.calls.lock().unwrap().is_empty());
    }

    /// The legacy thumbnail of an offline video is linked after the folders
    /// are registered, so it must land in the new folder's directory rather
    /// than `_unassigned/` (where it would then be unreadable).
    #[test]
    fn legacy_thumbnails_are_linked_into_the_newly_added_folder() {
        let h = harness();
        let dir = OnlineDir::new("real.mp4");
        let offline_path = dir.file("missing.mp4");
        std::fs::write(
            h.paths.thumbnail_folder.join("Off.mp4.#2f6f1cc0.jpg"),
            b"fake jpg",
        )
        .unwrap();
        let offline = row(|r| {
            r.movie_id = 2;
            r.movie_path = offline_path.clone();
            r.hash = "2f6f1cc0".to_string();
        });

        let summary = import_registering(
            &h,
            vec![online_row(1, dir.file("real.mp4")), offline],
            &FakeWatchReconfigurer::default(),
        );
        assert_eq!(summary.thumbnails_linked, 1);

        let id = video_id_of(&h, &offline_path);
        let dest = crate::thumbnail::paths::video_thumbnail_dir(
            &h.paths.thumbnails_dir,
            &[dir.folder()],
            &offline_path,
        );
        for i in 0..THUMBNAILS_PER_VIDEO {
            assert!(dest.join(format!("{id}_{i}.webp")).exists(), "slot {i}");
        }
        assert!(!unassigned_dir(&h.paths.thumbnails_dir)
            .join(format!("{id}_0.webp"))
            .exists());
    }

    /// A video registered before the folder was added (here an offline one
    /// whose legacy thumbnail was linked by an earlier import) has its files
    /// in `_unassigned/`; adding the folder must carry them along.
    #[test]
    fn thumbnails_of_previously_registered_videos_follow_the_new_folder() {
        let h = harness();
        let dir = OnlineDir::new("real.mp4");
        let offline_path = dir.file("missing.mp4");
        std::fs::write(
            h.paths.thumbnail_folder.join("Off.mp4.#2f6f1cc0.jpg"),
            b"fake jpg",
        )
        .unwrap();
        let offline = || {
            row(|r| {
                r.movie_id = 2;
                r.movie_path = offline_path.clone();
                r.hash = "2f6f1cc0".to_string();
            })
        };

        import_all(
            &h.db,
            &h.paths,
            &FakeWbSourceAdapter {
                movies: Ok(vec![offline()]),
                ..Default::default()
            },
            &no_op_ffmpeg(),
            &FakeWbImportNotifier::default(),
            &FakeCatalogNotifier::default(),
        );
        let id = video_id_of(&h, &offline_path);
        let unassigned = unassigned_dir(&h.paths.thumbnails_dir);
        assert!(unassigned.join(format!("{id}_0.webp")).exists());

        let summary = import_registering(
            &h,
            vec![online_row(1, dir.file("real.mp4")), offline()],
            &FakeWatchReconfigurer::default(),
        );
        assert_eq!(summary.skipped, 1);

        let dest = crate::thumbnail::paths::video_thumbnail_dir(
            &h.paths.thumbnails_dir,
            &[dir.folder()],
            &offline_path,
        );
        for i in 0..THUMBNAILS_PER_VIDEO {
            assert!(dest.join(format!("{id}_{i}.webp")).exists(), "slot {i}");
            assert!(!unassigned.join(format!("{id}_{i}.webp")).exists());
        }
    }

    /// Integration coverage against the committed, anonymized real-shaped
    /// fixture (`tests/fixtures/wb/sample_small.wb`, 50 rows -- see
    /// `tests/fixtures/wb/README.md`). Reads it once via the production
    /// `RealWbSourceAdapter`, then feeds those same rows through
    /// `import_all` twice via a `FakeWbSourceAdapter` (no real `.wb` I/O in
    /// the pipeline itself) to check counts at a realistic scale/shape and
    /// idempotency on a repeat run.
    #[test]
    fn sample_fixture_imports_with_expected_counts_and_is_idempotent_on_rerun() {
        use crate::adapters::wb_source::RealWbSourceAdapter;
        use gb_core::ports::wb_source::WbSourceAdapter as _;

        let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/wb/sample_small.wb");
        let real_adapter = RealWbSourceAdapter::open(&fixture_path)
            .expect("should open the committed sample fixture read-only");
        let rows = real_adapter
            .read_movies()
            .expect("read_movies should succeed against the fixture");
        assert_eq!(
            rows.len(),
            50,
            "sample_small.wb is documented as exactly 50 rows"
        );

        let expected_clamped = count_clamped_scores(&rows) as u32;
        let expected_valid_rows = rows
            .iter()
            .filter(|r| gb_core::wb_import::wb_row_to_import_candidate(r).is_ok())
            .count() as u32;

        let h = harness();
        let wb_source = FakeWbSourceAdapter {
            movies: Ok(rows),
            ..Default::default()
        };
        let ffmpeg = no_op_ffmpeg();

        let notifier_first = FakeWbImportNotifier::default();
        let catalog_notifier_first = FakeCatalogNotifier::default();
        import_all(
            &h.db,
            &h.paths,
            &wb_source,
            &ffmpeg,
            &notifier_first,
            &catalog_notifier_first,
        );
        let first_summary = notifier_first.complete_calls.lock().unwrap()[0].clone();
        assert_eq!(first_summary.clamped_scores, expected_clamped);
        assert_eq!(
            first_summary.registered + first_summary.skipped,
            expected_valid_rows,
            "every row that produced a valid ImportCandidate must be either \
             registered or skipped"
        );
        assert_eq!(
            first_summary.skipped, 0,
            "a fresh DB has nothing to skip yet"
        );

        let notifier_second = FakeWbImportNotifier::default();
        let catalog_notifier_second = FakeCatalogNotifier::default();
        import_all(
            &h.db,
            &h.paths,
            &wb_source,
            &ffmpeg,
            &notifier_second,
            &catalog_notifier_second,
        );
        let second_summary = notifier_second.complete_calls.lock().unwrap()[0].clone();
        assert_eq!(
            second_summary.registered, 0,
            "re-importing the same fixture must register nothing new"
        );
        assert_eq!(second_summary.skipped, expected_valid_rows);
    }

    /// Runs the real pipeline logic against the developer's actual `.wb`
    /// library (`tests/fixtures/wb/local/default_20110504.wb`, gitignored
    /// -- never committed, absent in CI/other machines) to get real-scale
    /// counts. Mirrors `tests/wb_source_local_fixture.rs`'s safety
    /// conventions exactly:
    /// - Skips itself (prints a message, does not fail) when the local
    ///   fixture is absent -- this must never fail a normal `cargo test`.
    /// - Opens the `.wb` strictly read-only via `RealWbSourceAdapter`;
    ///   nothing in this test (or `import_all`) ever writes back to it.
    /// - Only counts are asserted or printed to stderr -- never real paths,
    ///   tags, filenames, or any other cell content.
    ///
    /// The legacy thumbnail folder is deliberately an empty temp directory
    /// (the developer doesn't have the real legacy thumbnail folder at
    /// hand), so this exercises the "3072 real online/offline rows, but no
    /// legacy thumbnails available at all" shape. Every DB write lands in a
    /// throwaway temp DB (`harness()`'s `init_temp_db`), never the real
    /// `app.db` -- this test cannot touch a real user's database.
    #[test]
    fn real_local_fixture_import_reports_realistic_counts() {
        use crate::adapters::wb_source::RealWbSourceAdapter;
        use gb_core::ports::wb_source::WbSourceAdapter as _;

        let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/wb/local/default_20110504.wb");
        if !fixture_path.exists() {
            eprintln!(
                "skipping: local .wb fixture not present at {} (never committed, developer-only)",
                fixture_path.display()
            );
            return;
        }

        let real_adapter =
            RealWbSourceAdapter::open(&fixture_path).expect("should open the real .wb read-only");
        let rows = real_adapter
            .read_movies()
            .expect("read_movies should succeed against the real .wb");
        assert_eq!(rows.len(), 3072, "movie row count");

        let expected_clamped = count_clamped_scores(&rows) as u32;
        assert_eq!(
            expected_clamped, 89,
            "known confirmed value, same as wb_source_local_fixture.rs"
        );
        let expected_valid_rows = rows
            .iter()
            .filter(|r| gb_core::wb_import::wb_row_to_import_candidate(r).is_ok())
            .count() as u32;

        // Fresh temp DB + an empty temp "legacy thumbnail folder" + an empty
        // temp thumbnails_dir -- none of this touches the real app.db or a
        // real legacy thumbnail folder.
        let h = harness();
        let wb_source = FakeWbSourceAdapter {
            movies: Ok(rows),
            ..Default::default()
        };
        let ffmpeg = no_op_ffmpeg();
        let notifier = FakeWbImportNotifier::default();
        let catalog_notifier = FakeCatalogNotifier::default();

        import_all(
            &h.db,
            &h.paths,
            &wb_source,
            &ffmpeg,
            &notifier,
            &catalog_notifier,
        );

        let summary = notifier.complete_calls.lock().unwrap()[0].clone();

        // Counts only -- never content.
        eprintln!(
            "wb import (real local .wb, empty legacy thumbnail folder) summary: \
             registered={} skipped={} clamped_scores={} tags_assigned={} \
             thumbnails_linked={} thumbnails_failed={} thumbnails_unmatched={}",
            summary.registered,
            summary.skipped,
            summary.clamped_scores,
            summary.tags_assigned,
            summary.thumbnails_linked,
            summary.thumbnails_failed,
            summary.thumbnails_unmatched
        );

        assert_eq!(
            summary.registered + summary.skipped,
            expected_valid_rows,
            "every row that produced a valid ImportCandidate must be either \
             registered or skipped -- none silently dropped"
        );
        assert_eq!(
            summary.skipped, 0,
            "a fresh temp DB has nothing to skip yet"
        );
        assert_eq!(summary.clamped_scores, expected_clamped);
        assert_eq!(
            summary.thumbnails_linked, 0,
            "the legacy thumbnail folder is an empty temp dir -- nothing to link"
        );
        assert_eq!(
            summary.thumbnails_failed, 0,
            "no legacy thumbnail files exist to fail converting"
        );
        assert_eq!(
            summary.thumbnails_unmatched, 0,
            "no legacy thumbnail files exist, so none can be unmatched either"
        );
    }

    // ---- legacy thumbnail splitting ----

    use gb_core::ports::ffmpeg::{CropRect, VideoMetadata};
    use gb_core::ports::wb_source::WbSourceError;
    use gb_core::testing::fake_ffmpeg::FakeCall;

    fn image_size(w: u32, h: u32) -> Result<VideoMetadata, FfmpegError> {
        Ok(VideoMetadata {
            width: Some(w),
            height: Some(h),
            ..Default::default()
        })
    }

    /// An ffmpeg fake whose JPG probe returns `size`, and whose convert and
    /// crop calls succeed (crop unless `fail_crop_at_x` names its x).
    fn split_ffmpeg(
        size: Result<VideoMetadata, FfmpegError>,
        fail_crop_at_x: Option<u32>,
    ) -> FakeFfmpegAdapter {
        FakeFfmpegAdapter {
            metadata_for: Some(Box::new(move |_| size.clone())),
            convert_result: Box::new(|_| Ok(())),
            crop_result: Box::new(move |_, crop| {
                if Some(crop.x) == fail_crop_at_x {
                    Err(FfmpegError::NonZeroExit {
                        status: 1,
                        stderr: "simulated crop failure".into(),
                    })
                } else {
                    Ok(())
                }
            }),
            ..Default::default()
        }
    }

    fn crops(ffmpeg: &FakeFfmpegAdapter) -> Vec<CropRect> {
        ffmpeg
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter_map(|c| match c {
                FakeCall::CropImageToWebp { crop, .. } => Some(*crop),
                _ => None,
            })
            .collect()
    }

    fn converts(ffmpeg: &FakeFfmpegAdapter) -> usize {
        ffmpeg
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| matches!(c, FakeCall::ConvertImageToWebp { .. }))
            .count()
    }

    struct SplitRun {
        summary: WbImportSummary,
        video_dir: PathBuf,
        video_id: String,
        ready: i64,
    }

    impl SplitRun {
        fn slot(&self, i: usize) -> PathBuf {
            self.video_dir.join(format!("{}_{i}.webp", self.video_id))
        }

        fn slot_tmp(&self, i: usize) -> PathBuf {
            self.video_dir
                .join(format!("{}_{i}.webp.tmp", self.video_id))
        }
    }

    /// Imports one offline row with a matching legacy JPG.
    fn run_split_import(
        h: &Harness,
        profile_values: Result<Vec<String>, WbSourceError>,
        ffmpeg: &FakeFfmpegAdapter,
    ) -> SplitRun {
        let wb_source = FakeWbSourceAdapter {
            movies: Ok(vec![row(|r| {
                r.movie_path = "Z:\\split.mp4".to_string();
                r.hash = "1e5e0fbf".to_string();
            })]),
            profile_values,
            ..Default::default()
        };
        std::fs::write(
            h.paths.thumbnail_folder.join("MyVideo.mp4.#1e5e0fbf.jpg"),
            b"fake jpg",
        )
        .unwrap();
        let notifier = FakeWbImportNotifier::default();
        import_all(
            &h.db,
            &h.paths,
            &wb_source,
            ffmpeg,
            &notifier,
            &FakeCatalogNotifier::default(),
        );
        let summary = notifier.complete_calls.lock().unwrap()[0].clone();
        let conn = h.db.writer.lock().unwrap();
        let (video_id, ready): (String, i64) = conn
            .query_row(
                "SELECT id, thumbnail_ready FROM videos WHERE file_path = 'Z:\\split.mp4'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        SplitRun {
            summary,
            video_dir: unassigned_dir(&h.paths.thumbnails_dir),
            video_id,
            ready,
        }
    }

    fn six_by_one() -> Result<Vec<String>, WbSourceError> {
        Ok(vec!["200x150x6x1".to_string()])
    }

    #[test]
    fn a_matching_size_splits_the_jpg_into_one_frame_per_slot() {
        let h = harness();
        let ffmpeg = split_ffmpeg(image_size(1200, 150), None);

        let run = run_split_import(&h, six_by_one(), &ffmpeg);

        assert_eq!(run.summary.thumbnails_linked, 1);
        assert_eq!(run.summary.thumbnails_failed, 0);
        assert_eq!(run.ready, 1);
        let rects = crops(&ffmpeg);
        assert_eq!(
            rects.iter().map(|r| r.x).collect::<Vec<_>>(),
            [0, 200, 400, 600, 800, 1000]
        );
        assert!(rects.iter().all(|r| r.y == 0 && r.w == 200 && r.h == 150));
        assert_eq!(converts(&ffmpeg), 0, "the whole-image path must not run");
        for i in 0..THUMBNAILS_PER_VIDEO {
            assert!(run.slot(i).exists(), "slot {i}");
            assert!(!run.slot_tmp(i).exists(), "tmp {i}");
        }
        // Each frame is written through its own slot's scratch path.
        let outs: Vec<PathBuf> = ffmpeg
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter_map(|c| match c {
                FakeCall::CropImageToWebp {
                    output_tmp_path, ..
                } => Some(output_tmp_path.clone()),
                _ => None,
            })
            .collect();
        for (i, out) in outs.iter().enumerate() {
            assert_eq!(out, &run.slot_tmp(i));
        }
    }

    fn assert_imported_whole(run: &SplitRun, ffmpeg: &FakeFfmpegAdapter) {
        assert_eq!(crops(ffmpeg).len(), 0, "no frame may be cut");
        assert_eq!(converts(ffmpeg), 1, "the whole image is converted once");
        assert_eq!(run.summary.thumbnails_linked, 1);
        assert_eq!(run.ready, 1);
        for i in 0..THUMBNAILS_PER_VIDEO {
            assert!(run.slot(i).exists(), "slot {i}");
        }
    }

    #[test]
    fn a_size_mismatch_imports_the_jpg_whole() {
        let h = harness();
        let ffmpeg = split_ffmpeg(image_size(1200, 151), None);
        let run = run_split_import(&h, six_by_one(), &ffmpeg);
        assert_imported_whole(&run, &ffmpeg);
    }

    #[test]
    fn a_failed_size_probe_imports_the_jpg_whole() {
        let h = harness();
        let ffmpeg = split_ffmpeg(
            Err(FfmpegError::NonZeroExit {
                status: 1,
                stderr: "bad jpg".into(),
            }),
            None,
        );
        let run = run_split_import(&h, six_by_one(), &ffmpeg);
        assert_imported_whole(&run, &ffmpeg);
    }

    #[test]
    fn a_probe_without_a_size_imports_the_jpg_whole() {
        let h = harness();
        let ffmpeg = split_ffmpeg(Ok(VideoMetadata::default()), None);
        let run = run_split_import(&h, six_by_one(), &ffmpeg);
        assert_imported_whole(&run, &ffmpeg);
    }

    #[test]
    fn a_missing_thum_size_imports_the_jpg_whole() {
        let h = harness();
        let ffmpeg = split_ffmpeg(image_size(1200, 150), None);
        let run = run_split_import(&h, Ok(vec![]), &ffmpeg);
        assert_imported_whole(&run, &ffmpeg);
    }

    #[test]
    fn an_unreadable_thum_size_imports_the_jpg_whole() {
        let h = harness();
        let ffmpeg = split_ffmpeg(image_size(1200, 150), None);
        let err = WbSourceError::Query("no profile table".into());
        let run = run_split_import(&h, Err(err), &ffmpeg);
        assert_imported_whole(&run, &ffmpeg);
    }

    #[test]
    fn an_unparsable_thum_size_imports_the_jpg_whole() {
        let h = harness();
        let ffmpeg = split_ffmpeg(image_size(1200, 150), None);
        let run = run_split_import(&h, Ok(vec!["wide".to_string()]), &ffmpeg);
        assert_imported_whole(&run, &ffmpeg);
    }

    #[test]
    fn skins_that_disagree_on_thum_size_import_the_jpg_whole() {
        let h = harness();
        let ffmpeg = split_ffmpeg(image_size(1200, 150), None);
        let values = Ok(vec!["200x150x6x1".to_string(), "100x75x6x1".to_string()]);
        let run = run_split_import(&h, values, &ffmpeg);
        assert_imported_whole(&run, &ffmpeg);
    }

    #[test]
    fn skins_that_agree_on_thum_size_still_split() {
        let h = harness();
        let ffmpeg = split_ffmpeg(image_size(1200, 150), None);
        let values = Ok(vec!["200x150x6x1".to_string(), "200x150x6x1".to_string()]);
        let run = run_split_import(&h, values, &ffmpeg);
        assert_eq!(crops(&ffmpeg).len(), 6);
        assert_eq!(run.summary.thumbnails_linked, 1);
    }

    #[test]
    fn a_layout_that_is_not_six_frames_imports_the_jpg_whole() {
        let h = harness();
        let ffmpeg = split_ffmpeg(image_size(800, 150), None);
        let run = run_split_import(&h, Ok(vec!["200x150x4x1".to_string()]), &ffmpeg);
        assert_imported_whole(&run, &ffmpeg);
    }

    #[test]
    fn a_crop_failure_partway_removes_every_slot_and_counts_as_failed() {
        let h = harness();
        // Fails on the fourth frame, after three slots were already written.
        let ffmpeg = split_ffmpeg(image_size(1200, 150), Some(600));

        let run = run_split_import(&h, six_by_one(), &ffmpeg);

        assert_eq!(run.summary.thumbnails_failed, 1);
        assert_eq!(run.summary.thumbnails_linked, 0);
        assert_eq!(run.ready, 0);
        assert_eq!(
            crops(&ffmpeg).len(),
            4,
            "frames after the failure are not attempted"
        );
        assert_eq!(converts(&ffmpeg), 0, "a failed split does not fall back");
        for i in 0..THUMBNAILS_PER_VIDEO {
            assert!(!run.slot(i).exists(), "slot {i}");
            assert!(!run.slot_tmp(i).exists(), "tmp {i}");
        }
    }

    #[test]
    fn a_rename_failure_partway_removes_every_slot() {
        let h = harness();
        let video_id = "fixed-video-id-for-rename-failure-test";
        let video_dir = unassigned_dir(&h.paths.thumbnails_dir);
        std::fs::create_dir_all(&video_dir).unwrap();
        std::fs::write(
            h.paths.thumbnail_folder.join("MyVideo.mp4.#deadbeef.jpg"),
            b"fake jpg",
        )
        .unwrap();
        // A directory at slot 2's destination makes only that rename fail.
        std::fs::create_dir(video_dir.join(format!("{video_id}_2.webp"))).unwrap();
        let ffmpeg = split_ffmpeg(image_size(1200, 150), None);
        let layout = gb_core::wb_thumbnail_split::parse_thum_size("200x150x6x1").unwrap();

        let outcome = link_thumbnails(
            &h.paths.thumbnail_folder,
            &h.paths.thumbnails_dir,
            &[],
            &[(
                video_id.to_string(),
                "deadbeef".to_string(),
                "Z:\\rename-failure.mp4".to_string(),
            )],
            &[],
            &ffmpeg,
            Some(&layout),
        );

        assert_eq!(outcome.failed, 1);
        assert_eq!(outcome.linked, 0);
        assert!(outcome.linked_video_ids.is_empty());
        assert_eq!(
            crops(&ffmpeg).len(),
            3,
            "frames after the failure are not attempted"
        );
        for i in [0, 1, 3, 4, 5] {
            assert!(!video_dir.join(format!("{video_id}_{i}.webp")).exists());
        }
        for i in 0..THUMBNAILS_PER_VIDEO {
            assert!(!video_dir.join(format!("{video_id}_{i}.webp.tmp")).exists());
        }
        assert!(video_dir.join(format!("{video_id}_2.webp")).is_dir());
    }
}
