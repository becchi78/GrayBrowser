//! Integration tests for the search/sort query layer
//! (`queries::list_videos_filtered`), against a real tempdir-backed SQLite
//! file.

use gb_core::sort::{SortDirection, SortField};
use graybrowser_lib::db::{self, queries};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection};

fn init_temp_db() -> (tempfile::TempDir, db::Db) {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");
    let db = db::init(&db_path).expect("db::init should succeed against a fresh tempdir file");
    (dir, db)
}

#[allow(clippy::too_many_arguments)]
fn insert_video_full(
    conn: &Connection,
    id: &str,
    file_name: &str,
    kana: Option<&str>,
    roma: Option<&str>,
    rating: i64,
    mtime: Option<i64>,
    created_at: &str,
) {
    conn.execute(
        "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status, kana, roma, rating, mtime, created_at)
         VALUES (?1, ?2, ?3, 1024, 'hash', 'online', ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            format!("D:\\videos\\{file_name}"),
            file_name,
            kana,
            roma,
            rating,
            mtime,
            created_at
        ],
    )
    .unwrap();
}

fn list(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    search: &str,
    field: SortField,
    direction: SortDirection,
    tag_ids: &[i64],
) -> Vec<String> {
    let terms = gb_core::search::parse_search_terms(search);
    queries::list_videos_filtered(pool, &terms, field, direction, tag_ids, None)
        .unwrap()
        .into_iter()
        .map(|v| v.id)
        .collect()
}

#[test]
fn empty_search_returns_everything_in_default_order() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video_full(
            &conn,
            "v1",
            "a.mp4",
            None,
            None,
            0,
            None,
            "2026-01-01 00:00:01",
        );
        insert_video_full(
            &conn,
            "v2",
            "b.mp4",
            None,
            None,
            0,
            None,
            "2026-01-01 00:00:02",
        );
    }
    let ids = list(
        &db.read_pool,
        "",
        SortField::CreatedAt,
        SortDirection::Desc,
        &[],
    );
    assert_eq!(ids, vec!["v2", "v1"]);
}

#[test]
fn search_matches_file_name_substring() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video_full(
            &conn,
            "v1",
            "action_movie.mp4",
            None,
            None,
            0,
            None,
            "2026-01-01",
        );
        insert_video_full(&conn, "v2", "comedy.mp4", None, None, 0, None, "2026-01-01");
    }
    let ids = list(
        &db.read_pool,
        "action",
        SortField::CreatedAt,
        SortDirection::Desc,
        &[],
    );
    assert_eq!(ids, vec!["v1"]);
}

#[test]
fn search_matches_kana_and_roma_independently() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video_full(
            &conn,
            "v1",
            "movie1.mp4",
            Some("アクション"),
            None,
            0,
            None,
            "2026-01-01",
        );
        insert_video_full(
            &conn,
            "v2",
            "movie2.mp4",
            None,
            Some("comedy"),
            0,
            None,
            "2026-01-01",
        );
        insert_video_full(&conn, "v3", "movie3.mp4", None, None, 0, None, "2026-01-01");
    }
    assert_eq!(
        list(
            &db.read_pool,
            "アクション",
            SortField::CreatedAt,
            SortDirection::Desc,
            &[]
        ),
        vec!["v1"]
    );
    assert_eq!(
        list(
            &db.read_pool,
            "comedy",
            SortField::CreatedAt,
            SortDirection::Desc,
            &[]
        ),
        vec!["v2"]
    );
}

#[test]
fn null_kana_and_roma_never_match_and_never_crash() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video_full(&conn, "v1", "movie.mp4", None, None, 0, None, "2026-01-01");
    }
    let ids = list(
        &db.read_pool,
        "nonexistent",
        SortField::CreatedAt,
        SortDirection::Desc,
        &[],
    );
    assert!(ids.is_empty());
    let ids = list(
        &db.read_pool,
        "movie",
        SortField::CreatedAt,
        SortDirection::Desc,
        &[],
    );
    assert_eq!(ids, vec!["v1"]);
}

#[test]
fn multiple_terms_use_and_semantics() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video_full(
            &conn,
            "v1",
            "action_comedy.mp4",
            None,
            None,
            0,
            None,
            "2026-01-01",
        );
        insert_video_full(
            &conn,
            "v2",
            "action_only.mp4",
            None,
            None,
            0,
            None,
            "2026-01-01",
        );
    }
    let ids = list(
        &db.read_pool,
        "action comedy",
        SortField::CreatedAt,
        SortDirection::Desc,
        &[],
    );
    assert_eq!(ids, vec!["v1"]);
}

