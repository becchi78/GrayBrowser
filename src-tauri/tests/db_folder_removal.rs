//! Integration tests for `queries::count_videos_under_folder`/
//! `queries::delete_videos_under_folder_cascade` (the folder sidebar's
//! delete action), against a real tempdir-backed
//! SQLite file.
//!
//! Mirrors `db_folder_filter.rs`'s folder-boundary-safety fixtures and
//! helpers (same `gb_core::paths::folder_like_prefix` matching underlies
//! both), and additionally checks the orphan-prevention guarantee:
//! deleting a folder's videos must also
//! remove every `video_tags`/`path_collisions` row referencing one of them,
//! all inside a single transaction, without touching any other folder's
//! rows.

use graybrowser_lib::db::{self, queries};
use rusqlite::{params, Connection};

fn init_temp_db() -> (tempfile::TempDir, db::Db) {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");
    let db = db::init(&db_path).expect("db::init should succeed against a fresh tempdir file");
    (dir, db)
}

fn insert_video(conn: &Connection, id: &str, file_path: &str) {
    let file_name = file_path.rsplit('\\').next().unwrap_or(file_path);
    conn.execute(
        "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status, rating, created_at)
         VALUES (?1, ?2, ?3, 1024, 'hash', 'online', 0, '2026-01-01 00:00:00')",
        params![id, file_path, file_name],
    )
    .unwrap();
}

fn insert_tag(conn: &Connection, name: &str) -> i64 {
    conn.execute("INSERT INTO tags (name) VALUES (?1)", params![name])
        .unwrap();
    conn.last_insert_rowid()
}

fn assign_tag(conn: &Connection, video_id: &str, tag_id: i64) {
    conn.execute(
        "INSERT INTO video_tags (video_id, tag_id) VALUES (?1, ?2)",
        params![video_id, tag_id],
    )
    .unwrap();
}

fn insert_path_collision(conn: &Connection, video_id: &str, colliding_video_id: &str) {
    conn.execute(
        "INSERT INTO path_collisions (video_id, colliding_video_id, attempted_path)
         VALUES (?1, ?2, 'C:\\Videos\\attempted.mp4')",
        params![video_id, colliding_video_id],
    )
    .unwrap();
}

fn video_ids(conn: &Connection) -> Vec<String> {
    let mut stmt = conn.prepare("SELECT id FROM videos ORDER BY id").unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

#[test]
fn count_videos_under_folder_only_counts_that_folders_videos() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "in_folder_a", r"C:\Videos\a.mp4");
        insert_video(&conn, "in_folder_b", r"C:\Videos\Sub\b.mp4");
        insert_video(&conn, "sibling_folder", r"C:\Videos2\c.mp4");
        insert_video(&conn, "other_drive", r"D:\Other\d.mp4");
    }

    let count = queries::count_videos_under_folder(&db.read_pool, r"C:\Videos").unwrap();
    assert_eq!(count, 2, "must count only C:\\Videos and its subfolders");
}

#[test]
fn count_videos_under_folder_returns_zero_for_an_empty_folder() {
    let (_dir, db) = init_temp_db();
    let count = queries::count_videos_under_folder(&db.read_pool, r"C:\Empty").unwrap();
    assert_eq!(count, 0);
}

#[test]
fn delete_videos_under_folder_cascade_removes_videos_tags_and_path_collisions() {
    let (_dir, db) = init_temp_db();
    let deleted_ids = {
        let mut conn = db.writer.lock().unwrap();
        insert_video(&conn, "in_folder_a", r"C:\Videos\a.mp4");
        insert_video(&conn, "in_folder_b", r"C:\Videos\Sub\b.mp4");
        insert_video(&conn, "outside", r"D:\Other\c.mp4");

        let tag_id = insert_tag(&conn, "action");
        assign_tag(&conn, "in_folder_a", tag_id);
        assign_tag(&conn, "in_folder_b", tag_id);
        assign_tag(&conn, "outside", tag_id);

        // Both directions of path_collisions:
        // "in_folder_a" as the offline-side attempted-move row, and
        // "in_folder_b" as the online-side row another video collided into.
        insert_path_collision(&conn, "in_folder_a", "outside");
        insert_path_collision(&conn, "outside", "in_folder_b");

        let mut ids = queries::delete_videos_under_folder_cascade(&mut conn, r"C:\Videos").unwrap();
        ids.sort();
        ids
    };

    assert_eq!(deleted_ids, vec!["in_folder_a", "in_folder_b"]);

    let conn = db.writer.lock().unwrap();

    assert_eq!(
        video_ids(&conn),
        vec!["outside"],
        "only the outside video must remain"
    );

    let video_tags_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM video_tags", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        video_tags_count, 1,
        "only outside's own video_tags row must remain"
    );
    let remaining_video_tags_owner: String = conn
        .query_row("SELECT video_id FROM video_tags", [], |r| r.get(0))
        .unwrap();
    assert_eq!(remaining_video_tags_owner, "outside");

    let path_collisions_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM path_collisions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        path_collisions_count, 0,
        "every path_collisions row referencing a deleted video (from either \
         side) must be gone, even the one where the deleted video is only \
         the colliding_video_id side"
    );
}

#[test]
fn delete_videos_under_folder_cascade_does_not_touch_a_sibling_folder_with_the_same_prefix() {
    let (_dir, db) = init_temp_db();
    {
        let mut conn = db.writer.lock().unwrap();
        insert_video(&conn, "in_folder", r"C:\Videos\a.mp4");
        // Same string prefix as "C:\Videos" but a genuinely different,
        // sibling folder -- same boundary-safety fixture as
        // db_folder_filter.rs's folder_filter test.
        insert_video(&conn, "sibling_folder", r"C:\Videos2\a.mp4");

        let deleted_ids =
            queries::delete_videos_under_folder_cascade(&mut conn, r"C:\Videos").unwrap();
        assert_eq!(deleted_ids, vec!["in_folder"]);
    }

    let conn = db.writer.lock().unwrap();
    assert_eq!(
        video_ids(&conn),
        vec!["sibling_folder"],
        "C:\\Videos2\\a.mp4 must survive a delete scoped to C:\\Videos"
    );
}

#[test]
fn delete_videos_under_folder_cascade_is_a_no_op_for_an_already_empty_folder() {
    let (_dir, db) = init_temp_db();
    {
        let mut conn = db.writer.lock().unwrap();
        insert_video(&conn, "elsewhere", r"D:\Other\a.mp4");

        let deleted_ids =
            queries::delete_videos_under_folder_cascade(&mut conn, r"C:\Empty").unwrap();
        assert!(deleted_ids.is_empty());
    }

    let conn = db.writer.lock().unwrap();
    assert_eq!(video_ids(&conn), vec!["elsewhere"]);
}
