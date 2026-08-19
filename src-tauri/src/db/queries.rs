//! CRUD against the `videos` / `skipped_files` / `settings` / `tags` /
//! `video_tags` tables. Writes (`insert_video`, `upsert_skipped_file`,
//! `set_watch_folders`) take the single writer `Connection`; reads
//! (`list_videos`, `list_skipped_files`) take a pooled read connection
//! (single-writer/many-readers rule).
//!
//! `video_tags` referential integrity: since
//! `PRAGMA foreign_keys` is deliberately OFF (see `db::disable_foreign_keys`),
//! nothing in SQLite itself stops an orphan `video_tags` row. The multi-
//! statement functions below (`assign_tag_to_video`, `delete_tag`,
//! `delete_video_cascade`) take `&mut Connection` specifically so they can
//! open a real `conn.transaction()` and enforce that integrity in the
//! application layer, mirroring `db::migrations::run_migrations`'s
//! transaction pattern: multiple `tx.execute()`/`tx.query_row()` calls, then
//! one `tx.commit()`, with an automatic rollback on any earlier `Err` (the
//! `Transaction` is simply dropped uncommitted).

use gb_core::scan_pipeline::{NewSkippedFile, NewVideo};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, OptionalExtension, Transaction};

pub struct VideoRow {
    pub id: String,
    pub file_path: String,
    pub file_name: String,
    pub file_size: i64,
    pub duration: Option<i64>,
    pub quick_hash: String,
    pub full_hash: Option<String>,
    pub status: String,
    pub rating: i64,
    pub created_at: String,
    /// Filesystem mtime (Unix seconds) as of the last scan/poll that touched
    /// this row. `None` for rows written before mtime tracking was added --
    /// `gb_core::reconcile::classify_discovered_file` treats that the same
    /// as "changed" (never as "unchanged").
    pub mtime: Option<i64>,
    /// Whether `thumbnails/[id].webp` was present on disk as of the last
    /// time something actually wrote it (migration 0008).
    /// Deliberately kept in `VIDEO_COLUMNS`/the hot path unlike the other
    /// ffprobe-derived columns below (`probed_at` etc., which are fetched
    /// lazily by `get_video_properties`) -- `list_videos` needs this value
    /// for *every* returned row, and the whole point of this column is to
    /// let it read that straight off the row instead of paying a
    /// filesystem `stat()` per row. This column is the source of truth only
    /// for that hot path; `thumbnail::worker`'s resume logic
    /// (`list_videos_missing_thumbnails`) still treats the filesystem itself
    /// as authoritative and self-heals this flag when it's stale.
    pub thumbnail_ready: bool,
}

/// Shared column list + row-mapping closure for every query that selects a
/// full `videos` row, so the two never drift out of sync with each other.
const VIDEO_COLUMNS: &str = "id, file_path, file_name, file_size, duration, quick_hash, full_hash, status, rating, created_at, mtime, thumbnail_ready";

fn map_video_row(r: &rusqlite::Row) -> rusqlite::Result<VideoRow> {
    Ok(VideoRow {
        id: r.get(0)?,
        file_path: r.get(1)?,
        file_name: r.get(2)?,
        file_size: r.get(3)?,
        duration: r.get(4)?,
        quick_hash: r.get(5)?,
        full_hash: r.get(6)?,
        status: r.get(7)?,
        rating: r.get(8)?,
        created_at: r.get(9)?,
        mtime: r.get(10)?,
        thumbnail_ready: r.get(11)?,
    })
}

pub struct SkippedFileRow {
    pub id: i64,
    pub file_path: String,
    pub file_name: String,
    pub reason: String,
    pub detected_char: Option<String>,
    pub detected_at: String,
}

/// Registers a new video. If `file_path` is already registered (e.g. a
/// repeat manual scan of the same folder), this is a no-op -- this function
/// does no file-move follow-up, so an existing row is left as-is.
pub fn insert_video(conn: &Connection, video: &NewVideo) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status, mtime)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(file_path) DO NOTHING",
        params![
            video.id,
            video.file_path,
            video.file_name,
            video.file_size as i64,
            video.quick_hash,
            video.status,
            video.mtime
        ],
    )?;
    Ok(())
}

/// Records a skipped file. Re-detecting the same path (e.g. repeat scan)
/// only refreshes `detected_at`.
pub fn upsert_skipped_file(conn: &Connection, skipped: &NewSkippedFile) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO skipped_files (file_path, file_name, reason, detected_char)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(file_path) DO UPDATE SET detected_at = CURRENT_TIMESTAMP",
        params![
            skipped.file_path,
            skipped.file_name,
            skipped.reason,
            skipped.detected_char.to_string()
        ],
    )?;
    Ok(())
}

/// Writes back a video's duration once ffprobe has determined it (called
/// from the thumbnail worker). Independent of whether thumbnail extraction
/// itself succeeds -- a probe that works but a frame-extraction that fails
/// (e.g. unsupported codec) should still keep the duration.
pub fn update_video_duration(
    conn: &Connection,
    video_id: &str,
    duration_secs: f64,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET duration = ?1 WHERE id = ?2",
        params![duration_secs.round() as i64, video_id],
    )?;
    Ok(())
}

/// Writes back ffprobe-derived metadata once the background probe worker
/// has determined it, and stamps `probed_at` so
/// `list_videos_missing_metadata_with_attempts` stops picking this row up.
/// Set unconditionally on success -- a partial result (e.g. no audio
/// stream) is still a completed probe, not a reason to retry indefinitely.
pub fn update_video_metadata(
    conn: &Connection,
    video_id: &str,
    metadata: &gb_core::ports::ffmpeg::VideoMetadata,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos
         SET width = ?1, height = ?2, video_codec = ?3, audio_codec = ?4,
             bitrate = ?5, fps = ?6, probed_at = CURRENT_TIMESTAMP
         WHERE id = ?7",
        params![
            metadata.width,
            metadata.height,
            metadata.video_codec,
            metadata.audio_codec,
            metadata.bitrate,
            metadata.fps,
            video_id,
        ],
    )?;
    Ok(())
}

pub struct VideoPropertiesRow {
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub bitrate: Option<i64>,
    pub fps: Option<f64>,
    /// `None` means "not yet probed" (the background probe worker hasn't
    /// reached this row) -- the panel must show that as a distinct pending
    /// state, not blank fields indistinguishable from a probe that failed.
    pub probed_at: Option<String>,
}