#[test]
fn search_escapes_like_metacharacters_in_file_names() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video_full(
            &conn,
            "v1",
            "100%_done.mp4",
            None,
            None,
            0,
            None,
            "2026-01-01",
        );
        insert_video_full(&conn, "v2", "other.mp4", None, None, 0, None, "2026-01-01");
    }
    // A literal '%' in the search term must not act as a wildcard matching
    // every row -- only the file that actually contains "100%" should hit.
    let ids = list(
        &db.read_pool,
        "100%",
        SortField::CreatedAt,
        SortDirection::Desc,
        &[],
    );
    assert_eq!(ids, vec!["v1"]);
    // Likewise a literal '_' must not act as a single-character wildcard.
    let ids = list(
        &db.read_pool,
        "100%_done",
        SortField::CreatedAt,
        SortDirection::Desc,
        &[],
    );
    assert_eq!(ids, vec!["v1"]);
}

#[test]
fn sort_by_each_field_and_direction() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video_full(&conn, "a", "b.mp4", None, None, 3, Some(200), "2026-01-02");
        insert_video_full(&conn, "b", "a.mp4", None, None, 5, Some(100), "2026-01-01");
    }
    assert_eq!(
        list(
            &db.read_pool,
            "",
            SortField::FileName,
            SortDirection::Asc,
            &[]
        ),
        vec!["b", "a"]
    );
    assert_eq!(
        list(
            &db.read_pool,
            "",
            SortField::FileName,
            SortDirection::Desc,
            &[]
        ),
        vec!["a", "b"]
    );
    assert_eq!(
        list(
            &db.read_pool,
            "",
            SortField::CreatedAt,
            SortDirection::Asc,
            &[]
        ),
        vec!["b", "a"]
    );
    assert_eq!(
        list(
            &db.read_pool,
            "",
            SortField::Rating,
            SortDirection::Desc,
            &[]
        ),
        vec!["b", "a"]
    );
    assert_eq!(
        list(
            &db.read_pool,
            "",
            SortField::UpdatedDate,
            SortDirection::Desc,
            &[]
        ),
        vec!["a", "b"]
    );
}

#[test]
fn updated_date_sort_pins_null_mtime_rows_last_regardless_of_direction() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video_full(
            &conn,
            "has_mtime",
            "a.mp4",
            None,
            None,
            0,
            Some(100),
            "2026-01-01",
        );
        insert_video_full(
            &conn,
            "no_mtime",
            "b.mp4",
            None,
            None,
            0,
            None,
            "2026-01-01",
        );
    }
    assert_eq!(
        list(
            &db.read_pool,
            "",
            SortField::UpdatedDate,
            SortDirection::Desc,
            &[]
        ),
        vec!["has_mtime", "no_mtime"]
    );
    assert_eq!(
        list(
            &db.read_pool,
            "",
            SortField::UpdatedDate,
            SortDirection::Asc,
            &[]
        ),
        vec!["has_mtime", "no_mtime"]
    );
}

#[test]
fn tag_filter_ands_across_selected_tags() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video_full(
            &conn,
            "v1",
            "a.mp4",
            None,
            None,
            0,
            None,
            "2026-01-01 00:00:02",
        );
        insert_video_full(
            &conn,
            "v2",
            "b.mp4",
            None,
            None,
            0,
            None,
            "2026-01-01 00:00:01",
        );
    }
    let (action_tag, comedy_tag) = {
        let mut conn = db.writer.lock().unwrap();
        let action = queries::assign_tag_to_video(&mut conn, "v1", "action").unwrap();
        queries::assign_tag_to_video(&mut conn, "v2", "action").unwrap();
        let comedy = queries::assign_tag_to_video(&mut conn, "v1", "comedy").unwrap();
        (action, comedy)
    };
    // Both videos carry "action" -- single-tag filter matches both, in
    // created_at DESC order.
    assert_eq!(
        list(
            &db.read_pool,
            "",
            SortField::CreatedAt,
            SortDirection::Desc,
            &[action_tag.id]
        ),
        vec!["v1", "v2"]
    );
    // Only v1 carries both "action" AND "comedy".
    let both = list(
        &db.read_pool,
        "",
        SortField::CreatedAt,
        SortDirection::Desc,
        &[action_tag.id, comedy_tag.id],
    );
    assert_eq!(both, vec!["v1"]);
}

#[test]
fn search_and_tag_filter_combine() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video_full(
            &conn,
            "v1",
            "action_movie.mp4",
            None,
            None,
            0,
            None,
            "2026-01-01",
        );
        insert_video_full(
            &conn,
            "v2",
            "action_show.mp4",
            None,
            None,
            0,
            None,
            "2026-01-01",
        );
    }
    let tag = {
        let mut conn = db.writer.lock().unwrap();
        queries::assign_tag_to_video(&mut conn, "v1", "favorite").unwrap()
    };
    let ids = list(
        &db.read_pool,
        "action",
        SortField::CreatedAt,
        SortDirection::Desc,
        &[tag.id],
    );
    assert_eq!(ids, vec!["v1"]);
}
