//! Rows of the `videos` table: insert, lookup, listing, status/path/metadata updates and offline marking.

use gb_core::scan_pipeline::NewVideo;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, OptionalExtension};

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
    /// time something actually wrote it.
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
    /// Count of failed thumbnail generations. Read here so
    /// the list can tell "still being generated" from "automatic retries
    /// used up" without a second query per row.
    pub thumbnail_attempts: i64,
}

/// Shared column list + row-mapping closure for every query that selects a
/// full `videos` row, so the two never drift out of sync with each other.
const VIDEO_COLUMNS: &str = "id, file_path, file_name, file_size, duration, quick_hash, full_hash, status, rating, created_at, mtime, thumbnail_ready, thumbnail_attempts";

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
        thumbnail_attempts: r.get(12)?,
    })
}

/// Registers a new video. If `file_path` is already registered (e.g. a
/// repeat manual scan of the same folder), this is a no-op -- this function
/// does no file-move follow-up, so an existing row is left as-is.
pub fn insert_video(conn: &Connection, video: &NewVideo) -> rusqlite::Result<()> {
    insert_video_row(conn, video)?;
    Ok(())
}

/// Same insert as [`insert_video`], returning the affected row count (0 when
/// `file_path` was already registered) for callers that must tell the two
/// cases apart.
pub(super) fn insert_video_row(conn: &Connection, video: &NewVideo) -> rusqlite::Result<usize> {
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
    )
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
/// doc comment for why a plain prefix match is wrong here) are returned.
/// `ratings` (the rating bar's filter) is a fifth, independently-ANDed
/// filter: when non-empty, a row matches if its `rating` equals any listed
/// value exactly (OR within this filter; not "n or higher"). Duplicates are
/// dropped so the placeholder list stays minimal. The frontend only ever
/// sends values `>= 1` (there is no "show only unrated" mode), so unrated
/// rows (`rating = 0`) are naturally excluded whenever this filter is
/// active, with no special-casing needed here. An empty `search_terms`+
/// `tag_ids`+`ratings` and `folder_path: None`
/// (the common "browse everything" case) degrades to a plain unfiltered
/// listing.
pub fn list_videos_filtered(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    search_terms: &[String],
    sort_field: gb_core::sort::SortField,
    sort_direction: gb_core::sort::SortDirection,
    tag_ids: &[i64],
    folder_path: Option<&str>,
    ratings: &[u8],
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

    let mut unique_ratings: Vec<u8> = ratings.to_vec();
    unique_ratings.sort_unstable();
    unique_ratings.dedup();
    if !unique_ratings.is_empty() {
        // Placeholders are generated per value; the values themselves are
        // only ever bound, never interpolated into the SQL text.
        let placeholders = vec!["?"; unique_ratings.len()].join(", ");
        clauses.push(format!("rating IN ({placeholders})"));
        for rating in unique_ratings {
            bound_params.push(Box::new(rating as i64));
        }
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

/// Every `videos` row regardless of `status`, the same population
/// `list_videos_filtered` lists when nothing is filtered.
pub fn count_all_videos(conn: &Connection) -> rusqlite::Result<u32> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))?;
    Ok(count as u32)
}

/// `(rating, count)` for each rating that at least one video has, ascending.
/// Unrated videos are the `rating = 0` entry.
pub fn count_videos_by_rating(conn: &Connection) -> rusqlite::Result<Vec<(u8, u32)>> {
    let mut stmt =
        conn.prepare("SELECT rating, COUNT(*) FROM videos GROUP BY rating ORDER BY rating")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .filter_map(|(rating, count)| Some((u8::try_from(rating).ok()?, count as u32)))
        .collect())
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

/// `status='online'` rows sharing `quick_hash`+`file_size` with a newly
/// discovered file, other than the row (if any) at `excluding_path` itself.
/// These are either genuine duplicates or the same file seen before its old
/// folder was noticed to be gone; the caller tells them apart by checking
/// whether the old location is still reachable.
pub fn find_online_lookalikes_at_other_paths(
    conn: &Connection,
    quick_hash: &str,
    file_size: i64,
    excluding_path: &str,
) -> rusqlite::Result<Vec<VideoRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {VIDEO_COLUMNS} FROM videos
         WHERE status = 'online' AND quick_hash = ?1 AND file_size = ?2 AND file_path <> ?3
         ORDER BY created_at ASC"
    ))?;
    let rows = stmt
        .query_map(
            params![quick_hash, file_size, excluding_path],
            map_video_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `status='online'` rows whose `file_path` lies under `folder`, used to build
/// the "known" side of a NAS diff-scan pass
/// (`gb_core::reconcile::decide_missing_video_ids`'s `known_online`) and to
/// find the rows to flip offline when a folder becomes unreachable. Matching
/// is done in Rust (not SQL `LIKE`) to sidestep `LIKE` wildcard escaping, and
/// goes through `gb_core::paths::is_under_folder` so it is case-insensitive
/// (NTFS) and stops at a separator: `D:\video` must not pick up
/// `D:\video2\a.mp4`, or a sibling folder's rows would be treated as this
/// folder's (and flipped offline when this folder is scanned or goes down).
pub fn list_online_videos_under(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    folder: &str,
) -> anyhow::Result<Vec<VideoRow>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {VIDEO_COLUMNS} FROM videos WHERE status = 'online'"
    ))?;
    let rows = stmt
        .query_map([], map_video_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter(|v| gb_core::paths::is_under_folder(&v.file_path, folder))
        .collect();
    Ok(rows)
}

/// Ids per `UPDATE ... IN (...)`. SQLite's default bound-parameter limit is
/// 999 on older builds (32766 since 3.32); staying under the smaller one keeps
/// this independent of how SQLite was compiled.
const MARK_OFFLINE_CHUNK: usize = 500;

/// Flips the given rows from `online` to `offline` in one transaction, so a
/// failure part-way leaves no folder half-updated. Rows that are no longer
/// `online` (already offline, or removed meanwhile) are left alone and not
/// counted. Returns how many rows changed. Only `status` is written: tags,
/// rating and thumbnails stay attached to the row.
pub fn mark_videos_offline(conn: &mut Connection, ids: &[String]) -> rusqlite::Result<usize> {
    let tx = conn.transaction()?;
    let mut changed = 0;
    for chunk in ids.chunks(MARK_OFFLINE_CHUNK) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        changed += tx.execute(
            &format!(
                "UPDATE videos SET status = 'offline'
                 WHERE status = 'online' AND id IN ({placeholders})"
            ),
            rusqlite::params_from_iter(chunk.iter()),
        )?;
    }
    tx.commit()?;
    Ok(changed)
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
                unverifiable: false,
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
