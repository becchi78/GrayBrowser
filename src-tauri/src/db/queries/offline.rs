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
    let rows = stmt.query_map(params![pattern, limit], candidate_from_row)?;
    rows.collect()
}

/// Maps a row selected as `id, file_name, file_path, thumbnail_ready`.
fn candidate_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<OfflineVideoCandidate> {
    Ok(OfflineVideoCandidate {
        id: r.get(0)?,
        file_name: r.get(1)?,
        file_path: r.get(2)?,
        thumbnail_ready: r.get(3)?,
    })
}

/// Every offline video, in one read, for matching legacy thumbnails by name in
/// memory. Unlike [`search_offline_videos`] it is neither filtered nor capped.
pub fn list_offline_videos(conn: &Connection) -> rusqlite::Result<Vec<OfflineVideoCandidate>> {
    let mut stmt = conn.prepare(
        "SELECT id, file_name, file_path, thumbnail_ready FROM videos WHERE status = 'offline'",
    )?;
    let rows = stmt.query_map([], candidate_from_row)?;
    rows.collect()
}

#[cfg(test)]
mod offline_video_tests {
    use super::*;
    use crate::db::test_support::init_temp_db;

    fn insert_video(conn: &Connection, id: &str, name: &str, path: &str, status: &str, ready: i64) {
        conn.execute(
            "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status, thumbnail_ready) \
             VALUES (?1, ?2, ?3, 1, 'h', ?4, ?5)",
            params![id, path, name, status, ready],
        )
        .unwrap();
    }

    fn ids(found: &[OfflineVideoCandidate]) -> Vec<&str> {
        found.iter().map(|c| c.id.as_str()).collect()
    }

    #[test]
    fn list_offline_videos_returns_every_offline_video_with_its_flag() {
        let (_d, db) = init_temp_db();
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "a", "a", "Z:/a.mp4", "offline", 1);
        insert_video(&conn, "b", "b", "Z:/b.mp4", "offline", 0);
        insert_video(&conn, "c", "c", "Z:/c.mp4", "online", 0);

        let mut found: Vec<(String, bool)> = list_offline_videos(&conn)
            .unwrap()
            .into_iter()
            .map(|v| (v.id, v.thumbnail_ready))
            .collect();
        found.sort();
        assert_eq!(
            found,
            vec![("a".to_string(), true), ("b".to_string(), false)]
        );
    }

    #[test]
    fn search_returns_only_offline_videos_matching_case_insensitively() {
        let (_d, db) = init_temp_db();
        let conn = db.writer.lock().unwrap();
        insert_video(
            &conn,
            "off1",
            "My Video.mp4",
            "Z:/a/My Video.mp4",
            "offline",
            1,
        );
        insert_video(&conn, "off2", "other.mp4", "Z:/a/other.mp4", "offline", 0);
        insert_video(&conn, "on1", "my video.mp4", "C:/my video.mp4", "online", 0);

        let found = search_offline_videos(&conn, "MY vid", 10).unwrap();
        assert_eq!(
            found,
            vec![OfflineVideoCandidate {
                id: "off1".into(),
                file_name: "My Video.mp4".into(),
                file_path: "Z:/a/My Video.mp4".into(),
                thumbnail_ready: true,
            }]
        );
        assert!(search_offline_videos(&conn, "nomatch", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn search_treats_like_metacharacters_literally() {
        let (_d, db) = init_temp_db();
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "pct", "100%.mp4", "Z:/pct.mp4", "offline", 0);
        insert_video(&conn, "und", "a_b.mp4", "Z:/und.mp4", "offline", 0);
        insert_video(&conn, "axb", "axb.mp4", "Z:/axb.mp4", "offline", 0);
        insert_video(&conn, "bs", "c\\d.mp4", "Z:/bs.mp4", "offline", 0);
        insert_video(&conn, "other", "zzz.mp4", "Z:/zzz.mp4", "offline", 0);

        assert_eq!(
            ids(&search_offline_videos(&conn, "%", 10).unwrap()),
            ["pct"]
        );
        assert_eq!(
            ids(&search_offline_videos(&conn, "a_b", 10).unwrap()),
            ["und"]
        );
        assert_eq!(
            ids(&search_offline_videos(&conn, "\\", 10).unwrap()),
            ["bs"]
        );
        assert_eq!(
            ids(&search_offline_videos(&conn, "c\\d", 10).unwrap()),
            ["bs"]
        );
    }

    #[test]
    fn search_orders_by_name_then_path_and_honours_limit() {
        let (_d, db) = init_temp_db();
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "c", "b.mp4", "Z:/2/b.mp4", "offline", 0);
        insert_video(&conn, "a", "A.mp4", "Z:/1/A.mp4", "offline", 0);
        insert_video(&conn, "b", "b.mp4", "Z:/1/b.mp4", "offline", 0);

        assert_eq!(
            ids(&search_offline_videos(&conn, "mp4", 10).unwrap()),
            ["a", "b", "c"]
        );
        assert_eq!(
            ids(&search_offline_videos(&conn, "mp4", 2).unwrap()),
            ["a", "b"]
        );
        assert!(search_offline_videos(&conn, "mp4", 0).unwrap().is_empty());
    }

    #[test]
    fn search_with_an_empty_term_lists_offline_videos() {
        let (_d, db) = init_temp_db();
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "off", "a.mp4", "Z:/a.mp4", "offline", 0);
        insert_video(&conn, "on", "b.mp4", "C:/b.mp4", "online", 0);
        assert_eq!(ids(&search_offline_videos(&conn, "", 10).unwrap()), ["off"]);
    }
}
