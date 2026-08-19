//! Integration tests for the DB layer's
//! `thumbnail_attempts`/`metadata_attempts` counters (migration 0007) and the
//! queries built on top of them (increment/reset, the attempts-carrying
//! listing variants, and the exhausted-attempts listings feeding the retry
//! UI). Real tempdir-backed SQLite file, matching `db_dedup_integrity.rs`'s
//! established convention.

use gb_core::retry::MAX_GENERATION_ATTEMPTS;
use graybrowser_lib::db::{self, queries};
use rusqlite::{params, Connection};

fn init_temp_db() -> (tempfile::TempDir, db::Db) {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");
    let db = db::init(&db_path).expect("db::init should succeed against a fresh tempdir file");
    (dir, db)
}

fn insert_video(conn: &Connection, id: &str, file_path: &str, status: &str) {
    conn.execute(
        "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status)
         VALUES (?1, ?2, ?3, 1024, 'qh', ?4)",
        params![
            id,
            file_path,
            file_path.rsplit('\\').next().unwrap_or(file_path),
            status,
        ],
    )
    .unwrap();
}

fn thumbnail_attempts(conn: &Connection, id: &str) -> i64 {
    conn.query_row(
        "SELECT thumbnail_attempts FROM videos WHERE id = ?1",
        [id],
        |r| r.get(0),
    )
    .unwrap()
}

fn metadata_attempts(conn: &Connection, id: &str) -> i64 {
    conn.query_row(
        "SELECT metadata_attempts FROM videos WHERE id = ?1",
        [id],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn a_freshly_inserted_video_starts_with_zero_attempts_of_either_kind() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    insert_video(&conn, "v1", "D:\\videos\\a.mp4", "online");

    assert_eq!(thumbnail_attempts(&conn, "v1"), 0);
    assert_eq!(metadata_attempts(&conn, "v1"), 0);
}

#[test]
fn increment_thumbnail_attempts_accumulates_across_repeated_calls() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "v1", "D:\\videos\\a.mp4", "online");
    }

    let conn = db.writer.lock().unwrap();
    for expected in 1..=3 {
        queries::increment_thumbnail_attempts(&conn, "v1").unwrap();
        assert_eq!(thumbnail_attempts(&conn, "v1"), expected);
    }
}

#[test]
fn increment_metadata_attempts_accumulates_across_repeated_calls() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "v1", "D:\\videos\\a.mp4", "online");
    }

    let conn = db.writer.lock().unwrap();
    for expected in 1..=3 {
        queries::increment_metadata_attempts(&conn, "v1").unwrap();
        assert_eq!(metadata_attempts(&conn, "v1"), expected);
    }
}

#[test]
fn increment_thumbnail_and_metadata_attempts_are_independent_counters() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "v1", "D:\\videos\\a.mp4", "online");
    }

    let conn = db.writer.lock().unwrap();
    queries::increment_thumbnail_attempts(&conn, "v1").unwrap();
    queries::increment_thumbnail_attempts(&conn, "v1").unwrap();
    queries::increment_metadata_attempts(&conn, "v1").unwrap();

    assert_eq!(thumbnail_attempts(&conn, "v1"), 2);
    assert_eq!(metadata_attempts(&conn, "v1"), 1);
}

#[test]
fn reset_thumbnail_attempts_returns_the_counter_to_zero() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "v1", "D:\\videos\\a.mp4", "online");
    }

    let conn = db.writer.lock().unwrap();
    queries::increment_thumbnail_attempts(&conn, "v1").unwrap();
    queries::increment_thumbnail_attempts(&conn, "v1").unwrap();
    assert_eq!(thumbnail_attempts(&conn, "v1"), 2);

    queries::reset_thumbnail_attempts(&conn, "v1").unwrap();
    assert_eq!(thumbnail_attempts(&conn, "v1"), 0);
}

#[test]
fn reset_metadata_attempts_returns_the_counter_to_zero() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "v1", "D:\\videos\\a.mp4", "online");
    }

    let conn = db.writer.lock().unwrap();
    queries::increment_metadata_attempts(&conn, "v1").unwrap();
    queries::increment_metadata_attempts(&conn, "v1").unwrap();
    assert_eq!(metadata_attempts(&conn, "v1"), 2);

    queries::reset_metadata_attempts(&conn, "v1").unwrap();
    assert_eq!(metadata_attempts(&conn, "v1"), 0);
}

#[test]
fn list_online_video_paths_with_thumbnail_attempts_returns_the_current_counter_and_excludes_offline(
) {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "online-1", "D:\\videos\\a.mp4", "online");
        insert_video(&conn, "offline-1", "D:\\videos\\b.mp4", "offline");
        queries::increment_thumbnail_attempts(&conn, "online-1").unwrap();
        queries::increment_thumbnail_attempts(&conn, "online-1").unwrap();
    }

    let rows = queries::list_online_video_paths_with_thumbnail_attempts(&db.read_pool).unwrap();
    assert_eq!(
        rows,
        vec![(
            "online-1".to_string(),
            "D:\\videos\\a.mp4".to_string(),
            2i64,
            false
        )]
    );
}

#[test]
fn list_online_video_paths_with_thumbnail_attempts_reports_the_current_ready_flag() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "ready-1", "D:\\videos\\a.mp4", "online");
        insert_video(&conn, "not-ready-1", "D:\\videos\\b.mp4", "online");
        queries::mark_thumbnail_ready(&conn, "ready-1").unwrap();
    }

    let mut rows = queries::list_online_video_paths_with_thumbnail_attempts(&db.read_pool).unwrap();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        rows,
        vec![
            (
                "not-ready-1".to_string(),
                "D:\\videos\\b.mp4".to_string(),
                0i64,
                false
            ),
            (
                "ready-1".to_string(),
                "D:\\videos\\a.mp4".to_string(),
                0i64,
                true
            ),
        ]
    );
}

