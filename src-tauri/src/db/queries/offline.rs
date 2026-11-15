//! Offline videos shown in the search/restore views.

use rusqlite::{params, Connection};

/// An offline video offered as the target of a manual thumbnail link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfflineVideoCandidate {
    pub id: String,
    pub file_name: String,
    pub file_path: String,
    pub thumbnail_ready: bool,
}

/// Offline videos whose file name contains `term` (`%`, `_` and `\` in the
/// term match literally), at most `limit`, ordered by name then path. SQLite's
/// `LIKE` folds case for ASCII only, so a non-ASCII letter must match in case.
/// An empty term lists offline videos unfiltered.
pub fn search_offline_videos(
    conn: &Connection,
    term: &str,
    limit: u32,
) -> rusqlite::Result<Vec<OfflineVideoCandidate>> {
    let pattern = format!("%{}%", gb_core::search::escape_like_pattern(term));
    let mut stmt = conn.prepare(
        "SELECT id, file_name, file_path, thumbnail_ready FROM videos \
         WHERE status = 'offline' AND file_name LIKE ?1 ESCAPE '\\' \
         ORDER BY file_name COLLATE NOCASE, file_path LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![pattern, limit], |r| {
        Ok(OfflineVideoCandidate {
            id: r.get(0)?,
            file_name: r.get(1)?,
            file_path: r.get(2)?,
            thumbnail_ready: r.get(3)?,
        })
    })?;
    rows.collect()
}

/// Every offline video, in one read, for matching legacy thumbnails by name in
/// memory. Unlike [`search_offline_videos`] it is neither filtered nor capped.
pub fn list_offline_videos(conn: &Connection) -> rusqlite::Result<Vec<OfflineVideoCandidate>> {
    let mut stmt = conn.prepare(
        "SELECT id, file_name, file_path, thumbnail_ready FROM videos WHERE status = 'offline'",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(OfflineVideoCandidate {
            id: r.get(0)?,
            file_name: r.get(1)?,
            file_path: r.get(2)?,
            thumbnail_ready: r.get(3)?,
        })
    })?;
    rows.collect()
}
