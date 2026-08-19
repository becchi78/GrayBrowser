//! Verifies (via `EXPLAIN QUERY PLAN`) that the sort-order indexes added in
//! migration 0005 are actually used by SQLite's query planner for the sort
//! orders they exist to support, against a real tempdir-backed SQLite file
//! with enough rows that the planner wouldn't just prefer a full scan anyway.
//! Policy: an index not shown to be
//! used here must not ship in 0005_add_sort_indexes.sql.

use graybrowser_lib::db;
use rusqlite::{params, Connection};

const ROW_COUNT: i64 = 2000;

fn init_temp_db_with_rows() -> (tempfile::TempDir, db::Db) {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");
    let db = db::init(&db_path).expect("db::init should succeed against a fresh tempdir file");
    {
        let conn = db.writer.lock().unwrap();
        for i in 0..ROW_COUNT {
            insert_synthetic_video(&conn, i);
        }
    }
    (dir, db)
}

fn insert_synthetic_video(conn: &Connection, i: i64) {
    let id = format!("v{i}");
    let file_path = format!("D:\\videos\\video_{i}.mp4");
    let file_name = format!("video_{i}.mp4");
    let quick_hash = format!("hash{i}");
    let rating = i % 6; // spread across 0..=5
                        // Every third row has no mtime, to exercise the NULL-handling sort.
    let mtime: Option<i64> = if i % 3 == 0 {
        None
    } else {
        Some(1_700_000_000 + i)
    };
    let created_at = format!("2026-01-01 00:{:02}:{:02}", (i / 60) % 60, i % 60);
    conn.execute(
        "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status, rating, mtime, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'online', ?6, ?7, ?8)",
        params![id, file_path, file_name, 1024_i64, quick_hash, rating, mtime, created_at],
    )
    .unwrap();
}

fn query_plan(conn: &Connection, sql: &str) -> String {
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let lines: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    lines.join("\n")
}

#[test]
fn sort_by_created_at_uses_its_index_instead_of_a_temp_sort() {
    let (_dir, db) = init_temp_db_with_rows();
    let conn = db.writer.lock().unwrap();
    let plan = query_plan(&conn, "SELECT id FROM videos ORDER BY created_at DESC");
    assert!(
        plan.contains("USING INDEX idx_videos_created_at"),
        "expected idx_videos_created_at to back this sort, got plan:\n{plan}"
    );
    assert!(
        !plan.contains("USE TEMP B-TREE FOR ORDER BY"),
        "index should make a temp sort unnecessary, got plan:\n{plan}"
    );
}

#[test]
fn sort_by_rating_uses_its_index_instead_of_a_temp_sort() {
    let (_dir, db) = init_temp_db_with_rows();
    let conn = db.writer.lock().unwrap();
    let plan = query_plan(&conn, "SELECT id FROM videos ORDER BY rating DESC");
    assert!(
        plan.contains("USING INDEX idx_videos_rating"),
        "expected idx_videos_rating to back this sort, got plan:\n{plan}"
    );
    assert!(
        !plan.contains("USE TEMP B-TREE FOR ORDER BY"),
        "index should make a temp sort unnecessary, got plan:\n{plan}"
    );
}

/// "更新日" sort maps to filesystem mtime, not a new
/// updated_at column. NULL rows (never scanned/reconciled)
/// must sort to a fixed, deterministic position -- last, regardless of
/// direction -- rather than interleaving with real values.
///
/// No `idx_videos_mtime` index exists (see 0005_add_sort_indexes.sql's
/// comment): this query plan is asserted to fall back to a full scan + temp
/// b-tree sort, confirming that finding rather than silently regressing on
/// it if a future change makes the planner's behavior here uncertain again.
#[test]
fn sort_by_mtime_with_nulls_pinned_last() {
    let (_dir, db) = init_temp_db_with_rows();
    let conn = db.writer.lock().unwrap();
    let plan = query_plan(
        &conn,
        "SELECT id FROM videos ORDER BY mtime IS NULL, mtime DESC",
    );
    assert!(
        plan.contains("USE TEMP B-TREE FOR ORDER BY"),
        "expected no index to back this NULL-pinning compound sort (documented finding), got plan:\n{plan}"
    );

    let mut stmt = conn
        .prepare("SELECT mtime FROM videos ORDER BY mtime IS NULL, mtime DESC")
        .unwrap();
    let mtimes: Vec<Option<i64>> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let first_null_pos = mtimes.iter().position(Option::is_none);
    let last_non_null_pos = mtimes.iter().rposition(Option::is_some);
    if let (Some(first_null), Some(last_non_null)) = (first_null_pos, last_non_null_pos) {
        assert!(
            first_null > last_non_null,
            "all NULL mtimes must sort after every non-NULL mtime"
        );
    }
}
