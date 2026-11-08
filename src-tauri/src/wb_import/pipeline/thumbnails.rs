//! Linking legacy `.wb` JPG thumbnails to videos: matching by hash or by
//! name, converting (and, when the frame grid is known, splitting) each JPG
//! into the video's slot files in parallel, and recording what was left
//! unlinked.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread;

use gb_core::image_size::jpeg_dimensions;
use gb_core::ports::ffmpeg::{CropOutput, CropRect, FfmpegAdapter};
use gb_core::ports::wb_source::WbSourceAdapter;
use gb_core::wb_import::{
    classify_unmatched_filename, extract_thumbnail_hash, file_name_from_path,
    match_empty_hash_by_name, match_thumbnail_files, partition_unmatched,
};
use gb_core::wb_thumbnail_split::{parse_thum_size, split_plan, ThumbLayout};

use crate::adapters::long_path;
use crate::db::{queries, Db};
use crate::thumbnail::worker::THUMBNAILS_PER_VIDEO;
use crate::wb_import::thumbnail_scan;

use super::is_progress_tick;

/// Matches `thumbnail::worker::THUMBNAIL_QUALITY` -- both are ffmpeg's
/// `-quality` for a low-bitrate-leaning WebP catalog thumbnail; there's no
/// reason for legacy-imported thumbnails to look different from freshly
/// generated ones.
const WB_THUMBNAIL_QUALITY: u8 = 55;

/// `profile` key holding the legacy thumbnail's frame grid.
const THUM_SIZE_KEY: &str = "thum_size";