/// The ffprobe-derived columns deliberately excluded from `VIDEO_COLUMNS`/
/// `VideoDto` (kept off the hot `list_videos` payload) -- fetched lazily,
/// one row at a time, only when the properties panel actually opens for a
/// video.
pub fn get_video_properties(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    video_id: &str,
) -> anyhow::Result<Option<VideoPropertiesRow>> {
    let conn = pool.get()?;
    let row = conn
        .query_row(
            "SELECT width, height, video_codec, audio_codec, bitrate, fps, probed_at
             FROM videos WHERE id = ?1",
            [video_id],
            |r| {
                Ok(VideoPropertiesRow {
                    width: r.get(0)?,
                    height: r.get(1)?,
                    video_codec: r.get(2)?,
                    audio_codec: r.get(3)?,
                    bitrate: r.get(4)?,
                    fps: r.get(5)?,
                    probed_at: r.get(6)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// Filtered + sorted video listing. `search_terms` (already parsed
/// via `gb_core::search::parse_search_terms`) are ANDed together; each term
/// matches a row if `file_name`/`kana`/`roma` contains it as a substring
/// (`COALESCE(..., '')` makes NULL `kana`/`roma` -- only ever set via a
/// `.wb` import, so most rows have neither -- safely never-match rather
/// than crash or wrongly match everything).
/// `tag_ids` are also ANDed via `EXISTS` (a video must carry every selected
/// tag, using `idx_video_tags_tag`). `sort_field`/`sort_direction` resolve
/// through `gb_core::sort::order_by_clause` -- the only thing allowed to
/// produce raw `ORDER BY` SQL, so no frontend string reaches SQL text
/// directly. `folder_path` (the folder sidebar's filter) is a fourth,
/// independently-ANDed filter: when `Some`, only rows
/// whose `file_path` falls *under* that folder (honoring folder boundaries,
/// not a bare string prefix -- see `gb_core::paths::folder_like_prefix`'s
/// doc comment for why a plain prefix match is wrong here) are returned. An
/// empty `search_terms`+`tag_ids` and `folder_path: None` (the common
/// "browse everything" case) degrades to the original unfiltered
/// `list_videos` query this function replaces.
pub fn list_videos_filtered(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    search_terms: &[String],
    sort_field: gb_core::sort::SortField,
    sort_direction: gb_core::sort::SortDirection,
    tag_ids: &[i64],
    folder_path: Option<&str>,
) -> anyhow::Result<Vec<VideoRow>> {
    let conn = pool.get()?;

    let mut clauses: Vec<String> = Vec::new();
    let mut bound_params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    for term in search_terms {
        // Escaped once, reused for all three columns in this term's clause.
        let pattern = format!("%{}%", gb_core::search::escape_like_pattern(term));
        clauses.push(
            "(COALESCE(file_name,'') LIKE ? ESCAPE '\\' \
              OR COALESCE(kana,'') LIKE ? ESCAPE '\\' \
              OR COALESCE(roma,'') LIKE ? ESCAPE '\\')"
                .to_string(),
        );
        bound_params.push(Box::new(pattern.clone()));
        bound_params.push(Box::new(pattern.clone()));
        bound_params.push(Box::new(pattern));
    }

    for tag_id in tag_ids {
        clauses.push(
            "EXISTS (SELECT 1 FROM video_tags vt WHERE vt.video_id = videos.id AND vt.tag_id = ?)"
                .to_string(),
        );
        bound_params.push(Box::new(*tag_id));
    }

    if let Some(folder_path) = folder_path {
        let pattern = format!("{}%", gb_core::paths::folder_like_prefix(folder_path));
        clauses.push("file_path LIKE ? ESCAPE '\\'".to_string());
        bound_params.push(Box::new(pattern));
    }

    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    let order_sql = gb_core::sort::order_by_clause(sort_field, sort_direction);

    let sql = format!("SELECT {VIDEO_COLUMNS} FROM videos {where_sql} ORDER BY {order_sql}");
    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = bound_params.iter().map(|p| p.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), map_video_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Looks up a video by its exact current path. Used by per-file
/// reconciliation to tell "known path" (reconcile/reactivate) apart from
/// "genuinely new path" (path-follow match or plain insert) before any
/// hashing happens.
pub fn find_video_by_path(
    conn: &Connection,
    file_path: &str,
) -> rusqlite::Result<Option<VideoRow>> {
    conn.query_row(
        &format!("SELECT {VIDEO_COLUMNS} FROM videos WHERE file_path = ?1"),
        [file_path],
        map_video_row,
    )
    .optional()
}

/// `status='offline'` rows sharing `quick_hash`+`file_size` with a newly
/// discovered file (uses `idx_videos_quick_hash`).
/// Ordered `created_at ASC` so callers can hand the result straight to
/// `gb_core::reconcile::decide_path_follow`, which resolves multiple matches
/// to the first (earliest-registered) element deterministically.
pub fn find_offline_candidates_by_quick_hash_and_size(
    conn: &Connection,
    quick_hash: &str,
    file_size: i64,
) -> rusqlite::Result<Vec<VideoRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {VIDEO_COLUMNS} FROM videos
         WHERE status = 'offline' AND quick_hash = ?1 AND file_size = ?2
         ORDER BY created_at ASC"
    ))?;
    let rows = stmt
        .query_map(params![quick_hash, file_size], map_video_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `status='online'` rows whose `file_path` falls under `folder_prefix`,
/// used to build the "known" side of a NAS diff-scan pass
/// (`gb_core::reconcile::decide_missing_video_ids`'s `known_online`). Prefix
/// matching is done in Rust (not SQL `LIKE`) to sidestep `LIKE` wildcard
/// escaping entirely, and case-insensitively since NTFS/Windows paths are
/// case-insensitive.
pub fn list_online_videos_under(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    folder_prefix: &str,
) -> anyhow::Result<Vec<VideoRow>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {VIDEO_COLUMNS} FROM videos WHERE status = 'online'"
    ))?;
    let prefix_lower = folder_prefix.to_lowercase();
    let rows = stmt
        .query_map([], map_video_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|v| v.file_path.to_lowercase().starts_with(&prefix_lower))
        .collect();
    Ok(rows)
}

/// Overwrites `status` only (online/offline transitions and the
/// "reconnected at the same path" reactivation case).
pub fn update_video_status(
    conn: &Connection,
    video_id: &str,
    status: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET status = ?1 WHERE id = ?2",
        params![status, video_id],
    )?;
    Ok(())
}

/// The path-follow rewrite: reassigns `file_path`/`file_name` and
/// flips `status` (normally to `"online"`), preserving `id` (and therefore
/// tags/rating/thumbnail once those exist) across the move. `file_path` is
/// `UNIQUE`, so this can fail with a constraint-violation `Err` if the
/// caller didn't pre-check for a collision -- callers must treat that `Err`
/// as a recoverable "leave it offline" outcome, never propagate it as
/// fatal/panic.
pub fn update_video_path_and_status(
    conn: &Connection,
    video_id: &str,
    new_path: &str,
    new_file_name: &str,
    status: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET file_path = ?1, file_name = ?2, status = ?3 WHERE id = ?4",
        params![new_path, new_file_name, status, video_id],
    )?;
    Ok(())
}

/// Checks whether `file_path` currently belongs to a *different*, online
/// video row -- the pre-write guard for path-follow's UNIQUE-collision
/// handling. Returns the colliding video's id, if any.
pub fn is_path_used_by_online_video(
    conn: &Connection,
    file_path: &str,
    excluding_id: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM videos WHERE file_path = ?1 AND status = 'online' AND id != ?2",
        params![file_path, excluding_id],
        |r| r.get(0),
    )
    .optional()
}

/// Updates the scan-derived metadata for an already-known row after a
/// rescan/poll determines its content changed (`gb_core::reconcile`'s
/// `FileClassification::NeedsRehash`).
pub fn update_video_scan_metadata(
    conn: &Connection,
    video_id: &str,
    file_size: i64,
    mtime: i64,
    quick_hash: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET file_size = ?1, mtime = ?2, quick_hash = ?3 WHERE id = ?4",
        params![file_size, mtime, quick_hash, video_id],
    )?;
    Ok(())
}

