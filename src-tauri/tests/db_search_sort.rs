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
    queries::list_videos_filtered(pool, &terms, field, direction, tag_ids, None, &[])
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

fn insert_at(conn: &Connection, id: &str, file_path: &str, file_size: i64, rating: i64) {
    let file_name = file_path.rsplit('\\').next().unwrap();
    conn.execute(
        "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status, rating, created_at)
         VALUES (?1, ?2, ?3, ?4, 'hash', 'online', ?5, '2026-01-01 00:00:00')",
        params![id, file_path, file_name, file_size, rating],
    )
    .unwrap();
}

fn list_filtered(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    search: &str,
    field: SortField,
    direction: SortDirection,
    tag_ids: &[i64],
    folder: Option<&str>,
    ratings: &[u8],
) -> Vec<String> {
    let terms = gb_core::search::parse_search_terms(search);
    queries::list_videos_filtered(pool, &terms, field, direction, tag_ids, folder, ratings)
        .unwrap()
        .into_iter()
        .map(|v| v.id)
        .collect()
}

#[test]
fn sort_by_file_size_orders_by_size_then_file_name() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        // Sizes are chosen so that string order ("10" < "9") differs from
        // numeric order, and two rows share a size to exercise the tiebreak.
        insert_at(&conn, "big", "D:\\videos\\m.mp4", 10_000_000_000, 0);
        insert_at(&conn, "small", "D:\\videos\\n.mp4", 9, 0);
        insert_at(&conn, "mid_b", "D:\\videos\\b.mp4", 500, 0);
        insert_at(&conn, "mid_a", "D:\\videos\\z.mp4", 500, 0);
        insert_at(&conn, "mid_c", "D:\\videos\\a.mp4", 500, 0);
    }
    let asc = list(
        &db.read_pool,
        "",
        SortField::FileSize,
        SortDirection::Asc,
        &[],
    );
    assert_eq!(asc, vec!["small", "mid_c", "mid_b", "mid_a", "big"]);
    // The tiebreak stays file_name ASC even when the size direction flips.
    let desc = list(
        &db.read_pool,
        "",
        SortField::FileSize,
        SortDirection::Desc,
        &[],
    );
    assert_eq!(desc, vec!["big", "mid_c", "mid_b", "mid_a", "small"]);
}

fn insert_folder_fixture(conn: &Connection) {
    insert_at(conn, "root_z", "C:\\videos\\z.mp4", 1, 0);
    insert_at(conn, "a_x", "C:\\videos\\a\\x.mp4", 1, 0);
    insert_at(conn, "a_b", "C:\\videos\\a\\b.mp4", 1, 0);
    insert_at(conn, "sub_c", "C:\\videos\\a\\sub\\c.mp4", 1, 0);
    // Same folder name "a" under another drive: must not merge with C:\\videos\\a.
    insert_at(conn, "d_a", "D:\\videos\\a\\a.mp4", 1, 0);
}

#[test]
fn sort_by_folder_groups_by_full_directory_then_file_name() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_folder_fixture(&conn);
    }
    // Ordering by file_name alone would give d_a, a_b, sub_c, a_x, root_z, so
    // this order can only come from the directory part.
    let asc = list(
        &db.read_pool,
        "",
        SortField::Folder,
        SortDirection::Asc,
        &[],
    );
    assert_eq!(asc, vec!["root_z", "a_b", "a_x", "sub_c", "d_a"]);
    let desc = list(
        &db.read_pool,
        "",
        SortField::Folder,
        SortDirection::Desc,
        &[],
    );
    assert_eq!(desc, vec!["d_a", "sub_c", "a_b", "a_x", "root_z"]);
}

#[test]
fn new_sort_fields_combine_with_search_tag_folder_and_rating_filters() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_at(&conn, "keep_big", "C:\\videos\\a\\clip_b.mp4", 900, 4);
        insert_at(&conn, "keep_small", "C:\\videos\\a\\clip_a.mp4", 100, 4);
        insert_at(&conn, "keep_sub", "C:\\videos\\a\\sub\\clip_c.mp4", 500, 4);
        insert_at(&conn, "other_rating", "C:\\videos\\a\\clip_d.mp4", 300, 2);
        insert_at(&conn, "other_name", "C:\\videos\\a\\movie.mp4", 700, 4);
        insert_at(&conn, "other_folder", "D:\\videos\\clip_e.mp4", 200, 4);
    }
    {
        let mut conn = db.writer.lock().unwrap();
        for id in [
            "keep_big",
            "keep_small",
            "keep_sub",
            "other_rating",
            "other_folder",
        ] {
            queries::assign_tag_to_video(&mut conn, id, "fav").unwrap();
        }
        queries::assign_tag_to_video(&mut conn, "other_name", "else").unwrap();
    }
    let fav = {
        let conn = db.writer.lock().unwrap();
        conn.query_row("SELECT id FROM tags WHERE name = 'fav'", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
    };
    let by_size = list_filtered(
        &db.read_pool,
        "clip",
        SortField::FileSize,
        SortDirection::Desc,
        &[fav],
        Some("C:\\videos\\a"),
        &[4],
    );
    assert_eq!(by_size, vec!["keep_big", "keep_sub", "keep_small"]);
    let by_folder = list_filtered(
        &db.read_pool,
        "clip",
        SortField::Folder,
        SortDirection::Desc,
        &[fav],
        Some("C:\\videos\\a"),
        &[4],
    );
    assert_eq!(by_folder, vec!["keep_sub", "keep_small", "keep_big"]);
}