#[test]
fn list_videos_missing_metadata_with_attempts_returns_the_current_counter_and_excludes_probed_and_offline(
) {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "unprobed", "D:\\videos\\a.mp4", "online");
        insert_video(&conn, "offline", "D:\\videos\\b.mp4", "offline");
        insert_video(&conn, "probed", "D:\\videos\\c.mp4", "online");
        conn.execute(
            "UPDATE videos SET probed_at = CURRENT_TIMESTAMP WHERE id = 'probed'",
            [],
        )
        .unwrap();
        queries::increment_metadata_attempts(&conn, "unprobed").unwrap();
    }

    let rows = queries::list_videos_missing_metadata_with_attempts(&db.read_pool).unwrap();
    assert_eq!(
        rows,
        vec![(
            "unprobed".to_string(),
            "D:\\videos\\a.mp4".to_string(),
            1i64
        )]
    );
}

#[test]
fn list_videos_with_exhausted_thumbnail_attempts_only_includes_rows_at_or_above_the_limit() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "below-limit", "D:\\videos\\below.mp4", "online");
        insert_video(&conn, "at-limit", "D:\\videos\\at.mp4", "online");
        insert_video(&conn, "above-limit", "D:\\videos\\above.mp4", "online");
        insert_video(
            &conn,
            "offline-at-limit",
            "D:\\videos\\offline.mp4",
            "offline",
        );

        // Boundary: MAX_GENERATION_ATTEMPTS - 1 must NOT be exhausted.
        for _ in 0..(MAX_GENERATION_ATTEMPTS - 1) {
            queries::increment_thumbnail_attempts(&conn, "below-limit").unwrap();
        }
        // Boundary: exactly MAX_GENERATION_ATTEMPTS must be exhausted.
        for _ in 0..MAX_GENERATION_ATTEMPTS {
            queries::increment_thumbnail_attempts(&conn, "at-limit").unwrap();
        }
        // Beyond the limit must also be exhausted.
        for _ in 0..(MAX_GENERATION_ATTEMPTS + 2) {
            queries::increment_thumbnail_attempts(&conn, "above-limit").unwrap();
        }
        for _ in 0..MAX_GENERATION_ATTEMPTS {
            queries::increment_thumbnail_attempts(&conn, "offline-at-limit").unwrap();
        }
    }

    let rows = queries::list_videos_with_exhausted_thumbnail_attempts(&db.read_pool).unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();

    assert!(
        !ids.contains(&"below-limit"),
        "attempts one below the limit must not be reported as exhausted"
    );
    assert!(
        ids.contains(&"at-limit"),
        "attempts exactly at the limit must be reported as exhausted"
    );
    assert!(
        ids.contains(&"above-limit"),
        "attempts beyond the limit must still be reported as exhausted"
    );
    assert!(
        !ids.contains(&"offline-at-limit"),
        "an offline video must never be reported, regardless of its attempts count"
    );
    assert_eq!(ids.len(), 2);

    let at_limit_row = rows.iter().find(|r| r.id == "at-limit").unwrap();
    assert_eq!(at_limit_row.file_path, "D:\\videos\\at.mp4");
    assert_eq!(at_limit_row.file_name, "at.mp4");
    assert_eq!(
        at_limit_row.thumbnail_attempts,
        i64::from(MAX_GENERATION_ATTEMPTS)
    );
}

#[test]
fn list_videos_with_exhausted_metadata_attempts_only_includes_rows_at_or_above_the_limit() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "below-limit", "D:\\videos\\below.mp4", "online");
        insert_video(&conn, "at-limit", "D:\\videos\\at.mp4", "online");
        insert_video(
            &conn,
            "offline-at-limit",
            "D:\\videos\\offline.mp4",
            "offline",
        );

        for _ in 0..(MAX_GENERATION_ATTEMPTS - 1) {
            queries::increment_metadata_attempts(&conn, "below-limit").unwrap();
        }
        for _ in 0..MAX_GENERATION_ATTEMPTS {
            queries::increment_metadata_attempts(&conn, "at-limit").unwrap();
        }
        for _ in 0..MAX_GENERATION_ATTEMPTS {
            queries::increment_metadata_attempts(&conn, "offline-at-limit").unwrap();
        }
    }

    let rows = queries::list_videos_with_exhausted_metadata_attempts(&db.read_pool).unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();

    assert!(
        !ids.contains(&"below-limit"),
        "attempts one below the limit must not be reported as exhausted"
    );
    assert!(
        ids.contains(&"at-limit"),
        "attempts exactly at the limit must be reported as exhausted"
    );
    assert!(
        !ids.contains(&"offline-at-limit"),
        "an offline video must never be reported, regardless of its attempts count"
    );
    assert_eq!(ids.len(), 1);

    let at_limit_row = rows.iter().find(|r| r.id == "at-limit").unwrap();
    assert_eq!(at_limit_row.file_path, "D:\\videos\\at.mp4");
    assert_eq!(at_limit_row.file_name, "at.mp4");
    assert_eq!(
        at_limit_row.metadata_attempts,
        i64::from(MAX_GENERATION_ATTEMPTS)
    );
}
