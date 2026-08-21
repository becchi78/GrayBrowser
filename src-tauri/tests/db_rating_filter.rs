//! Integration tests for `queries::list_videos_filtered`'s `min_rating`
//! argument (the rating bar's filter), against a real tempdir-backed SQLite
//! file. Same structure as `db_folder_filter.rs`.

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
fn insert_video(conn: &Connection, id: &str, file_path: &str, rating: i64) {
    let file_name = file_path.rsplit('\\').next().unwrap_or(file_path);
    conn.execute(
        "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status, rating, created_at)
         VALUES (?1, ?2, ?3, 1024, 'hash', 'online', ?4, '2026-01-01 00:00:00')",
        params![id, file_path, file_name, rating],
    )
    .unwrap();
}

fn list_with_min_rating(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    min_rating: Option<u8>,
) -> Vec<String> {
    queries::list_videos_filtered(
        pool,
        &[],
        SortField::CreatedAt,
        SortDirection::Desc,
        &[],
        None,
        min_rating,
    )
    .unwrap()
    .into_iter()
    .map(|v| v.id)
    .collect()
}

#[test]
fn min_rating_none_returns_every_row_regardless_of_rating() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "unrated", r"C:\Videos\a.mp4", 0);
        insert_video(&conn, "rated_5", r"C:\Videos\b.mp4", 5);
    }

    let mut ids = list_with_min_rating(&db.read_pool, None);
    ids.sort();

    assert_eq!(ids, vec!["rated_5", "unrated"]);
}

#[test]
fn min_rating_filters_by_threshold() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "r1", r"C:\Videos\a.mp4", 1);
        insert_video(&conn, "r2", r"C:\Videos\b.mp4", 2);
        insert_video(&conn, "r3", r"C:\Videos\c.mp4", 3);
        insert_video(&conn, "r4", r"C:\Videos\d.mp4", 4);
        insert_video(&conn, "r5", r"C:\Videos\e.mp4", 5);
    }

    let mut ids = list_with_min_rating(&db.read_pool, Some(3));
    ids.sort();
    assert_eq!(ids, vec!["r3", "r4", "r5"]);

    let mut ids = list_with_min_rating(&db.read_pool, Some(5));
    ids.sort();
    assert_eq!(ids, vec!["r5"]);

    let mut ids = list_with_min_rating(&db.read_pool, Some(1));
    ids.sort();
    assert_eq!(ids, vec!["r1", "r2", "r3", "r4", "r5"]);
}

#[test]
fn min_rating_always_excludes_unrated_videos() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "unrated", r"C:\Videos\a.mp4", 0);
        insert_video(&conn, "rated_1", r"C:\Videos\b.mp4", 1);
    }

    // Even at the lowest possible threshold (>=1), the unrated (rating=0)
    // video must never appear.
    let ids = list_with_min_rating(&db.read_pool, Some(1));
    assert_eq!(ids, vec!["rated_1"]);
    assert!(
        !ids.contains(&"unrated".to_string()),
        "an unrated (rating=0) video must never match any min_rating filter"
    );
}

#[test]
fn min_rating_combines_with_search_tag_and_folder_filters_as_an_and_condition() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        // Satisfies all four conditions (search term "action", folder
        // C:\Videos, rating >= 4, and will get the "favorite" tag below).
        insert_video(&conn, "matches_all", r"C:\Videos\action_movie.mp4", 5);
        // Satisfies search term + folder + rating, but NOT the tag.
        insert_video(&conn, "missing_tag", r"C:\Videos\action_show.mp4", 5);
        // Satisfies search term + folder + tag, but rating is too low.
        insert_video(&conn, "low_rating", r"C:\Videos\action_clip.mp4", 2);
        // Satisfies search term + tag + rating, but wrong folder.
        insert_video(&conn, "wrong_folder", r"D:\Other\action_scene.mp4", 5);
        // Satisfies folder + tag + rating, but wrong search term.
        insert_video(&conn, "wrong_term", r"C:\Videos\comedy_bit.mp4", 5);
    }

    let tag = {
        let mut conn = db.writer.lock().unwrap();
        let tag = queries::assign_tag_to_video(&mut conn, "matches_all", "favorite").unwrap();
        queries::assign_tag_to_video(&mut conn, "low_rating", "favorite").unwrap();
        queries::assign_tag_to_video(&mut conn, "wrong_folder", "favorite").unwrap();
        queries::assign_tag_to_video(&mut conn, "wrong_term", "favorite").unwrap();
        tag
    };

    let terms = gb_core::search::parse_search_terms("action");
    let ids: Vec<String> = queries::list_videos_filtered(
        &db.read_pool,
        &terms,
        SortField::CreatedAt,
        SortDirection::Desc,
        &[tag.id],
        Some(r"C:\Videos"),
        Some(4),
    )
    .unwrap()
    .into_iter()
    .map(|v| v.id)
    .collect();

    assert_eq!(ids, vec!["matches_all"]);
}
