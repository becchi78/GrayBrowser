//! Integration tests for `queries::list_videos_filtered`'s `ratings`
//! argument (the rating bar's filter, an exact match against any listed value), against a real
//! tempdir-backed SQLite file. Same structure as `db_folder_filter.rs`.

mod common;

use common::{init_temp_db, video};
use gb_core::sort::{SortDirection, SortField};
use graybrowser_lib::db::queries;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;

fn insert_rated(conn: &Connection, id: &str, file_path: &str, rating: i64) {
    video(id, file_path).rating(rating).insert(conn);
}

fn list_with_rating(pool: &r2d2::Pool<SqliteConnectionManager>, ratings: &[u8]) -> Vec<String> {
    queries::list_videos_filtered(
        pool,
        &[],
        SortField::CreatedAt,
        SortDirection::Desc,
        &[],
        None,
        ratings,
    )
    .unwrap()
    .into_iter()
    .map(|v| v.id)
    .collect()
}

#[test]
fn empty_ratings_returns_every_row_regardless_of_rating() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_rated(&conn, "unrated", r"C:\Videos\a.mp4", 0);
        insert_rated(&conn, "rated_5", r"C:\Videos\b.mp4", 5);
    }

    let mut ids = list_with_rating(&db.read_pool, &[]);
    ids.sort();

    assert_eq!(ids, vec!["rated_5", "unrated"]);
}

#[test]
fn rating_filters_by_exact_match() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_rated(&conn, "unrated", r"C:\Videos\u.mp4", 0);
        insert_rated(&conn, "r1", r"C:\Videos\a.mp4", 1);
        insert_rated(&conn, "r2", r"C:\Videos\b.mp4", 2);
        // A second video with the same rating: every one of them is returned.
        insert_rated(&conn, "r2_twin", r"C:\Videos\b2.mp4", 2);
        insert_rated(&conn, "r3", r"C:\Videos\c.mp4", 3);
        // No video is rated 4.
        insert_rated(&conn, "r5", r"C:\Videos\e.mp4", 5);
    }

    // Neither lower nor higher ratings may leak in, and the unrated
    // (rating=0) video never matches any selectable rating (1..=5).
    assert_eq!(list_with_rating(&db.read_pool, &[1]), vec!["r1"]);
    let mut rated_2 = list_with_rating(&db.read_pool, &[2]);
    rated_2.sort();
    assert_eq!(rated_2, vec!["r2", "r2_twin"]);
    assert_eq!(list_with_rating(&db.read_pool, &[3]), vec!["r3"]);
    assert!(list_with_rating(&db.read_pool, &[4]).is_empty());
    assert_eq!(list_with_rating(&db.read_pool, &[5]), vec!["r5"]);
}

#[test]
fn rating_always_excludes_unrated_videos() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_rated(&conn, "unrated", r"C:\Videos\a.mp4", 0);
        insert_rated(&conn, "rated_1", r"C:\Videos\b.mp4", 1);
        insert_rated(&conn, "rated_5", r"C:\Videos\c.mp4", 5);
    }

    assert_eq!(list_with_rating(&db.read_pool, &[1]), vec!["rated_1"]);
}

/// The rating filter is ANDed with the search, tag and folder filters, for a
/// single rating and for several (a video matching any listed rating).
#[test]
fn rating_combines_with_search_tag_and_folder_filters_as_an_and_condition() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        // Satisfies all four conditions (search term "action", folder
        // C:\Videos, rating 4 or 5, and will get the "favorite" tag below).
        insert_rated(&conn, "match_r4", r"C:\Videos\action_movie.mp4", 4);
        insert_rated(&conn, "match_r5", r"C:\Videos\action_epic.mp4", 5);
        // Satisfies search term + folder + rating, but NOT the tag.
        insert_rated(&conn, "missing_tag", r"C:\Videos\action_show.mp4", 4);
        // Satisfies search term + folder + tag, but the rating is lower.
        insert_rated(&conn, "low_rating", r"C:\Videos\action_clip.mp4", 2);
        // Satisfies search term + tag + rating, but wrong folder.
        insert_rated(&conn, "wrong_folder", r"D:\Other\action_scene.mp4", 4);
        // Satisfies folder + tag + rating, but wrong search term.
        insert_rated(&conn, "wrong_term", r"C:\Videos\comedy_bit.mp4", 4);
    }

    let tag = {
        let mut conn = db.writer.lock().unwrap();
        let tag = queries::assign_tag_to_video(&mut conn, "match_r4", "favorite").unwrap();
        for id in ["match_r5", "low_rating", "wrong_folder", "wrong_term"] {
            queries::assign_tag_to_video(&mut conn, id, "favorite").unwrap();
        }
        tag
    };

    let terms = gb_core::search::parse_search_terms("action");
    let list = |ratings: &[u8]| -> Vec<String> {
        let mut ids: Vec<String> = queries::list_videos_filtered(
            &db.read_pool,
            &terms,
            SortField::CreatedAt,
            SortDirection::Desc,
            &[tag.id],
            Some(r"C:\Videos"),
            ratings,
        )
        .unwrap()
        .into_iter()
        .map(|v| v.id)
        .collect();
        ids.sort();
        ids
    };

    // An exact match: the rating-5 video must not leak into a filter on 4.
    assert_eq!(list(&[4]), vec!["match_r4"]);
    assert_eq!(list(&[4, 5]), vec!["match_r4", "match_r5"]);
}

#[test]
fn multiple_ratings_match_any_of_them() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_rated(&conn, "unrated", r"C:\Videos\u.mp4", 0);
        insert_rated(&conn, "r1", r"C:\Videos\a.mp4", 1);
        insert_rated(&conn, "r2", r"C:\Videos\b.mp4", 2);
        insert_rated(&conn, "r3", r"C:\Videos\c.mp4", 3);
    }

    let mut ids = list_with_rating(&db.read_pool, &[1, 2]);
    ids.sort();
    assert_eq!(ids, vec!["r1", "r2"]);

    // Order of the requested values must not matter.
    let mut reversed = list_with_rating(&db.read_pool, &[2, 1]);
    reversed.sort();
    assert_eq!(reversed, vec!["r1", "r2"]);
}

#[test]
fn duplicate_ratings_behave_like_a_single_value() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_rated(&conn, "r1", r"C:\Videos\a.mp4", 1);
        insert_rated(&conn, "r2", r"C:\Videos\b.mp4", 2);
    }

    assert_eq!(list_with_rating(&db.read_pool, &[2, 2, 2]), vec!["r2"]);
}