pub fn list_skipped_files(
    pool: &r2d2::Pool<SqliteConnectionManager>,
) -> anyhow::Result<Vec<SkippedFileRow>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, file_path, file_name, reason, detected_char, detected_at
         FROM skipped_files ORDER BY detected_at DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(SkippedFileRow {
                id: r.get(0)?,
                file_path: r.get(1)?,
                file_name: r.get(2)?,
                reason: r.get(3)?,
                detected_char: r.get(4)?,
                detected_at: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

// --- Generation retry-limit tracking (gb_core::retry's pure classification)
// ----------------------------------------------------------------------
//
// `thumbnail_attempts`/`metadata_attempts` (migration 0007) count *failed*
// attempts only -- a successful generation is recorded elsewhere (a
// `thumbnails/[id].webp` file existing, `probed_at` being non-NULL) and never
// increments these columns. Whether a given attempts count still permits an
// automatic retry is `gb_core::retry::is_eligible_for_automatic_retry`'s job,
// not this module's -- these functions only read/write the raw counter.

/// Increments `thumbnail_attempts` by 1 for `video_id`. Called by
/// `thumbnail::worker` immediately after a generation attempt is
/// determined to have failed.
pub fn increment_thumbnail_attempts(conn: &Connection, video_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET thumbnail_attempts = thumbnail_attempts + 1 WHERE id = ?1",
        params![video_id],
    )?;
    Ok(())
}

/// Increments `metadata_attempts` by 1 for `video_id`. Called by
/// `metadata::worker` immediately after a probe attempt is
/// determined to have failed.
pub fn increment_metadata_attempts(conn: &Connection, video_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET metadata_attempts = metadata_attempts + 1 WHERE id = ?1",
        params![video_id],
    )?;
    Ok(())
}

/// Sets `thumbnail_ready = 1` for `video_id` (migration 0008).
/// Called by `thumbnail::worker::generate_thumbnail_for_video` right after a
/// successful generation, and by `list_videos_missing_thumbnails`'s resume
/// pass as a backfill for rows whose file already exists but whose flag
/// hadn't been set yet. The `AND thumbnail_ready = 0` guard makes the common
/// case (flag already set) a no-op write rather than an unconditional
/// `UPDATE` on every call.
pub fn mark_thumbnail_ready(conn: &Connection, video_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET thumbnail_ready = 1 WHERE id = ?1 AND thumbnail_ready = 0",
        params![video_id],
    )?;
    Ok(())
}

/// Resets `thumbnail_attempts` back to 0 for `video_id`. Intended for a
/// manual "retry" action (surfaced in the UI) that gives an exhausted video
/// another chance at automatic generation.
pub fn reset_thumbnail_attempts(conn: &Connection, video_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET thumbnail_attempts = 0 WHERE id = ?1",
        params![video_id],
    )?;
    Ok(())
}

/// Resets `metadata_attempts` back to 0 for `video_id`. Intended for a manual
/// "retry" action (surfaced in the UI) that gives an exhausted video another
/// chance at automatic probing.
pub fn reset_metadata_attempts(conn: &Connection, video_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET metadata_attempts = 0 WHERE id = ?1",
        params![video_id],
    )?;
    Ok(())
}

/// `(id, file_path, thumbnail_attempts, thumbnail_ready)` for every online
/// video -- lets `thumbnail::worker::list_videos_missing_thumbnails`
/// skip rows `gb_core::retry::is_eligible_for_automatic_retry`
/// deems exhausted without a second query per row. `thumbnail_ready`
/// additionally lets that caller skip its backfill write
/// (`mark_thumbnail_ready`) for rows the DB already knows are ready, instead
/// of calling it unconditionally on every row whose file happens to exist.
pub fn list_online_video_paths_with_thumbnail_attempts(
    pool: &r2d2::Pool<SqliteConnectionManager>,
) -> anyhow::Result<Vec<(String, String, i64, bool)>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, file_path, thumbnail_attempts, thumbnail_ready FROM videos WHERE status = 'online'",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `(id, file_path, metadata_attempts)` for every online video that hasn't
/// been probed for metadata yet -- mirrors
/// `list_online_video_paths_with_thumbnail_attempts` above, consumed by
/// `metadata::worker::list_videos_missing_metadata` the same way.
pub fn list_videos_missing_metadata_with_attempts(
    pool: &r2d2::Pool<SqliteConnectionManager>,
) -> anyhow::Result<Vec<(String, String, i64)>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, file_path, metadata_attempts FROM videos
         WHERE status = 'online' AND probed_at IS NULL",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// One row of `list_videos_with_exhausted_thumbnail_attempts`'s result --
/// enough for a UI list (which file, how many attempts) without
/// pulling in the full `VideoRow`/`VIDEO_COLUMNS` payload, same lightweight
/// intent as `SkippedFileRow`.
pub struct ExhaustedThumbnailRow {
    pub id: String,
    pub file_path: String,
    pub file_name: String,
    pub thumbnail_attempts: i64,
}

