//! Verifies (via `EXPLAIN QUERY PLAN`) that the sort-order indexes added in
//! migration 0005 are actually used by SQLite's query planner for the sort
//! orders they exist to support, against a real tempdir-backed SQLite file
//! with enough rows that the planner wouldn't just prefer a full scan anyway.
//! Policy: an index not shown to be
//! used here must not ship in 0005_add_sort_indexes.sql.

mod common;

use common::{init_temp_db, video};
use gb_core::sort::{order_by_clause, SortDirection, SortField};
use graybrowser_lib::db;
use rusqlite::Connection;

const ROW_COUNT: i64 = 2000;

fn init_temp_db_with_rows() -> (tempfile::TempDir, db::Db) {
    let (dir, db) = init_temp_db();
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
    let quick_hash = format!("hash{i}");
    let rating = i % 6; // spread across 0..=5
                        // Every third row has no mtime, to exercise the NULL-handling sort.
    let mtime: Option<i64> = if i % 3 == 0 {
        None
    } else {
        Some(1_700_000_000 + i)
    };
    let created_at = format!("2026-01-01 00:{:02}:{:02}", (i / 60) % 60, i % 60);
    video(&id, &file_path)
        .quick_hash(&quick_hash)
        .rating(rating)
        .mtime(mtime)
        .created_at(&created_at)
        .insert(conn);
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
    let clause = order_by_clause(SortField::CreatedAt, SortDirection::Desc);
    let plan = query_plan(&conn, &format!("SELECT id FROM videos ORDER BY {clause}"));
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
    let clause = order_by_clause(SortField::Rating, SortDirection::Desc);
    let plan = query_plan(&conn, &format!("SELECT id FROM videos ORDER BY {clause}"));
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
    let clause = order_by_clause(SortField::UpdatedDate, SortDirection::Desc);
    let plan = query_plan(&conn, &format!("SELECT id FROM videos ORDER BY {clause}"));
    assert!(
        plan.contains("USE TEMP B-TREE FOR ORDER BY"),
        "expected no index to back this NULL-pinning compound sort (documented finding), got plan:\n{plan}"
    );

    let mut stmt = conn
        .prepare(&format!("SELECT mtime FROM videos ORDER BY {clause}"))
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

/// "ファイルサイズ" and "フォルダ名" have no index: this pins that (both fall
/// back to a full scan + temp b-tree sort) using the exact clause production
/// builds. The decision not to add one rests on the measured cost in
/// `search_performance_bench.rs`; if an index is added later, this test must
/// be updated to assert it is actually used instead.
#[test]
fn sort_by_file_size_and_folder_have_no_index() {
    let (_dir, db) = init_temp_db_with_rows();
    let conn = db.writer.lock().unwrap();
    for field in [SortField::FileSize, SortField::Folder] {
        for direction in [SortDirection::Asc, SortDirection::Desc] {
            let clause = order_by_clause(field, direction);
            let plan = query_plan(&conn, &format!("SELECT id FROM videos ORDER BY {clause}"));
            assert!(
                plan.contains("USE TEMP B-TREE FOR ORDER BY"),
                "expected no index to back ORDER BY {clause}, got plan:
{plan}"
            );
        }
    }
}

#[test]
fn online_listing_uses_the_status_index() {
    let (_dir, db) = init_temp_db_with_rows();
    let conn = db.writer.lock().unwrap();
    let plan = query_plan(&conn, "SELECT id FROM videos WHERE status = 'online'");
    assert!(
        plan.contains("idx_videos_status"),
        "expected idx_videos_status to serve the status filter, got plan:\n{plan}"
    );
}

#[test]
fn folder_filter_like_cannot_use_a_path_index() {
    // LIKE is case-insensitive while the file_path indexes use BINARY
    // collation, so no index narrows the scan; this is why no separate
    // file_path index earns its keep beyond the one UNIQUE creates.
    let (_dir, db) = init_temp_db_with_rows();
    let conn = db.writer.lock().unwrap();
    let plan = query_plan(
        &conn,
        "SELECT id FROM videos WHERE file_path LIKE 'D:/videos/%' ESCAPE '\\'",
    );
    assert!(
        !plan.contains("SEARCH") && !plan.contains("file_path>"),
        "LIKE must not be index-bounded, got plan:\n{plan}"
    );
}
