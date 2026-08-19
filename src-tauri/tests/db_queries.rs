//! Integration tests for `db::queries` functions, against
//! a real tempdir-backed SQLite file. Test rows are inserted via raw SQL
//! (not `queries::insert_video`) so each test can freely control
//! `status`/`quick_hash`/`file_size`/`mtime`/`created_at` directly.

use graybrowser_lib::db::{self, queries};
use rusqlite::{params, Connection};

fn init_temp_db() -> (tempfile::TempDir, db::Db) {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");
    let db = db::init(&db_path).expect("db::init should succeed against a fresh tempdir file");
    (dir, db)
}

#[allow(clippy::too_many_arguments)]
fn insert_test_video(
    conn: &Connection,
    id: &str,
    file_path: &str,
    quick_hash: &str,
    file_size: i64,
    status: &str,
    mtime: Option<i64>,
    created_at: &str,
) {
    conn.execute(
        "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status, mtime, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            file_path,
            file_path.rsplit('\\').next().unwrap_or(file_path),
            file_size,
            quick_hash,
            status,
            mtime,
            created_at
        ],
    )
    .unwrap();
}

#[test]
fn find_video_by_path_returns_none_for_an_unknown_path() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    assert!(queries::find_video_by_path(&conn, "D:\\nope.mp4")
        .unwrap()
        .is_none());
}

#[test]
fn find_video_by_path_returns_the_matching_row() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    insert_test_video(
        &conn,
        "v1",
        "D:\\videos\\a.mp4",
        "abc123",
        1000,
        "online",
        Some(555),
        "2026-01-01 00:00:00",
    );

    let row = queries::find_video_by_path(&conn, "D:\\videos\\a.mp4")
        .unwrap()
        .expect("row should be found");
    assert_eq!(row.id, "v1");
    assert_eq!(row.quick_hash, "abc123");
    assert_eq!(row.file_size, 1000);
    assert_eq!(row.mtime, Some(555));
}

#[test]
fn find_offline_candidates_matches_by_hash_and_size_only_among_offline_rows() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    // Matching hash+size but online -- must not be returned.
    insert_test_video(
        &conn,
        "online-match",
        "D:\\a.mp4",
        "hash1",
        1000,
        "online",
        None,
        "2026-01-01 00:00:00",
    );
    // Offline but different hash -- must not be returned.
    insert_test_video(
        &conn,
        "offline-nomatch",
        "D:\\b.mp4",
        "hash2",
        1000,
        "offline",
        None,
        "2026-01-01 00:00:01",
    );
    // Offline, matching hash+size -- should be returned.
    insert_test_video(
        &conn,
        "offline-match",
        "D:\\c.mp4",
        "hash1",
        1000,
        "offline",
        None,
        "2026-01-01 00:00:02",
    );

    let candidates =
        queries::find_offline_candidates_by_quick_hash_and_size(&conn, "hash1", 1000).unwrap();
    let ids: Vec<&str> = candidates.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, vec!["offline-match"]);
}

#[test]
fn find_offline_candidates_orders_by_created_at_ascending() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    insert_test_video(
        &conn,
        "later",
        "D:\\later.mp4",
        "hash1",
        1000,
        "offline",
        None,
        "2026-01-02 00:00:00",
    );
    insert_test_video(
        &conn,
        "earlier",
        "D:\\earlier.mp4",
        "hash1",
        1000,
        "offline",
        None,
        "2026-01-01 00:00:00",
    );

    let candidates =
        queries::find_offline_candidates_by_quick_hash_and_size(&conn, "hash1", 1000).unwrap();
    let ids: Vec<&str> = candidates.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, vec!["earlier", "later"]);
}

#[test]
fn is_path_used_by_online_video_detects_a_different_online_owner() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    insert_test_video(
        &conn,
        "owner",
        "D:\\shared.mp4",
        "h",
        1,
        "online",
        None,
        "2026-01-01 00:00:00",
    );

    let collision =
        queries::is_path_used_by_online_video(&conn, "D:\\shared.mp4", "someone-else").unwrap();
    assert_eq!(collision, Some("owner".to_string()));
}

#[test]
fn is_path_used_by_online_video_ignores_the_excluded_id_and_offline_owners() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    insert_test_video(
        &conn,
        "self",
        "D:\\a.mp4",
        "h",
        1,
        "online",
        None,
        "2026-01-01 00:00:00",
    );
    insert_test_video(
        &conn,
        "offline-owner",
        "D:\\b.mp4",
        "h",
        1,
        "offline",
        None,
        "2026-01-01 00:00:01",
    );

    assert!(
        queries::is_path_used_by_online_video(&conn, "D:\\a.mp4", "self")
            .unwrap()
            .is_none(),
        "excluding the row's own id must not report a collision with itself"
    );
    assert!(
        queries::is_path_used_by_online_video(&conn, "D:\\b.mp4", "irrelevant")
            .unwrap()
            .is_none(),
        "an offline owner must not count as a collision"
    );
}

