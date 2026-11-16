//! Integration tests for `queries::list_videos_filtered`'s `folder_path`
//! argument (used by the folder sidebar), against a
//! real tempdir-backed SQLite file.
//!
//! The behavior under test is folder-*boundary* safety: a naive
//! `file_path LIKE 'folder_path%'` would wrongly match a sibling folder
//! whose name merely starts with the same characters (`C:\Videos2\...` when
//! filtering on `C:\Videos`), and would also wrongly treat a literal `_`/`%`
//! inside a folder name as a `LIKE` wildcard. Both are fixed by
//! `gb_core::paths::folder_like_prefix` (unit-tested for the pattern string
//! itself in `crates/gb-core/src/paths.rs`); this file checks the fix
//! end-to-end through the real SQL `LIKE ... ESCAPE '\'` query.

mod common;

use common::{init_temp_db, insert_video};
use gb_core::sort::{SortDirection, SortField};
use graybrowser_lib::db::queries;
use r2d2_sqlite::SqliteConnectionManager;

fn list_under(pool: &r2d2::Pool<SqliteConnectionManager>, folder_path: &str) -> Vec<String> {
    queries::list_videos_filtered(
        pool,
        &[],
        SortField::CreatedAt,
        SortDirection::Desc,
        &[],
        Some(folder_path),
        &[],
    )
    .unwrap()
    .into_iter()
    .map(|v| v.id)
    .collect()
}

#[test]
fn folder_filter_does_not_match_a_sibling_folder_with_the_same_prefix() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "in_folder", r"C:\Videos\a.mp4");
        // Same string prefix as "C:\Videos" but a genuinely different,
        // sibling folder -- a naive `LIKE 'C:\Videos%'` would wrongly match
        // this too.
        insert_video(&conn, "sibling_folder", r"C:\Videos2\a.mp4");
    }

    let ids = list_under(&db.read_pool, r"C:\Videos");

    assert_eq!(ids, vec!["in_folder"]);
}

#[test]
fn folder_filter_treats_a_trailing_separator_on_the_filter_the_same_as_no_trailing_separator() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "v1", r"C:\Videos\a.mp4");
    }

    let without_sep = list_under(&db.read_pool, r"C:\Videos");
    let with_sep = list_under(&db.read_pool, r"C:\Videos\");

    assert_eq!(without_sep, vec!["v1"]);
    assert_eq!(with_sep, vec!["v1"]);
}

#[test]
fn folder_filter_escapes_a_literal_underscore_in_the_folder_name() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "underscore_folder", r"C:\my_videos\a.mp4");
        // If `_` were treated as the SQL LIKE single-character wildcard
        // (unescaped), this would also match `C:\my_videos\` and wrongly
        // appear in the result below.
        insert_video(&conn, "x_folder", r"C:\myXvideos\a.mp4");
    }

    let ids = list_under(&db.read_pool, r"C:\my_videos");

    assert_eq!(ids, vec!["underscore_folder"]);
}

#[test]
fn folder_filter_matches_nested_subfolders() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "direct_child", r"C:\Videos\a.mp4");
        insert_video(&conn, "nested_child", r"C:\Videos\Sub\Deep\b.mp4");
        insert_video(&conn, "outside", r"D:\Other\c.mp4");
    }

    let mut ids = list_under(&db.read_pool, r"C:\Videos");
    ids.sort();

    assert_eq!(ids, vec!["direct_child", "nested_child"]);
}