/// The legacy thumbnail frame grid, when the `.wb` states one unambiguously:
/// every skin that sets `thum_size` agrees and the value parses. Anything
/// else (none, conflicting, unreadable) yields `None`, because cutting
/// images by a guessed grid is worse than importing them whole.
pub(super) fn read_thumb_layout<S: WbSourceAdapter>(wb_source: &S) -> Option<ThumbLayout> {
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

/// Records the legacy thumbnails this import left unlinked, replacing what an
/// earlier import of the same folder recorded, so they can be linked by hand
/// later. Nothing is changed when the folder could not be listed: its previous
/// record is then still the best knowledge there is. A failure is logged and
/// the import carries on, since the record is a convenience on top of files and
/// registrations that are already in place.
///
/// Returns whether the list now reflects this import. Anything reading the
/// list afterwards must not run when it does not, or it would act on rows
/// left by an earlier import.
pub(super) fn save_unmatched_thumbnails(
    db: &Db,
    thumbnail_folder: &Path,
    link_outcome: &LinkOutcome,
    layout: Option<&ThumbLayout>,
) -> bool {
    if link_outcome.folder_error.is_some() {
        return false;
    }
    let thum_layout = layout.map(ThumbLayout::to_thum_size);
    let rows: Vec<queries::NewUnmatchedThumbnail> = link_outcome
        .unmatched_filenames
        .iter()
        .map(|file_name| queries::NewUnmatchedThumbnail {
            file_name: file_name.clone(),
            kind: classify_unmatched_filename(file_name)
                .as_db_str()
                .to_string(),
            thum_layout: thum_layout.clone(),
        })
        .collect();
    let folder = thumbnail_folder.to_string_lossy();
    let mut conn = db.writer.lock().unwrap();
    if let Err(e) = queries::replace_unmatched_thumbnails(&mut conn, &folder, &rows) {
        log::warn!("wb import: failed to record the unmatched legacy thumbnails: {e}");
        return false;
    }
    true
}

/// Result of [`link_thumbnails`]. `linked == linked_video_ids.len()` and
/// `failed == failed_video_ids.len()`; both count videos, each video in at most
/// one of the two lists.
#[derive(Debug, Default)]
pub(super) struct LinkOutcome {
    pub(super) linked: u32,
    pub(super) failed: u32,
    pub(super) unmatched: u32,
    /// Part of `unmatched` whose file name carries an empty hash
    /// (`unmatched - unmatched_empty_hash` is the rest).
    pub(super) unmatched_empty_hash: u32,
    pub(super) skipped_online: u32,
    /// Videos whose last item wrote all slot files, each once; the caller must
    /// flag these ready in the DB.
    pub(super) linked_video_ids: Vec<String>,
    /// (video id, reason) for every matched video whose last item could not
    /// be written, each video once; `failed == failed_video_ids.len()`.
    pub(super) failed_video_ids: Vec<(String, String)>,
    /// Legacy JPG files that matched no registered or online video; the
    /// names behind `unmatched`.
    pub(super) unmatched_filenames: Vec<String>,
    /// Set when the legacy thumbnail folder could not be listed and linking
    /// was skipped altogether.
    pub(super) folder_error: Option<String>,
}

/// Links legacy `.wb` JPG thumbnails to the matched offline videos by
/// converting each JPG to WebP and writing it into all `THUMBNAILS_PER_VIDEO`
/// slots (`{video_id}_0.webp` .. `{video_id}_{THUMBNAILS_PER_VIDEO - 1}.webp`),
/// atomically (`.tmp` then rename). Only writes files; the caller marks
/// `LinkOutcome::linked_video_ids` ready in the DB afterwards.
///
/// Every slot gets the same image by default because the source video is
/// unreachable, so the 6 distinct frames the normal worker extracts cannot be
/// produced, and `get_thumbnails` / `list_videos_missing_thumbnails` require
/// all slots to exist. When `layout` describes the JPG (exact pixel size,
/// exactly `THUMBNAILS_PER_VIDEO` frames), the JPG is instead cut into its
/// frames and slot `i` receives frame `i`, as a normal scan would; with no
/// layout, a size mismatch or a probe failure it falls back to the default.
///
/// Returns a [`LinkOutcome`]: `linked`/`failed` count videos, while
/// `unmatched`/`skipped_online` count JPG files.
///
/// - `thumbnails_root`: the `thumbnails/` root, not a per-video directory.
///   Each video's subdirectory is resolved from `watch_folders` (fetched once
///   by `import_all`) and its own `file_path` in `registered_movies`
///   (`thumbnail::paths::video_thumbnail_dir`).
/// - `online_hashes`: JPGs of online videos, intentionally not migrated, kept
///   apart from truly unmatched ones.
/// - `invalid_name_hashes`: hashes of rows refused for a machine-dependent
///   file name. Their JPGs are dropped before matching, so they count as
///   neither unmatched nor skipped-online, unless the same hash also belongs
///   to a registered video, which still takes it.
/// - `already_registered_offline_hashes`: hashes of offline rows already in
///   the library (re-import). Dropped the same way, so a repeat import does
///   not report already-linked thumbnails as unmatched; a hash shared with a
///   video registered by this run still goes to that video.
/// - `on_progress(processed, total)`: called once with `(0, matched)` before
///   the first conversion, then at the [`is_progress_tick`] cadence, and
///   finally with `(matched, matched)`. Nothing is reported while listing and
///   matching (the total is unknown) or when the folder cannot be listed.
// One argument over clippy's limit: the hash lists and registered videos are
// independent inputs, and grouping them would only rename the problem.
#[allow(clippy::too_many_arguments)]
pub(super) fn link_thumbnails<F: FfmpegAdapter>(
    thumbnail_folder: &Path,
    thumbnails_root: &Path,
    watch_folders: &[String],
    registered_movies: &[(String, String, String)],
    online_hashes: &[String],
    invalid_name_hashes: &HashSet<String>,
    already_registered_offline_hashes: &HashSet<String>,
    ffmpeg: &F,
    layout: Option<&ThumbLayout>,
    on_progress: impl Fn(u32, u32) + Sync,
) -> LinkOutcome {
    let mut filenames = match thumbnail_scan::list_filenames(thumbnail_folder) {
        Ok(names) => names,
        Err(e) => {
            log::warn!(
                "wb import: failed to list legacy thumbnail folder {}: {e}; \
                 skipping thumbnail linking entirely",
                thumbnail_folder.display()
            );
            return LinkOutcome {
                folder_error: Some(e.to_string()),
                ..LinkOutcome::default()
            };
        }
    };

    if !invalid_name_hashes.is_empty() || !already_registered_offline_hashes.is_empty() {
        filenames.retain(|filename| {
            let Some(hash) = extract_thumbnail_hash(filename) else {
                return true;
            };
            if !invalid_name_hashes.contains(&hash)
                && !already_registered_offline_hashes.contains(&hash)
            {
                return true;
            }
            // Only a hash shared with a registered video needs the scan.
            online_hashes.contains(&hash) || registered_movies.iter().any(|(_, h, _)| *h == hash)
        });
    }

    // `match_thumbnail_files` needs the video's file name to tell apart videos
    // sharing a hash; the full `file_path` is looked back up per matched video
    // via this map for the thumbnail directory.
    let movies_for_matching: Vec<(String, String, String)> = registered_movies
        .iter()
        .map(|(id, hash, file_path)| {
            (
                id.clone(),
                hash.clone(),
                file_name_from_path(file_path).to_string(),
            )
        })
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

    let matched_total = plan.matched.len() as u32;
    on_progress(0, matched_total);
    let context = LinkContext {
        thumbnail_folder,
        thumbnails_root,
        watch_folders,
        layout,
        ffmpeg,
    };
    let results = link_in_parallel(
        &plan.matched,
        matched_total,
        &on_progress,
        |video_id, filename| {
            context.link_one(video_id, filename, file_path_by_id.get(video_id).copied())
        },
    );
    if matched_total > 0 {
        on_progress(matched_total, matched_total);
    }

    // A video can have several items; as in a serial run the last one decides
    // (a later item overwrites the slots of an earlier one). Each video is
    // classified once, in the order its first item appears in `plan.matched`.
    let mut order: Vec<&str> = Vec::new();
    let mut last_result: HashMap<&str, ItemResult> = HashMap::new();
    for ((video_id, _), result) in plan.matched.iter().zip(results) {
        if last_result.insert(video_id.as_str(), result).is_none() {
            order.push(video_id.as_str());
        }
    }
    let mut linked_video_ids: Vec<String> = Vec::new();
    let mut failed_video_ids: Vec<(String, String)> = Vec::new();
    let mut split_count = 0u32;
    for video_id in order {
        match last_result.remove(video_id) {
            Some(ItemResult::Linked { split }) => {
                split_count += u32::from(split);
                linked_video_ids.push(video_id.to_string());
            }
            Some(ItemResult::Failed(reason)) => {
                failed_video_ids.push((video_id.to_string(), reason));
            }
            None => {}
        }
    }

    if split_count > 0 {
        log::info!("wb import: split {split_count} legacy thumbnail(s) into frames");
    }

    LinkOutcome {
        linked: linked_video_ids.len() as u32,
        failed: failed_video_ids.len() as u32,
        unmatched: thumbnails_unmatched,
        unmatched_empty_hash: partition_unmatched(&plan.unmatched_filenames)
            .empty_hash
            .len() as u32,
        skipped_online: thumbnails_skipped_online,
        linked_video_ids,
        failed_video_ids,
        unmatched_filenames: plan.unmatched_filenames,
        folder_error: None,
    }
}

/// Fewest and most legacy thumbnails linked at once.
const LINK_MIN_WORKERS: usize = 2;
const LINK_MAX_WORKERS: usize = 8;

/// How many legacy thumbnails to link at once on a machine with `cores`
/// logical cores. Each one is dominated by short ffmpeg process launches, so
/// several in flight hide the launch latency; but ffmpeg is itself
/// multi-threaded and the disk and antivirus are shared, so only half the
/// cores are used and the count is capped, which also leaves the UI room.
/// Measured on 16 cores: going from 8 to 16 workers gained about 20% while
/// every ffmpeg call got about 60% slower.
fn link_worker_count(cores: usize) -> usize {
    (cores / 2).clamp(LINK_MIN_WORKERS, LINK_MAX_WORKERS)
}

/// What became of one matched thumbnail.
pub(crate) enum ItemResult {
    Linked { split: bool },
    Failed(String),
}

/// Why attaching a recorded thumbnail failed.
pub(crate) struct AttachFailure {
    /// Text meant for the user.
    pub message: String,
    /// The failed attempt cleared the video's `thumbnail_ready` flag, so the
    /// catalog view is stale and the caller should notify.
    pub catalog_changed: bool,
}

/// Attaches the recorded legacy thumbnail `record` to an offline video,
/// replacing any thumbnails it has. Shared by linking by hand and by name, so
/// both leave identical files and rows.
///
/// The source file is the record's folder joined with its name; callers pass a
/// record, never a path. On failure the record stays and no slot or scratch
/// file is left behind.
pub(crate) fn attach_recorded_thumbnail<F: FfmpegAdapter>(
    db: &Db,
    ffmpeg: &F,
    thumbnails_root: &Path,
    watch_folders: &[String],
    record: &queries::UnmatchedThumbnailRow,
    video: &queries::OfflineVideoCandidate,
) -> Result<(), AttachFailure> {
    let video_id = video.id.as_str();
    let fail = |message: String| AttachFailure {
        message,
        catalog_changed: false,
    };
    let db_err = |e: rusqlite::Error| fail(format!("データベースの操作に失敗しました: {e}"));

    let src_path = Path::new(&record.folder_path).join(&record.file_name);
    if fs::metadata(crate::adapters::long_path::to_long_path(&src_path)).is_err() {
        return Err(fail("旧サムネイルのファイルが見つかりません。".to_string()));
    }

    // A stored value that no longer parses is treated as "no layout" (import
    // the image whole), the same as when the .wb stated none.
    let layout = record.thum_layout.as_deref().and_then(parse_thum_size);

    match convert_thumbnail_to_slots(
        ffmpeg,
        thumbnails_root,
        watch_folders,
        video_id,
        &video.file_path,
        &src_path,
        layout.as_ref(),
    ) {
        ItemResult::Linked { .. } => {}
        ItemResult::Failed(reason) => {
            // The failed attempt removed the video's slot files, including
            // any it had before; the flag must not keep claiming them.
            let mut catalog_changed = false;
            if video.thumbnail_ready {
                let conn = db.writer.lock().unwrap();
                if let Err(e) = queries::clear_thumbnail_ready(&conn, video_id) {
                    log::warn!("failed to clear thumbnail_ready of {video_id}: {e}");
                }
                catalog_changed = true;
            }
            return Err(AttachFailure {
                message: format!("紐付けに失敗しました: {reason}"),
                catalog_changed,
            });
        }
    }

    // One transaction: the flag and the record change together, so a failure
    // cannot leave a linked video still listed as unmatched.
    let conn = db.writer.lock().unwrap();
    let tx = conn.unchecked_transaction().map_err(db_err)?;
    queries::mark_thumbnail_ready(&tx, video_id).map_err(db_err)?;
    queries::delete_unmatched_thumbnail(&tx, &record.folder_path, &record.file_name)
        .map_err(db_err)?;
    tx.commit().map_err(db_err)?;
    Ok(())
}

/// What [`link_empty_hash_thumbnails_by_name`] attached.
#[derive(Debug, Default)]
pub(crate) struct NameLinkOutcome {
    /// `(image file name, video file name)` for each thumbnail attached.
    pub linked: Vec<(String, String)>,
}

/// Attaches the recorded empty-hash legacy thumbnails of `folder` whose name
/// equals exactly one offline video's name, when that video has no thumbnails
/// and no other recorded file claims it (`match_empty_hash_by_name`). Runs on
/// the rows already saved, so it never sees a file the list does not hold.
///
/// Best-effort: a failure leaves that row in the list and is only logged, so
/// it can never fail the import.
pub(crate) fn link_empty_hash_thumbnails_by_name<F: FfmpegAdapter>(
    db: &Db,
    ffmpeg: &F,
    thumbnails_root: &Path,
    watch_folders: &[String],
    folder: &Path,
) -> NameLinkOutcome {
    let mut outcome = NameLinkOutcome::default();
    let folder = folder.to_string_lossy();
    let (records, videos) = {
        let conn = match db.read_pool.get() {
            Ok(conn) => conn,
            Err(e) => {
                log::warn!("wb import: cannot link thumbnails by name, no DB connection: {e}");
                return outcome;
            }
        };
        match (
            queries::list_unmatched_thumbnails(&conn),
            queries::list_offline_videos(&conn),
        ) {
            (Ok(records), Ok(videos)) => (records, videos),
            (r, v) => {
                log::warn!(
                    "wb import: cannot link thumbnails by name, read failed: {:?} {:?}",
                    r.err(),
                    v.err()
                );
                return outcome;
            }
        }
    };

    let records: HashMap<String, queries::UnmatchedThumbnailRow> = records
        .into_iter()
        .filter(|r| r.folder_path == folder && r.kind == "empty_hash")
        .map(|r| (r.file_name.clone(), r))
        .collect();
    if records.is_empty() {
        return outcome;
    }
    let mut names: Vec<String> = records.keys().cloned().collect();
    names.sort();
    let keyed: Vec<(String, String, bool)> = videos
        .iter()
        .map(|v| (v.id.clone(), v.file_name.clone(), v.thumbnail_ready))
        .collect();
    let by_id: HashMap<&str, &queries::OfflineVideoCandidate> =
        videos.iter().map(|v| (v.id.as_str(), v)).collect();

    for (video_id, image_name) in match_empty_hash_by_name(&names, &keyed) {
        let (Some(video), Some(record)) = (by_id.get(video_id.as_str()), records.get(&image_name))
        else {
            continue;
        };
        match attach_recorded_thumbnail(db, ffmpeg, thumbnails_root, watch_folders, record, video) {
            Ok(()) => outcome.linked.push((image_name, video.file_name.clone())),
            Err(e) => log::warn!(
                "wb import: could not link {image_name} to {} by name: {}",
                video.id,
                e.message
            ),
        }
    }
    outcome
}

/// Everything linking one thumbnail needs, shared by all workers.
struct LinkContext<'a, F: FfmpegAdapter> {
    thumbnail_folder: &'a Path,
    thumbnails_root: &'a Path,
    watch_folders: &'a [String],
    layout: Option<&'a ThumbLayout>,
    ffmpeg: &'a F,
}