/// `status='online'` videos whose `thumbnail_attempts` has reached (or
/// somehow exceeded) `gb_core::retry::MAX_GENERATION_ATTEMPTS` -- i.e. every
/// video `gb_core::retry::classify_retry_status` would call `Exhausted` for
/// the thumbnail pipeline. For the "these files could not get a
/// thumbnail" notification panel.
pub fn list_videos_with_exhausted_thumbnail_attempts(
    pool: &r2d2::Pool<SqliteConnectionManager>,
) -> anyhow::Result<Vec<ExhaustedThumbnailRow>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, file_path, file_name, thumbnail_attempts FROM videos
         WHERE status = 'online' AND thumbnail_attempts >= ?1
         ORDER BY file_path",
    )?;
    let rows = stmt
        .query_map(
            params![gb_core::retry::MAX_GENERATION_ATTEMPTS as i64],
            |r| {
                Ok(ExhaustedThumbnailRow {
                    id: r.get(0)?,
                    file_path: r.get(1)?,
                    file_name: r.get(2)?,
                    thumbnail_attempts: r.get(3)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// One row of `list_videos_with_exhausted_metadata_attempts`'s result,
/// mirroring `ExhaustedThumbnailRow` above for the metadata pipeline.
pub struct ExhaustedMetadataRow {
    pub id: String,
    pub file_path: String,
    pub file_name: String,
    pub metadata_attempts: i64,
}

/// `status='online'` videos whose `metadata_attempts` has reached (or somehow
/// exceeded) `gb_core::retry::MAX_GENERATION_ATTEMPTS` -- the metadata
/// pipeline's counterpart to `list_videos_with_exhausted_thumbnail_attempts`.
pub fn list_videos_with_exhausted_metadata_attempts(
    pool: &r2d2::Pool<SqliteConnectionManager>,
) -> anyhow::Result<Vec<ExhaustedMetadataRow>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, file_path, file_name, metadata_attempts FROM videos
         WHERE status = 'online' AND metadata_attempts >= ?1
         ORDER BY file_path",
    )?;
    let rows = stmt
        .query_map(
            params![gb_core::retry::MAX_GENERATION_ATTEMPTS as i64],
            |r| {
                Ok(ExhaustedMetadataRow {
                    id: r.get(0)?,
                    file_path: r.get(1)?,
                    file_name: r.get(2)?,
                    metadata_attempts: r.get(3)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn get_setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .optional()
}

fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
        params![key, value],
    )?;
    Ok(())
}

const WATCH_FOLDERS_KEY: &str = "watch_folders";

pub fn get_watch_folders(conn: &Connection) -> anyhow::Result<Vec<String>> {
    match get_setting(conn, WATCH_FOLDERS_KEY)? {
        Some(json) => Ok(serde_json::from_str(&json)?),
        None => Ok(Vec::new()),
    }
}

pub fn set_watch_folders(conn: &Connection, folders: &[String]) -> anyhow::Result<()> {
    let json = serde_json::to_string(folders)?;
    set_setting(conn, WATCH_FOLDERS_KEY, &json)?;
    Ok(())
}

const NAS_POLL_INTERVAL_KEY: &str = "nas_polling_interval_sec";
/// Default NAS polling interval: periodic (e.g. every 10 minutes),
/// configurable by the user.
const DEFAULT_NAS_POLL_INTERVAL_SECS: i64 = 600;

pub fn get_nas_poll_interval_secs(conn: &Connection) -> anyhow::Result<i64> {
    match get_setting(conn, NAS_POLL_INTERVAL_KEY)? {
        Some(value) => Ok(value.parse()?),
        None => Ok(DEFAULT_NAS_POLL_INTERVAL_SECS),
    }
}

pub fn set_nas_poll_interval_secs(conn: &Connection, secs: i64) -> anyhow::Result<()> {
    set_setting(conn, NAS_POLL_INTERVAL_KEY, &secs.to_string())?;
    Ok(())
}

pub struct TagRow {
    pub id: i64,
    pub name: String,
}

/// Errors from `assign_tag_to_video`. Deliberately hand-rolled (no
/// `thiserror`) rather than pulling in a new `src-tauri` dependency for a
/// single error enum -- `thiserror` is already a `gb-core`-only dependency,
/// and adding it here would touch `Cargo.toml`/`Cargo.lock` for no real
/// benefit.
#[derive(Debug)]
pub enum TagMutationError {
    InvalidName(gb_core::tags::TagNameError),
    VideoNotFound { video_id: String },
    Sqlite(rusqlite::Error),
}

impl std::fmt::Display for TagMutationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TagMutationError::InvalidName(e) => write!(f, "invalid tag name: {e}"),
            TagMutationError::VideoNotFound { video_id } => {
                write!(f, "video {video_id} does not exist")
            }
            TagMutationError::Sqlite(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TagMutationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TagMutationError::InvalidName(e) => Some(e),
            TagMutationError::Sqlite(e) => Some(e),
            TagMutationError::VideoNotFound { .. } => None,
        }
    }
}

impl From<gb_core::tags::TagNameError> for TagMutationError {
    fn from(e: gb_core::tags::TagNameError) -> Self {
        TagMutationError::InvalidName(e)
    }
}

impl From<rusqlite::Error> for TagMutationError {
    fn from(e: rusqlite::Error) -> Self {
        TagMutationError::Sqlite(e)
    }
}

/// Looks up or creates a tag by its already-normalized name in one round
/// trip. `ON CONFLICT ... RETURNING` makes this atomic and single-statement
/// whether the tag already existed or not -- under this app's single-writer
/// lock there's no real race to protect against, but this keeps the SQL
/// itself provably correct independent of that (no separate
/// SELECT-then-INSERT window at all).
fn get_or_create_tag(tx: &Transaction, normalized_name: &str) -> rusqlite::Result<TagRow> {
    tx.query_row(
        "INSERT INTO tags (name) VALUES (?1)
         ON CONFLICT(name) DO UPDATE SET name = excluded.name
         RETURNING id, name",
        params![normalized_name],
        |r| {
            Ok(TagRow {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        },
    )
}

/// Assigns a tag (by raw, user-typed name) to a video, creating the tag if
/// it doesn't already exist. The app-layer referential-integrity guarantee
/// for `video_tags`: both sides of the pair are
/// checked/created to exist *before* the `video_tags` row is inserted, all
/// inside one transaction --
///
/// 1. `gb_core::tags::normalize_tag_name` -- rejects empty/whitespace-only
///    names before any DB access at all.
/// 2. `SELECT 1 FROM videos WHERE id = ?1` -- the `videos.id` side of the
///    check. Returns `VideoNotFound` (and rolls back, touching nothing) if
///    absent, so a rejected `video_id` never leaves behind an unreferenced
///    (if harmless) new `tags` row.
/// 3. `get_or_create_tag` -- the `tags.id` side: the row is guaranteed to
///    exist within this same transaction by the time we reach step 4.
/// 4. `INSERT INTO video_tags ... ON CONFLICT DO NOTHING` -- idempotent:
///    re-adding an already-present tag is a silent no-op (same convention as
///    `insert_video`'s `ON CONFLICT(file_path) DO NOTHING`).
pub fn assign_tag_to_video(
    conn: &mut Connection,
    video_id: &str,
    raw_tag_name: &str,
) -> Result<TagRow, TagMutationError> {
    let normalized_name = gb_core::tags::normalize_tag_name(raw_tag_name)?;

    let tx = conn.transaction()?;

    let video_exists = tx
        .query_row(
            "SELECT 1 FROM videos WHERE id = ?1",
            params![video_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !video_exists {
        return Err(TagMutationError::VideoNotFound {
            video_id: video_id.to_string(),
        });
    }

    let tag = get_or_create_tag(&tx, &normalized_name)?;

    tx.execute(
        "INSERT INTO video_tags (video_id, tag_id) VALUES (?1, ?2)
         ON CONFLICT(video_id, tag_id) DO NOTHING",
        params![video_id, tag.id],
    )?;

    tx.commit()?;
    Ok(tag)
}

/// Un-assigns a tag from a video. Deletes only the `video_tags` row --
/// **the `tags` master row is deliberately left in place** even if this was
/// its last reference, so it stays available for reuse without the user
/// having to retype it. A tag
/// disappears from the `tags` table only via an explicit `delete_tag` call.
/// Single-statement delete: removing a `video_tags` row can never create an
/// orphan (it only ever shrinks the set), so no transaction is needed beyond
/// SQLite's own per-statement atomicity.
pub fn remove_tag_from_video(
    conn: &Connection,
    video_id: &str,
    tag_id: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM video_tags WHERE video_id = ?1 AND tag_id = ?2",
        params![video_id, tag_id],
    )?;
    Ok(())
}

pub fn list_tags_for_video(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    video_id: &str,
) -> anyhow::Result<Vec<TagRow>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT tags.id, tags.name FROM tags
         JOIN video_tags ON video_tags.tag_id = tags.id
         WHERE video_tags.video_id = ?1
         ORDER BY tags.name",
    )?;
    let rows = stmt
        .query_map(params![video_id], |r| {
            Ok(TagRow {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Every tag in the `tags` master table, for a tag picker/autocomplete UI --
/// independent of whether it's currently assigned to any video.
pub fn list_all_tags(pool: &r2d2::Pool<SqliteConnectionManager>) -> anyhow::Result<Vec<TagRow>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare("SELECT id, name FROM tags ORDER BY name")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(TagRow {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Deletes a tag from the `tags` master table and every `video_tags` row
/// referencing it, in one transaction -- the orphan-prevention cascade
/// required for tag deletion. Order (children before
/// parent) doesn't affect correctness given FK enforcement is off, but
/// matches the semantics that would still hold if it were ever turned back
/// on.
pub fn delete_tag(conn: &mut Connection, tag_id: i64) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM video_tags WHERE tag_id = ?1", params![tag_id])?;
    tx.execute("DELETE FROM tags WHERE id = ?1", params![tag_id])?;
    tx.commit()
}

/// Deletes a video row and every `video_tags` row referencing it, in one
/// transaction.
///
/// Provides the orphan-prevention guarantee video deletion needs; used by
/// `commands::dedup_cmds::delete_duplicate_video`. Does not touch
/// `thumbnails/[id].webp` -- callers that also want to remove the cached
/// thumbnail files must do that separately.
pub fn delete_video_cascade(conn: &mut Connection, video_id: &str) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM video_tags WHERE video_id = ?1",
        params![video_id],
    )?;
    // path_collisions rows reference video_id from either side (the
    // offline row that attempted the move, or the online row that already
    // occupied the path) -- both must go, same app-layer orphan prevention
    // as the video_tags delete above.
    tx.execute(
        "DELETE FROM path_collisions WHERE video_id = ?1 OR colliding_video_id = ?1",
        params![video_id],
    )?;
    tx.execute("DELETE FROM videos WHERE id = ?1", params![video_id])?;
    tx.commit()
}

/// Counts `videos` rows currently under `folder_path` (same folder-boundary-
/// safe `LIKE` matching as `list_videos_filtered`'s `folder_path` argument,
/// via `gb_core::paths::folder_like_prefix`), for the folder-management
/// dialog's delete confirmation (shows the user how many videos' tags,
/// ratings, and registration dates under this folder will be lost).
/// Read-only -- pairs with `delete_videos_under_folder_cascade` below,
/// which performs the actual deletion once the user confirms.
pub fn count_videos_under_folder(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    folder_path: &str,
) -> anyhow::Result<u32> {
    let conn = pool.get()?;
    let pattern = format!("{}%", gb_core::paths::folder_like_prefix(folder_path));
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM videos WHERE file_path LIKE ?1 ESCAPE '\\'",
        params![pattern],
        |r| r.get(0),
    )?;
    Ok(count as u32)
}

/// Deletes every `videos` row under `folder_path` (same folder-boundary-safe
/// `LIKE` matching as `count_videos_under_folder`/`list_videos_filtered`),
/// plus every `video_tags`/`path_collisions` row referencing one of them, in
/// a **single transaction** -- the app-layer orphan-
/// prevention guarantee, extended from `delete_video_cascade`'s one-video
/// case to "every video under a folder" without looping that function call
/// by call (which would split the work across many transactions and risk
/// leaving a partially-deleted folder behind on a mid-loop failure).
///
/// The three `DELETE`s below intentionally re-run the same `videos`-matching
/// `LIKE` subquery/condition (rather than collecting ids into a bound `IN
/// (...)` list first) so `video_tags`/`path_collisions` are always resolved
/// against the *same* still-intact `videos` rows the final `DELETE FROM
/// videos` will remove -- the `videos` rows themselves are only ever touched
/// by that last statement, so every earlier statement's subquery still sees
/// the full untouched set.
///
/// Returns the deleted video ids (for the caller to also clean up their
/// cached `thumbnails/[id].webp` files, mirroring `delete_duplicate_video`'s
/// own thumbnail cleanup for the single-video case -- this function itself
/// never touches the filesystem).
pub fn delete_videos_under_folder_cascade(
    conn: &mut Connection,
    folder_path: &str,
) -> rusqlite::Result<Vec<String>> {
    let pattern = format!("{}%", gb_core::paths::folder_like_prefix(folder_path));
    let tx = conn.transaction()?;

    let ids: Vec<String> = {
        let mut stmt = tx.prepare("SELECT id FROM videos WHERE file_path LIKE ?1 ESCAPE '\\'")?;
        let rows = stmt
            .query_map(params![pattern], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    tx.execute(
        "DELETE FROM video_tags WHERE video_id IN (
             SELECT id FROM videos WHERE file_path LIKE ?1 ESCAPE '\\'
         )",
        params![pattern],
    )?;
    tx.execute(
        "DELETE FROM path_collisions WHERE video_id IN (
             SELECT id FROM videos WHERE file_path LIKE ?1 ESCAPE '\\'
         ) OR colliding_video_id IN (
             SELECT id FROM videos WHERE file_path LIKE ?1 ESCAPE '\\'
         )",
        params![pattern],
    )?;
    tx.execute(
        "DELETE FROM videos WHERE file_path LIKE ?1 ESCAPE '\\'",
        params![pattern],
    )?;

    tx.commit()?;
    Ok(ids)
}

/// Result of `rename_watch_folder_videos`: how many rows under the folder
/// were rewritten to the new path vs. left untouched because the new path
/// would collide with another video's `file_path`.
pub struct RenameWatchFolderOutcome {
    pub renamed_count: u32,
    pub collision_skipped_count: u32,
}

/// Checks whether `file_path` currently belongs to any *other* video row,
/// regardless of `status`. Broader than `is_path_used_by_online_video`'s
/// online-only check: `videos.file_path` is `UNIQUE` across *every* row
/// regardless of status (an offline row's `file_path` is never cleared, only
/// its `status` flips), so `rename_watch_folder_videos`'s batch `UPDATE`
/// must also treat an offline occupant as a collision -- otherwise the
/// `UPDATE` could hit the same `UNIQUE` violation `update_video_path_and_
/// status`'s doc comment warns about, but this time inside a transaction
/// whose atomicity guarantee we cannot afford to break.
fn find_colliding_video_id(
    tx: &Transaction,
    file_path: &str,
    excluding_id: &str,
) -> rusqlite::Result<Option<String>> {
    tx.query_row(
        "SELECT id FROM videos WHERE file_path = ?1 AND id != ?2",
        params![file_path, excluding_id],
        |r| r.get(0),
    )
    .optional()
}

/// The folder-management dialog's ✎ path edit: rewrites every
/// `videos.file_path` under `old_folder_path` to fall under
/// `new_folder_path` instead, preserving `id`/`file_name`/tags/
/// rating/`created_at` untouched -- only `file_path` and (per-row) `status`
/// change. Same folder-boundary-safe `LIKE` matching as
/// `delete_videos_under_folder_cascade`/`count_videos_under_folder`, and the
/// same "snapshot the affected rows before mutating anything" shape.
///
/// For each matched row:
/// - the new path is computed via `gb_core::paths::replace_folder_prefix`,
///   which only rewrites the folder portion -- `file_name` never changes, so
///   this function does not touch that column at all;
/// - if the new path would collide with another row's `file_path`
///   (`find_colliding_video_id`, any status), the row is left completely
///   untouched (old `file_path`/`status` preserved) and the collision is
///   recorded via `record_path_collision` for the duplicate-candidates UI to
///   surface later;
/// - otherwise `file_path` is rewritten and `status` is set to `"online"` or
///   `"offline"` based on a fresh `std::fs` existence check against the new
///   path -- no separate rescan is needed to reflect the move.
///
/// All of the above happens inside a single transaction: either
/// every non-colliding row is rewritten, or (on an `Err`) none of them are.
pub fn rename_watch_folder_videos(
    conn: &mut Connection,
    old_folder_path: &str,
    new_folder_path: &str,
) -> rusqlite::Result<RenameWatchFolderOutcome> {
    let pattern = format!("{}%", gb_core::paths::folder_like_prefix(old_folder_path));
    let tx = conn.transaction()?;

    let rows: Vec<(String, String)> = {
        let mut stmt =
            tx.prepare("SELECT id, file_path FROM videos WHERE file_path LIKE ?1 ESCAPE '\\'")?;
        let rows = stmt
            .query_map(params![pattern], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    let mut renamed_count = 0u32;
    let mut collision_skipped_count = 0u32;

    for (video_id, old_path) in rows {
        let Some(new_path) =
            gb_core::paths::replace_folder_prefix(&old_path, old_folder_path, new_folder_path)
        else {
            // Defensive only: every row here was already selected by the
            // same-shaped `LIKE` prefix match above, so this should be
            // unreachable. Leave the row untouched rather than guess.
            log::warn!(
                "rename_watch_folder_videos: {old_path} matched the folder LIKE prefix but \
                 replace_folder_prefix could not rewrite it under {old_folder_path} -- leaving \
                 it untouched"
            );
            collision_skipped_count += 1;
            continue;
        };

        if let Some(colliding_id) = find_colliding_video_id(&tx, &new_path, &video_id)? {
            record_path_collision(&tx, &video_id, &colliding_id, &new_path)?;
            collision_skipped_count += 1;
            continue;
        }

        let status = if std::path::Path::new(&new_path).exists() {
            "online"
        } else {
            "offline"
        };
        tx.execute(
            "UPDATE videos SET file_path = ?1, status = ?2 WHERE id = ?3",
            params![new_path, status, video_id],
        )?;
        renamed_count += 1;
    }

    tx.commit()?;
    Ok(RenameWatchFolderOutcome {
        renamed_count,
        collision_skipped_count,
    })
}

/// Sets `videos.rating` directly (0 = unrated/cleared, 1-5 = stars).
/// Caller (command layer) must validate `rating` via
/// `gb_core::rating::validate_rating` first -- this function trusts its
/// input and does not re-check the range itself.
pub fn set_rating(conn: &Connection, video_id: &str, rating: u8) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET rating = ?1 WHERE id = ?2",
        params![rating, video_id],
    )?;
    Ok(())
}

// --- Duplicate detection + path-collision persistence ----------------------

/// `status='online'` rows with a non-empty `quick_hash`, for
/// `gb_core::dedup::group_candidates_by_quick_hash`'s stage-1 grouping.
/// Filtering `status='online'` here (rather than relying solely on
/// `group_candidates_by_quick_hash`'s own empty-`quick_hash` defense) keeps
/// this query's contract self-explanatory: only currently-present files are
/// duplicate-detection candidates. The
/// `quick_hash != ''` filter is redundant with that function's own check but
/// kept here too so this row set is already duplicate-detection-ready
/// without relying on the caller remembering that detail.
pub fn list_online_video_hash_info(
    pool: &r2d2::Pool<SqliteConnectionManager>,
) -> anyhow::Result<Vec<gb_core::dedup::VideoHashInfo>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, quick_hash, file_size, full_hash FROM videos
         WHERE status = 'online' AND quick_hash != ''",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(gb_core::dedup::VideoHashInfo {
                id: r.get(0)?,
                quick_hash: r.get(1)?,
                file_size: r.get(2)?,
                full_hash: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Writes back a video's `full_hash` (BLAKE3, `gb_core::hash::full_hash`)
/// once stage-2 duplicate confirmation has computed it for a
/// candidate group member. `videos.full_hash` has existed since the
/// initial schema (always NULL until this call) -- no migration needed.
pub fn update_full_hash(
    conn: &Connection,
    video_id: &str,
    full_hash: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET full_hash = ?1 WHERE id = ?2",
        params![full_hash, video_id],
    )?;
    Ok(())
}

pub struct PathCollisionRow {
    pub id: i64,
    pub video_id: String,
    pub colliding_video_id: String,
    pub attempted_path: String,
    pub detected_at: String,
}

/// Records a duplicate-candidate pair pending full_hash confirmation
/// (`dedup::detect_duplicate_groups` reads these back uniformly). Used by two
/// distinct routes in `scan::mod`, both funneling into this same table
/// because both produce the identical "offline candidate `video_id` /
/// online row `colliding_video_id` may share content" shape:
/// - 経路X (`register_new_path`'s `BlockedByCollision`): `video_id` (the
///   offline row) tried to follow its path to
///   `attempted_path`, but `colliding_video_id` (an online row) already
///   occupies that exact path.
/// - 経路Y (`reconcile_known_path`'s `NeedsRehash` arm):
///   `colliding_video_id` (an online row, rehashed in place) happens to now
///   share quick_hash+file_size with `video_id` (an unrelated offline row);
///   `attempted_path` is `colliding_video_id`'s own current path, not a path
///   `video_id` tried to claim.
///
/// Re-detecting the exact same pair (e.g. a repeat scan/poll before the user
/// resolves it) only refreshes `detected_at`/`attempted_path`, mirroring
/// `upsert_skipped_file`'s "re-detection just bumps the timestamp" pattern --
/// it must not pile up duplicate rows for the same standing collision.
pub fn record_path_collision(
    conn: &Connection,
    video_id: &str,
    colliding_video_id: &str,
    attempted_path: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO path_collisions (video_id, colliding_video_id, attempted_path)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(video_id, colliding_video_id)
         DO UPDATE SET detected_at = CURRENT_TIMESTAMP, attempted_path = excluded.attempted_path",
        params![video_id, colliding_video_id, attempted_path],
    )?;
    Ok(())
}

pub fn list_path_collisions(
    pool: &r2d2::Pool<SqliteConnectionManager>,
) -> anyhow::Result<Vec<PathCollisionRow>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, video_id, colliding_video_id, attempted_path, detected_at
         FROM path_collisions ORDER BY detected_at DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(PathCollisionRow {
                id: r.get(0)?,
                video_id: r.get(1)?,
                colliding_video_id: r.get(2)?,
                attempted_path: r.get(3)?,
                detected_at: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Looks up a single video by `id` (not `find_video_by_path`'s exact
/// `file_path` match) -- used by `dedup::hydrate_members`/`dedup::
/// ensure_full_hash` to turn the bare ids `gb_core::dedup`'s
/// pure grouping functions and `path_collisions` rows carry into full
/// display rows (file_path/file_name/file_size/status/created_at), and, via
/// `full_hash`, to check whether a full_hash has already been computed
/// before doing any file I/O.
pub fn find_video_by_id(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    video_id: &str,
) -> anyhow::Result<Option<VideoRow>> {
    let conn = pool.get()?;
    let row = conn
        .query_row(
            &format!("SELECT {VIDEO_COLUMNS} FROM videos WHERE id = ?1"),
            [video_id],
            map_video_row,
        )
        .optional()?;
    Ok(row)
}

// --- `.wb` import ------------------------------------------------------

/// Outcome of `import_wb_video`, distinguishing "this `.wb` row created a new
/// `videos` row" from "the `file_path` was already registered, so the whole
/// call was a no-op" -- the import log needs to tally both counts
/// separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WbImportOutcome {
    /// A new `videos` row was inserted and `tags_assigned` `video_tags`
    /// associations were written for it. Counts every input `tags` element
    /// that was actually assigned (duplicates within the slice each count,
    /// since `ON CONFLICT(video_id, tag_id) DO NOTHING` makes re-assigning
    /// the same tag harmless) -- except an element that normalizes to an
    /// empty tag name (see `find_or_create_tag_folding_case`), which is
    /// skipped and therefore not counted here.
    Inserted { tags_assigned: usize },
    /// `file_path` was already registered (e.g. a prior scan, or a repeat
    /// `.wb` import run). Nothing was written -- in particular, rating/kana/
    /// roma/tags are left exactly as they were, so a user edit is
    /// never clobbered by re-running the legacy import.
    Skipped,
}

/// Error from `find_or_create_tag_folding_case`'s "create" path. Kept
/// private/minimal -- it only exists to let `import_wb_video`'s tag loop
/// tell "this raw `.wb` tag string can't become a valid tag name, skip just
/// this one" apart from "a real SQLite failure happened, roll back the whole
/// transaction".
#[derive(Debug)]
enum WbTagResolutionError {
    /// `gb_core::tags::normalize_tag_name` rejected `display_name_if_new`
    /// (width-folds to empty/whitespace-only). `wb_import::split_tags`
    /// already trims and drops blank lines with Rust's own Unicode-aware
    /// `str::trim` -- which already strips the full-width space
    /// (`\u{3000}`) `normalize_tag_name`'s width-fold would otherwise turn
    /// into one -- so this should not occur on real `.wb` data; kept as a
    /// defensive case rather than a `debug_assert`/panic so one malformed
    /// tag can never take down an otherwise-valid video import.
    Empty,
    Sqlite(rusqlite::Error),
}

impl From<rusqlite::Error> for WbTagResolutionError {
    fn from(e: rusqlite::Error) -> Self {
        WbTagResolutionError::Sqlite(e)
    }
}

/// Finds an existing tag whose name folds (via `gb_core::wb_import::wb_tag_merge_key`)
/// to `merge_key`, or creates one named `display_name_if_new` (after running
/// it through `gb_core::tags::normalize_tag_name`) if none exists.
///
/// Scans every row in `tags` (via `tx`, so it sees any tag rows this same
/// transaction has already created) rather than doing the fold in SQL --
/// real data tops out at a few hundred tags, so an O(n) scan per call is not
/// worth indexing or caching for. The *first* matching existing row (by
/// `tags` table order) wins so import_wb_video's outcome is deterministic;
/// no new `tags` row is created when a fold match exists.
///
/// The "create" path normalizes `display_name_if_new` first (width-fold +
/// trim, no case-fold) so it upholds the invariant that every
/// `tags.name` is already width-folded -- otherwise a `.wb`-only full-width
/// tag (e.g. "Ａction") would be stored as-is, and a later manual tag entry
/// of the half-width "Action" (normalized by `normalize_tag_name` the same
/// way user-typed tags always are) would fail to match it on `UNIQUE(name)`
/// and create a duplicate row, defeating the point of this function.
fn find_or_create_tag_folding_case(
    tx: &Transaction,
    merge_key: &str,
    display_name_if_new: &str,
) -> Result<TagRow, WbTagResolutionError> {
    let mut stmt = tx.prepare("SELECT id, name FROM tags")?;
    let existing = stmt
        .query_map([], |r| {
            Ok(TagRow {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    if let Some(found) = existing
        .into_iter()
        .find(|row| gb_core::wb_import::wb_tag_merge_key(&row.name) == merge_key)
    {
        return Ok(found);
    }

    let normalized = gb_core::tags::normalize_tag_name(display_name_if_new)
        .map_err(|_| WbTagResolutionError::Empty)?;
    Ok(get_or_create_tag(tx, &normalized)?)
}

/// Imports one `.wb` video (Stage 1's `ImportCandidate`, minus the caller-
/// assembled `NewVideo`) in a single transaction, mirroring
/// `assign_tag_to_video`'s app-layer referential-integrity pattern:
///
/// 1. `INSERT INTO videos ... ON CONFLICT(file_path) DO NOTHING` -- the same
///    idempotency `insert_video` already relies on for repeat scans, reused
///    here so re-running a `.wb` import is always safe.
/// 2. If that insert changed 0 rows, the video was already registered --
///    commit immediately (nothing else was touched) and report `Skipped`,
///    deliberately *not* overwriting rating/kana/roma/tags so a user
///    edit always wins over stale `.wb` data.
/// 3. Otherwise (1 row inserted): write `rating`/`kana`/`roma`, then for each
///    raw `.wb` tag string, fold it via `wb_tag_merge_key` through
///    `find_or_create_tag_folding_case` and assign it
///    (`ON CONFLICT(video_id, tag_id) DO NOTHING`, same as
///    `assign_tag_to_video`).
///
/// Any error at any step drops `tx` uncommitted, rolling back the entire
/// video row, rating/kana/roma write, and every tag/assignment attempted so
/// far -- no orphaned `videos` or `video_tags` rows are ever left behind.
pub fn import_wb_video(
    conn: &mut Connection,
    video: &NewVideo,
    rating: u8,
    kana: &str,
    roma: &str,
    tags: &[String],
) -> rusqlite::Result<WbImportOutcome> {
    let tx = conn.transaction()?;

    let inserted = tx.execute(
        "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status, mtime)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(file_path) DO NOTHING",
        params![
            video.id,
            video.file_path,
            video.file_name,
            video.file_size as i64,
            video.quick_hash,
            video.status,
            video.mtime
        ],
    )?;

    if inserted == 0 {
        tx.commit()?;
        return Ok(WbImportOutcome::Skipped);
    }

    tx.execute(
        "UPDATE videos SET rating = ?1, kana = ?2, roma = ?3 WHERE id = ?4",
        params![rating, kana, roma, video.id],
    )?;

    let mut tags_assigned = 0usize;
    for raw_tag in tags {
        let merge_key = gb_core::wb_import::wb_tag_merge_key(raw_tag);
        let tag = match find_or_create_tag_folding_case(&tx, &merge_key, raw_tag) {
            Ok(tag) => tag,
            // Not a SQLite failure -- this one raw tag string can't become a
            // valid tag name (see `WbTagResolutionError::Empty`). Skip just
            // it and keep importing the rest of this video's tags/rating/
            // kana/roma rather than rolling back the whole row over it.
            Err(WbTagResolutionError::Empty) => {
                log::warn!(
                    "skipping .wb tag {raw_tag:?} for video {}: normalizes to an empty tag name",
                    video.id
                );
                continue;
            }
            Err(WbTagResolutionError::Sqlite(e)) => return Err(e),
        };
        tx.execute(
            "INSERT INTO video_tags (video_id, tag_id) VALUES (?1, ?2)
             ON CONFLICT(video_id, tag_id) DO NOTHING",
            params![video.id, tag.id],
        )?;
        tags_assigned += 1;
    }

    tx.commit()?;
    Ok(WbImportOutcome::Inserted { tags_assigned })
}

#[cfg(test)]
mod wb_import_tests {
    use super::*;
    use crate::db::test_support::init_temp_db;

    fn new_video(id: &str, file_path: &str) -> NewVideo {
        NewVideo {
            id: id.to_string(),
            file_path: file_path.to_string(),
            file_name: "movie.mp4".to_string(),
            file_size: 1234,
            quick_hash: "h".to_string(),
            status: "online",
            mtime: 1_700_000_000,
        }
    }

    fn tags_of(conn: &Connection) -> Vec<String> {
        let mut stmt = conn.prepare("SELECT name FROM tags ORDER BY name").unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    fn video_tag_ids_for(conn: &Connection, video_id: &str) -> Vec<i64> {
        let mut stmt = conn
            .prepare("SELECT tag_id FROM video_tags WHERE video_id = ?1 ORDER BY tag_id")
            .unwrap();
        stmt.query_map(params![video_id], |r| r.get::<_, i64>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    #[test]
    fn new_video_creates_row_rating_kana_roma_and_tags() {
        let (_dir, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();

        let video = new_video("id-1", "C:/videos/a.mp4");
        let outcome = import_wb_video(
            &mut conn,
            &video,
            4,
            "アクション",
            "akushon",
            &["Action".to_string(), "Comedy".to_string()],
        )
        .unwrap();

        assert_eq!(outcome, WbImportOutcome::Inserted { tags_assigned: 2 });

        let row = find_video_by_path(&conn, "C:/videos/a.mp4")
            .unwrap()
            .expect("video row should exist");
        assert_eq!(row.id, "id-1");
        assert_eq!(row.rating, 4);

        let (kana, roma): (Option<String>, Option<String>) = conn
            .query_row("SELECT kana, roma FROM videos WHERE id = 'id-1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(kana.as_deref(), Some("アクション"));
        assert_eq!(roma.as_deref(), Some("akushon"));

        assert_eq!(
            tags_of(&conn),
            vec!["Action".to_string(), "Comedy".to_string()]
        );
        assert_eq!(video_tag_ids_for(&conn, "id-1").len(), 2);
    }

    #[test]
    fn repeat_import_of_same_file_path_is_fully_skipped_and_does_not_overwrite() {
        let (_dir, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();

        let video = new_video("id-1", "C:/videos/a.mp4");
        let first = import_wb_video(
            &mut conn,
            &video,
            2,
            "かな1",
            "roma1",
            &["Original".to_string()],
        )
        .unwrap();
        assert_eq!(first, WbImportOutcome::Inserted { tags_assigned: 1 });

        // Second call: different id (as a fresh UUID generated by a re-run
        // would be), different rating/kana/roma/tags -- all of it must be
        // ignored because file_path already exists.
        let video_retry = new_video("id-2-different-uuid", "C:/videos/a.mp4");
        let second = import_wb_video(
            &mut conn,
            &video_retry,
            5,
            "かな2",
            "roma2",
            &["Different".to_string(), "MoreTags".to_string()],
        )
        .unwrap();
        assert_eq!(second, WbImportOutcome::Skipped);

        // Exactly one videos row, unchanged from the first call.
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let row = find_video_by_path(&conn, "C:/videos/a.mp4")
            .unwrap()
            .expect("video row should exist");
        assert_eq!(row.id, "id-1");
        assert_eq!(row.rating, 2, "second call's rating=5 must be ignored");

        let (kana, roma): (Option<String>, Option<String>) = conn
            .query_row("SELECT kana, roma FROM videos WHERE id = 'id-1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(kana.as_deref(), Some("かな1"));
        assert_eq!(roma.as_deref(), Some("roma1"));

        // Only the first call's tag exists; the second call's tags were
        // never touched.
        assert_eq!(tags_of(&conn), vec!["Original".to_string()]);
        assert_eq!(video_tag_ids_for(&conn, "id-1").len(), 1);
    }

    #[test]
    fn case_and_width_variant_tags_fold_into_one_tags_row() {
        let (_dir, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();

        let video_a = new_video("id-a", "C:/videos/a.mp4");
        import_wb_video(&mut conn, &video_a, 0, "", "", &["Action".to_string()]).unwrap();

        let video_b = new_video("id-b", "C:/videos/b.mp4");
        import_wb_video(&mut conn, &video_b, 0, "", "", &["action".to_string()]).unwrap();

        // Still exactly one "Action"/"action" tag row -- the second import's
        // lowercase variant folded onto the first's, no new row was created.
        assert_eq!(tags_of(&conn), vec!["Action".to_string()]);

        let tag_id: i64 = conn
            .query_row("SELECT id FROM tags WHERE name = 'Action'", [], |r| {
                r.get(0)
            })
            .unwrap();

        assert_eq!(video_tag_ids_for(&conn, "id-a"), vec![tag_id]);
        assert_eq!(video_tag_ids_for(&conn, "id-b"), vec![tag_id]);
    }

    #[test]
    fn error_partway_through_rolls_back_video_row_and_earlier_tag_writes() {
        let (_dir, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();

        // A trigger that fails the *second* tag's insert into `tags`,
        // simulating a mid-transaction error after the video row, its
        // rating/kana/roma, and the first tag/assignment have already been
        // written within the same (uncommitted) transaction.
        conn.execute(
            "CREATE TRIGGER fail_on_boom_tag BEFORE INSERT ON tags
             WHEN NEW.name = 'boom'
             BEGIN SELECT RAISE(ABORT, 'simulated failure'); END;",
            [],
        )
        .unwrap();

        let video = new_video("id-1", "C:/videos/a.mp4");
        let result = import_wb_video(
            &mut conn,
            &video,
            3,
            "kana",
            "roma",
            &["safe".to_string(), "boom".to_string()],
        );
        assert!(result.is_err(), "the simulated failure should propagate");

        let video_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(video_count, 0, "the videos row must be rolled back too");

        let tag_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            tag_count, 0,
            "the first tag ('safe'), already inserted before the failure, must be rolled back"
        );

        let vt_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM video_tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            vt_count, 0,
            "no video_tags orphan may remain for the rolled-back video/tag"
        );
    }

    #[test]
    fn a_new_tag_created_from_a_full_width_wb_tag_is_stored_width_folded() {
        let (_dir, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();

        // Full-width "Action" (U+FF21/FF43/FF54/FF49/FF4F/FF4E), same
        // construction as wb_import.rs's own
        // wb_tag_merge_key_folds_full_width_characters test. No tag named
        // (half-width) "Action" exists yet, so this must go through
        // find_or_create_tag_folding_case's *create* path.
        let full_width_action = "\u{FF21}\u{FF43}\u{FF54}\u{FF49}\u{FF4F}\u{FF4E}";

        let video = new_video("id-1", "C:/videos/a.mp4");
        let outcome = import_wb_video(
            &mut conn,
            &video,
            0,
            "",
            "",
            &[full_width_action.to_string()],
        )
        .unwrap();
        assert_eq!(outcome, WbImportOutcome::Inserted { tags_assigned: 1 });

        // Stored half-width, per the "tags.name is always
        // width-folded" invariant -- not the raw full-width `.wb` string.
        assert_eq!(tags_of(&conn), vec!["Action".to_string()]);

        // A later manual tag entry of the half-width "Action" (as
        // gb_core::tags::normalize_tag_name would produce from user input)
        // must now match this same row on UNIQUE(name), not create a
        // duplicate.
        let tag_id: i64 = conn
            .query_row("SELECT id FROM tags WHERE name = 'Action'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(video_tag_ids_for(&conn, "id-1"), vec![tag_id]);
    }

    #[test]
    fn a_tag_that_normalizes_to_empty_is_skipped_without_failing_the_rest_of_the_import() {
        let (_dir, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();

        // A full-width space alone width-folds to a plain space, which then
        // trims away to nothing -- gb_core::tags::normalize_tag_name rejects
        // it. (In practice wb_import::split_tags's own Unicode-aware
        // str::trim already drops a tag line like this before it ever
        // reaches import_wb_video; this test exercises the defensive path
        // directly.)
        let video = new_video("id-1", "C:/videos/a.mp4");
        let outcome = import_wb_video(
            &mut conn,
            &video,
            0,
            "",
            "",
            &["Action".to_string(), "\u{3000}".to_string()],
        )
        .unwrap();

        // The video row and its one valid tag were still fully imported --
        // one bad tag string does not roll back the whole video.
        assert_eq!(outcome, WbImportOutcome::Inserted { tags_assigned: 1 });
        assert_eq!(tags_of(&conn), vec!["Action".to_string()]);
        assert_eq!(video_tag_ids_for(&conn, "id-1").len(), 1);

        let video_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(video_count, 1);
    }
}