#[test]
fn update_video_status_flips_status() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    insert_test_video(
        &conn,
        "v1",
        "D:\\a.mp4",
        "h",
        1,
        "online",
        None,
        "2026-01-01 00:00:00",
    );

    queries::update_video_status(&conn, "v1", "offline").unwrap();

    let status: String = conn
        .query_row("SELECT status FROM videos WHERE id = 'v1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(status, "offline");
}

#[test]
fn update_video_path_and_status_rewrites_path_and_preserves_id() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    insert_test_video(
        &conn,
        "v1",
        "D:\\old\\a.mp4",
        "h",
        1,
        "offline",
        None,
        "2026-01-01 00:00:00",
    );

    queries::update_video_path_and_status(&conn, "v1", "E:\\new\\a.mp4", "a.mp4", "online")
        .unwrap();

    let (path, name, status): (String, String, String) = conn
        .query_row(
            "SELECT file_path, file_name, status FROM videos WHERE id = 'v1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(path, "E:\\new\\a.mp4");
    assert_eq!(name, "a.mp4");
    assert_eq!(status, "online");
}

#[test]
fn update_video_path_and_status_returns_err_not_panic_on_unique_violation() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    insert_test_video(
        &conn,
        "offline-row",
        "D:\\old\\a.mp4",
        "h",
        1,
        "offline",
        None,
        "2026-01-01 00:00:00",
    );
    insert_test_video(
        &conn,
        "online-row",
        "D:\\taken.mp4",
        "h2",
        2,
        "online",
        None,
        "2026-01-01 00:00:01",
    );

    // Caller is expected to pre-check via is_path_used_by_online_video before
    // calling this -- this test proves the fallback safety net also holds:
    // a UNIQUE violation surfaces as a recoverable Err, never a panic.
    let result = queries::update_video_path_and_status(
        &conn,
        "offline-row",
        "D:\\taken.mp4",
        "taken.mp4",
        "online",
    );
    assert!(result.is_err());

    // And the offline row must be left untouched by the failed write.
    let (path, status): (String, String) = conn
        .query_row(
            "SELECT file_path, status FROM videos WHERE id = 'offline-row'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(path, "D:\\old\\a.mp4");
    assert_eq!(status, "offline");
}

#[test]
fn list_online_videos_under_filters_by_prefix_case_insensitively() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    insert_test_video(
        &conn,
        "inside",
        "D:\\Videos\\sub\\a.mp4",
        "h",
        1,
        "online",
        None,
        "2026-01-01 00:00:00",
    );
    insert_test_video(
        &conn,
        "outside",
        "E:\\other\\b.mp4",
        "h",
        1,
        "online",
        None,
        "2026-01-01 00:00:01",
    );
    insert_test_video(
        &conn,
        "inside-but-offline",
        "D:\\videos\\sub\\c.mp4",
        "h",
        1,
        "offline",
        None,
        "2026-01-01 00:00:02",
    );
    drop(conn);

    let rows = queries::list_online_videos_under(&db.read_pool, "d:\\videos\\").unwrap();
    let ids: Vec<&str> = rows.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, vec!["inside"]);
}

#[test]
fn update_video_scan_metadata_updates_size_mtime_and_hash() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    insert_test_video(
        &conn,
        "v1",
        "D:\\a.mp4",
        "old-hash",
        100,
        "online",
        Some(1),
        "2026-01-01 00:00:00",
    );

    queries::update_video_scan_metadata(&conn, "v1", 200, 999, "new-hash").unwrap();

    let row = queries::find_video_by_path(&conn, "D:\\a.mp4")
        .unwrap()
        .unwrap();
    assert_eq!(row.file_size, 200);
    assert_eq!(row.mtime, Some(999));
    assert_eq!(row.quick_hash, "new-hash");
}

#[test]
fn nas_poll_interval_defaults_when_unset_and_round_trips_when_set() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();

    assert_eq!(queries::get_nas_poll_interval_secs(&conn).unwrap(), 600);

    queries::set_nas_poll_interval_secs(&conn, 120).unwrap();
    assert_eq!(queries::get_nas_poll_interval_secs(&conn).unwrap(), 120);
}