impl<F: FfmpegAdapter> LinkContext<'_, F> {
    /// Links one legacy thumbnail to its video; see [`convert_thumbnail_to_slots`].
    fn link_one(&self, video_id: &str, filename: &str, file_path: Option<&str>) -> ItemResult {
        // Defensive only: every `video_id` here came from `movies_for_matching`,
        // which is derived from the very same `registered_movies` the lookup
        // was built from, so a miss should never actually occur.
        let Some(file_path) = file_path else {
            log::warn!(
                "wb import: internal inconsistency -- matched video {video_id} has no known \
                 file_path; skipping its legacy thumbnail link"
            );
            return ItemResult::Failed("内部の不整合".to_string());
        };
        convert_thumbnail_to_slots(
            self.ffmpeg,
            self.thumbnails_root,
            self.watch_folders,
            video_id,
            file_path,
            &self.thumbnail_folder.join(filename),
            self.layout,
        )
    }
}

/// Converts one legacy thumbnail JPG into the video's slot files: creates the
/// video's thumbnail directory, then writes either its frames or the whole
/// image into every slot. A failure removes whatever slot files were written.
/// Shared by the import and by linking a thumbnail to a video by hand, so both
/// place identical files.
pub(crate) fn convert_thumbnail_to_slots<F: FfmpegAdapter>(
    ffmpeg: &F,
    thumbnails_root: &Path,
    watch_folders: &[String],
    video_id: &str,
    video_file_path: &str,
    src_path: &Path,
    layout: Option<&ThumbLayout>,
) -> ItemResult {
    let filename = src_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let filename = filename.as_str();
    let video_dir = crate::thumbnail::paths::video_thumbnail_dir(
        thumbnails_root,
        watch_folders,
        video_file_path,
    );
    if let Err(e) = fs::create_dir_all(&video_dir) {
        log::warn!(
            "wb import: failed to create thumbnail directory {} for video {video_id}: {e}",
            video_dir.display()
        );
        return ItemResult::Failed(format!("サムネイルの保存先を作れない: {e}"));
    }

    if let Some(rects) = layout.and_then(|l| split_rects(l, src_path, filename, ffmpeg)) {
        return match write_split_slots(ffmpeg, src_path, &video_dir, video_id, &rects) {
            Ok(()) => ItemResult::Linked { split: true },
            Err(e) => {
                log::warn!(
                    "wb import: failed to split thumbnail for video {video_id} \
                     ({filename}): {e}"
                );
                remove_slot_files(&video_dir, video_id);
                ItemResult::Failed(format!("分割に失敗: {e}"))
            }
        };
    }
    // Shared scratch file for the single conversion pass below; distinct
    // from the per-slot `{video_id}_{i}.webp.tmp` names used by
    // `thumbnail::worker` so the two pipelines can never collide on a
    // tmp path.
    let shared_tmp_path = video_dir.join(format!("{video_id}.wb_legacy.webp.tmp"));

    if let Err(e) = ffmpeg.convert_image_to_webp(src_path, &shared_tmp_path, WB_THUMBNAIL_QUALITY) {
        log::warn!(
            "wb import: thumbnail conversion failed for video {video_id} \
             ({filename}): {e}"
        );
        let _ = fs::remove_file(&shared_tmp_path);
        // An earlier item of the same video may have written its slots;
        // a video that ends up failed must not keep them.
        remove_slot_files(&video_dir, video_id);
        return ItemResult::Failed(format!("WebPへの変換に失敗: {e}"));
    }

    // Duplicate the converted image into every slot. Fail-fast on the
    // first copy error (same rationale as
    // `thumbnail::worker::generate_thumbnail_for_video`'s slot
    // extraction loop): don't leave a partially populated set of slot
    // files behind, since `get_thumbnails` / `list_videos_missing_thumbnails`
    // treat "all slots present" as the only valid state.
    let mut copy_err: Option<std::io::Error> = None;
    for i in 0..THUMBNAILS_PER_VIDEO {
        let final_path = crate::thumbnail::paths::slot_path(&video_dir, video_id, i);
        if let Err(e) = fs::copy(&shared_tmp_path, &final_path) {
            copy_err = Some(e);
            break;
        }
    }
    let _ = fs::remove_file(&shared_tmp_path);

    match copy_err {
        None => ItemResult::Linked { split: false },
        Some(e) => {
            log::warn!(
                "wb import: failed to finalize thumbnail slot(s) for video {video_id} \
                 ({filename}): {e}"
            );
            remove_slot_files(&video_dir, video_id);
            ItemResult::Failed(format!("保存に失敗: {e}"))
        }
    }
}

/// Runs `link` for every matched item on up to [`link_worker_count`] threads
/// and returns the results in the order of `matched`. Items of the same video
/// are handled one after another by a single worker, in `matched` order.
///
/// Progress is counted by items finished and reported under one lock, so the
/// counts handed to `on_progress` only ever rise, at the
/// [`is_progress_tick`] cadence. The final `(total, total)` is left to the
/// caller.
///
/// A panic while linking one item fails that item only, and the import thread
/// survives -- as with a panicking share probe, otherwise the UI would never
/// hear the import ended. An item that no worker got to also counts as failed.
fn link_in_parallel(
    matched: &[(String, String)],
    total: u32,
    on_progress: &(impl Fn(u32, u32) + Sync),
    link: impl Fn(&str, &str) -> ItemResult + Sync,
) -> Vec<ItemResult> {
    let slots: Vec<Mutex<Option<ItemResult>>> = matched.iter().map(|_| Mutex::new(None)).collect();
    // Items of one video share their slot and scratch file names, so they go
    // to one worker together, in `matched` order -- as in a serial run, the
    // later one wins. Groups are queued in order of first appearance.
    let mut group_of: HashMap<&str, usize> = HashMap::new();
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (idx, (video_id, _)) in matched.iter().enumerate() {
        let group = *group_of.entry(video_id.as_str()).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[group].push(idx);
    }
    let next = AtomicUsize::new(0);
    let finished = Mutex::new(0u32);
    let cores = thread::available_parallelism().map_or(1, |n| n.get());
    let worker_count = link_worker_count(cores).min(groups.len());
    thread::scope(|scope| {
        let handles: Vec<_> = (0..worker_count)
            .map(|_| {
                scope.spawn(|| {
                    while let Some(group) = groups.get(next.fetch_add(1, Ordering::Relaxed)) {
                        for &idx in group {
                            let (video_id, filename) = &matched[idx];
                            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                                link(video_id, filename)
                            }))
                            .unwrap_or_else(|_| {
                                log::warn!(
                                    "wb import: linking the thumbnail of {video_id} \
                                         panicked; counting it as failed"
                                );
                                ItemResult::Failed("内部エラー".to_string())
                            });
                            *slots[idx].lock().unwrap_or_else(PoisonError::into_inner) =
                                Some(result);

                            let mut finished =
                                finished.lock().unwrap_or_else(PoisonError::into_inner);
                            *finished += 1;
                            if *finished < total && is_progress_tick(*finished, total) {
                                on_progress(*finished, total);
                            }
                        }
                    }
                })
            })
            .collect();
        for handle in handles {
            if handle.join().is_err() {
                log::warn!("wb import: a thumbnail linking worker stopped unexpectedly");
            }
        }
    });
    slots
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .unwrap_or_else(PoisonError::into_inner)
                .unwrap_or_else(|| ItemResult::Failed("処理されなかった".to_string()))
        })
        .collect()
}

/// Bytes of an image file's start that are read to find its size: enough for
/// a JPEG header even behind a large EXIF block.
const IMAGE_HEADER_READ_LIMIT: u64 = 256 * 1024;

/// Width and height of a JPEG read from the file's first bytes, or `None` if
/// the file is unreadable or not a JPEG that can be sized that way.
fn read_jpeg_size(path: &Path) -> Option<(u32, u32)> {
    let file = fs::File::open(long_path::to_long_path(path)).ok()?;
    let mut head = Vec::new();
    file.take(IMAGE_HEADER_READ_LIMIT)
        .read_to_end(&mut head)
        .ok()?;
    jpeg_dimensions(&head)
}

/// Crop rectangles for `src_path`, or `None` (with a log line) when the JPG
/// has to be imported whole.
///
/// The size comes from the JPEG header, which costs no process launch. Only
/// when that cannot be read (not a JPEG, damaged) is ffprobe asked.
fn split_rects<F: FfmpegAdapter>(
    layout: &ThumbLayout,
    src_path: &Path,
    filename: &str,
    ffmpeg: &F,
) -> Option<Vec<CropRect>> {
    let (w, h) = match read_jpeg_size(src_path) {
        Some(size) => size,
        None => {
            let metadata = match ffmpeg.probe_metadata(src_path) {
                Ok(m) => m,
                Err(e) => {
                    log::warn!(
                        "wb import: could not read the size of {filename}: {e}; importing it whole"
                    );
                    return None;
                }
            };
            let (Some(w), Some(h)) = (metadata.width, metadata.height) else {
                log::warn!("wb import: {filename} reports no size; importing it whole");
                return None;
            };
            (w, h)
        }
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

/// Writes frame `i` of `src_path` to slot `i`. ffmpeg writes every frame in
/// one launch, each into its own `{video_id}_{i}.webp.tmp`; the renames only
/// start once all of them succeeded, so a slot is never half written. Stops
/// at the first failure; the caller removes whatever was written.
fn write_split_slots<F: FfmpegAdapter>(
    ffmpeg: &F,
    src_path: &Path,
    video_dir: &Path,
    video_id: &str,
    rects: &[CropRect],
) -> Result<(), String> {
    let outputs: Vec<CropOutput> = rects
        .iter()
        .enumerate()
        .map(|(i, rect)| CropOutput {
            tmp_path: video_dir.join(format!("{video_id}_{i}.webp.tmp")),
            crop: *rect,
        })
        .collect();
    ffmpeg
        .split_image_to_webp(src_path, &outputs, WB_THUMBNAIL_QUALITY)
        // One launch covers every frame, so the ffmpeg stderr tail in `e` is
        // the only detail there is; there is no failing frame index to name.
        .map_err(|e| format!("split: {e}"))?;
    for (i, output) in outputs.iter().enumerate() {
        let final_path = crate::thumbnail::paths::slot_path(video_dir, video_id, i);
        fs::rename(&output.tmp_path, &final_path).map_err(|e| format!("frame {i}: {e}"))?;
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

    #[test]
    fn a_panic_in_one_link_fails_only_that_item() {
        let matched: Vec<(String, String)> = (0..20)
            .map(|i| (format!("video-{i}"), format!("f{i}.jpg")))
            .collect();
        let progress = Mutex::new(Vec::new());

        let results = link_in_parallel(
            &matched,
            20,
            &|processed, total| progress.lock().unwrap().push((processed, total)),
            |video_id, _| {
                if video_id == "video-7" {
                    panic!("simulated");
                }
                ItemResult::Linked { split: false }
            },
        );

        assert_eq!(results.len(), 20);
        for (i, result) in results.iter().enumerate() {
            match (i, result) {
                (7, ItemResult::Failed(_)) => {}
                (7, _) => panic!("the panicking item must be failed"),
                (_, ItemResult::Linked { split: false }) => {}
                _ => panic!("item {i} must be linked"),
            }
        }
        // 20 items stay below the first progress tick; the caller sends the
        // final count.
        assert!(progress.lock().unwrap().is_empty());
    }

    #[test]
    fn items_of_one_video_are_linked_in_order_and_never_at_the_same_time() {
        // Three videos, each matched by several files, interleaved.
        let matched: Vec<(String, String)> = (0..30)
            .map(|i| (format!("video-{}", i % 3), format!("f{i}.jpg")))
            .collect();
        let active: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
        let order: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

        let results = link_in_parallel(&matched, 30, &|_, _| {}, |video_id, filename| {
            assert!(
                active.lock().unwrap().insert(video_id.to_string()),
                "{video_id} linked by two workers at once"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
            order
                .lock()
                .unwrap()
                .push((video_id.to_string(), filename.to_string()));
            active.lock().unwrap().remove(video_id);
            ItemResult::Linked { split: false }
        });

        assert_eq!(results.len(), 30);
        let order = order.lock().unwrap();
        for video in ["video-0", "video-1", "video-2"] {
            let seen: Vec<&String> = order
                .iter()
                .filter(|(v, _)| v == video)
                .map(|(_, f)| f)
                .collect();
            let expected: Vec<String> = matched
                .iter()
                .filter(|(v, _)| v == video)
                .map(|(_, f)| f.clone())
                .collect();
            assert_eq!(seen, expected.iter().collect::<Vec<_>>(), "{video}");
        }
    }

    #[test]
    fn items_no_worker_reached_after_a_panicking_progress_callback_count_as_failed() {
        // One video, so one group and one worker: when its progress callback
        // panics at the first tick, the rest of the group is never reached.
        let matched: Vec<(String, String)> = (0..120)
            .map(|i| ("video".to_string(), format!("f{i}.jpg")))
            .collect();

        let results = link_in_parallel(
            &matched,
            120,
            &|processed, _| {
                if processed == 50 {
                    panic!("simulated progress failure");
                }
            },
            |_, _| ItemResult::Linked { split: false },
        );

        assert_eq!(results.len(), 120);
        for (i, result) in results.iter().enumerate() {
            match result {
                ItemResult::Linked { .. } => assert!(i < 50, "item {i}"),
                ItemResult::Failed(reason) => {
                    assert!(i >= 50, "item {i}");
                    assert_eq!(reason, "処理されなかった");
                }
            }
        }
    }

    #[test]
    fn link_worker_count_scales_with_cores_within_bounds() {
        assert_eq!(link_worker_count(0), LINK_MIN_WORKERS);
        assert_eq!(link_worker_count(1), LINK_MIN_WORKERS);
        assert_eq!(link_worker_count(4), 2);
        assert_eq!(link_worker_count(8), 4);
        assert_eq!(link_worker_count(16), LINK_MAX_WORKERS);
        assert_eq!(link_worker_count(128), LINK_MAX_WORKERS);
    }
}
