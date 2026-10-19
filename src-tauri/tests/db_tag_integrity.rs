//! Integration tests for the `video_tags` write path and its app-layer
//! referential-integrity guarantee: with
//! `PRAGMA foreign_keys` deliberately OFF, nothing in SQLite itself stops an
//! orphan `video_tags` row, so `assign_tag_to_video`/`delete_tag`/
//! `delete_video_cascade` must enforce it themselves. Real tempdir-backed
//! SQLite file, matching `db_queries.rs`'s established convention.

use graybrowser_lib::db::{self, queries};
use rusqlite::{params, Connection};

fn init_temp_db() -> (tempfile::TempDir, db::Db) {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");
    let db = db::init(&db_path).expect("db::init should succeed against a fresh tempdir file");
    (dir, db)
}

fn insert_test_video(conn: &Connection, id: &str, file_path: &str) {
    conn.execute(
        "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status)
         VALUES (?1, ?2, ?3, 1024, 'hash', 'online')",
        params![
            id,
            file_path,
            file_path.rsplit('\\').next().unwrap_or(file_path)
        ],
    )
    .unwrap();
}

/// Every `video_tags` row must reference a `tags.id` that still exists.
fn orphaned_video_tags_by_tag(conn: &Connection) -> Vec<(String, i64)> {
    let mut stmt = conn
        .prepare(
            "SELECT video_tags.video_id, video_tags.tag_id
             FROM video_tags LEFT JOIN tags ON tags.id = video_tags.tag_id
             WHERE tags.id IS NULL",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// Every `video_tags` row must reference a `videos.id` that still exists.
fn orphaned_video_tags_by_video(conn: &Connection) -> Vec<(String, i64)> {
    let mut stmt = conn
        .prepare(
            "SELECT video_tags.video_id, video_tags.tag_id
             FROM video_tags LEFT JOIN videos ON videos.id = video_tags.video_id
             WHERE videos.id IS NULL",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn assign_tag_to_video_creates_tag_and_video_tags_row() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_test_video(&conn, "v1", "D:\\videos\\a.mp4");
    }
    let mut conn = db.writer.lock().unwrap();
    let tag = queries::assign_tag_to_video(&mut conn, "v1", "action").unwrap();
    assert_eq!(tag.name, "action");

    let video_tags_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM video_tags WHERE video_id = 'v1' AND tag_id = ?1",
            params![tag.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(video_tags_count, 1);
}

#[test]
fn assign_tag_to_video_is_idempotent_for_the_same_video_and_tag() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_test_video(&conn, "v1", "D:\\videos\\a.mp4");
    }
    let mut conn = db.writer.lock().unwrap();
    queries::assign_tag_to_video(&mut conn, "v1", "action").unwrap();
    queries::assign_tag_to_video(&mut conn, "v1", "action").unwrap();

    let video_tags_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM video_tags", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        video_tags_count, 1,
        "re-adding the same tag must not duplicate the video_tags row"
    );

    let tags_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        tags_count, 1,
        "re-adding the same tag must not create a second tags row"
    );
}

#[test]
fn assign_tag_to_video_reuses_an_existing_tag_by_normalized_name() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_test_video(&conn, "v1", "D:\\videos\\a.mp4");
        insert_test_video(&conn, "v2", "D:\\videos\\b.mp4");
    }
    let mut conn = db.writer.lock().unwrap();
    let tag1 = queries::assign_tag_to_video(&mut conn, "v1", "  action  ").unwrap();
    let tag2 = queries::assign_tag_to_video(&mut conn, "v2", "action").unwrap();
    assert_eq!(
        tag1.id, tag2.id,
        "equivalent-after-normalization names should resolve to the same tag"
    );

    let tags_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tags_count, 1);
}

#[test]
fn assign_tag_to_video_rejects_a_nonexistent_video_and_leaves_tags_untouched() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    let result = queries::assign_tag_to_video(&mut conn, "does-not-exist", "action");
    assert!(matches!(
        result,
        Err(queries::TagMutationError::VideoNotFound { .. })
    ));

    let tags_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        tags_count, 0,
        "a rejected video_id must not leave behind an unreferenced tag row"
    );
}

#[test]
fn assign_tag_to_video_rejects_an_empty_tag_name() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_test_video(&conn, "v1", "D:\\videos\\a.mp4");
    }
    let mut conn = db.writer.lock().unwrap();
    let result = queries::assign_tag_to_video(&mut conn, "v1", "   ");
    assert!(matches!(
        result,
        Err(queries::TagMutationError::InvalidName(_))
    ));
}

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn seed_videos(conn: &Connection, names: &[&str]) {
    for n in names {
        insert_test_video(conn, n, &format!("D:\\videos\\{n}.mp4"));
    }
}

#[test]
fn bulk_assign_tags_every_video_and_creates_the_tag_once() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2", "v3"]);
    let out =
        queries::assign_tag_to_videos(&mut conn, &ids(&["v1", "v2", "v3"]), "action").unwrap();
    assert_eq!(out.tag.name, "action");
    assert_eq!(
        (out.newly_tagged, out.already_tagged, out.missing),
        (3, 0, 0)
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags"), 1);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 3);
    assert!(orphaned_video_tags_by_tag(&conn).is_empty());
    assert!(orphaned_video_tags_by_video(&conn).is_empty());
}

#[test]
fn bulk_assign_is_idempotent_and_reports_already_tagged() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2", "v3"]);
    queries::assign_tag_to_video(&mut conn, "v2", "action").unwrap();

    let out =
        queries::assign_tag_to_videos(&mut conn, &ids(&["v1", "v2", "v3"]), "action").unwrap();
    assert_eq!(
        (out.newly_tagged, out.already_tagged, out.missing),
        (2, 1, 0)
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 3);

    let again =
        queries::assign_tag_to_videos(&mut conn, &ids(&["v1", "v2", "v3"]), "action").unwrap();
    assert_eq!(
        (again.newly_tagged, again.already_tagged, again.missing),
        (0, 3, 0)
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 3);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags"), 1);
}

#[test]
fn bulk_assign_counts_unknown_ids_as_missing_and_tags_the_rest() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2"]);
    let out =
        queries::assign_tag_to_videos(&mut conn, &ids(&["v1", "gone", "v2"]), "action").unwrap();
    assert_eq!(
        (out.newly_tagged, out.already_tagged, out.missing),
        (2, 0, 1)
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 2);
    assert!(orphaned_video_tags_by_video(&conn).is_empty());
}

#[test]
fn bulk_assign_counts_duplicate_ids_once() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1"]);
    let out =
        queries::assign_tag_to_videos(&mut conn, &ids(&["v1", "v1", "gone", "gone"]), "a").unwrap();
    assert_eq!(
        (out.newly_tagged, out.already_tagged, out.missing),
        (1, 0, 1)
    );
}

#[test]
fn bulk_assign_with_an_invalid_name_changes_nothing() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2"]);
    for bad in ["", "   ", "\u{3000}"] {
        let result = queries::assign_tag_to_videos(&mut conn, &ids(&["v1", "v2"]), bad);
        assert!(matches!(
            result,
            Err(queries::TagMutationError::InvalidName(_))
        ));
    }
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 0);
}

#[test]
fn bulk_assign_rolls_back_everything_on_a_mid_batch_failure() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2", "v3"]);
    // Fails the insert for the second video, after the tag and the first
    // assignment were already written inside the open transaction.
    conn.execute(
        "CREATE TRIGGER fail_on_v2 BEFORE INSERT ON video_tags
         WHEN NEW.video_id = 'v2'
         BEGIN SELECT RAISE(ABORT, 'simulated failure'); END;",
        [],
    )
    .unwrap();
    let result = queries::assign_tag_to_videos(&mut conn, &ids(&["v1", "v2", "v3"]), "action");
    assert!(matches!(result, Err(queries::TagMutationError::Sqlite(_))));
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 0);
}

#[test]
fn bulk_assign_with_no_ids_tags_nothing() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1"]);
    let out = queries::assign_tag_to_videos(&mut conn, &[], "action").unwrap();
    assert_eq!(
        (out.newly_tagged, out.already_tagged, out.missing),
        (0, 0, 0)
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 0);
}

#[test]
fn bulk_assign_does_not_touch_other_videos_or_tags() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2", "v3"]);
    queries::assign_tag_to_video(&mut conn, "v3", "other").unwrap();
    queries::assign_tag_to_video(&mut conn, "v1", "other").unwrap();

    queries::assign_tag_to_videos(&mut conn, &ids(&["v1", "v2"]), "action").unwrap();

    let names_of = |id: &str| -> Vec<String> {
        queries::list_tags_for_video(&db.read_pool, id)
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect()
    };
    assert_eq!(names_of("v1"), vec!["action", "other"]);
    assert_eq!(names_of("v2"), vec!["action"]);
    assert_eq!(names_of("v3"), vec!["other"]);
}

#[test]
fn remove_tag_from_video_deletes_only_the_video_tags_row_tags_master_survives() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_test_video(&conn, "v1", "D:\\videos\\a.mp4");
    }
    let tag = {
        let mut conn = db.writer.lock().unwrap();
        queries::assign_tag_to_video(&mut conn, "v1", "action").unwrap()
    };

    let conn = db.writer.lock().unwrap();
    queries::remove_tag_from_video(&conn, "v1", tag.id).unwrap();

    let video_tags_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM video_tags", [], |r| r.get(0))
        .unwrap();
    assert_eq!(video_tags_count, 0);

    let tags_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tags WHERE id = ?1",
            params![tag.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        tags_count, 1,
        "removing a tag from a video must not delete the tags master row"
    );
}

/// The orphan-prevention regression test for tag deletion.
#[test]
fn delete_tag_removes_all_its_video_tags_rows_and_leaves_others_untouched() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_test_video(&conn, "v1", "D:\\videos\\a.mp4");
        insert_test_video(&conn, "v2", "D:\\videos\\b.mp4");
    }
    let (tag_action, tag_comedy) = {
        let mut conn = db.writer.lock().unwrap();
        let action = queries::assign_tag_to_video(&mut conn, "v1", "action").unwrap();
        queries::assign_tag_to_video(&mut conn, "v2", "action").unwrap();
        let comedy = queries::assign_tag_to_video(&mut conn, "v1", "comedy").unwrap();
        queries::assign_tag_to_video(&mut conn, "v2", "comedy").unwrap();
        (action, comedy)
    };
    // Fixture: 2 videos x 2 tags = 4 video_tags rows before deletion.
    {
        let conn = db.writer.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM video_tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 4);
    }

    {
        let mut conn = db.writer.lock().unwrap();
        queries::delete_tag(&mut conn, tag_action.id).unwrap();
    }

    let conn = db.writer.lock().unwrap();
    let remaining_action: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM video_tags WHERE tag_id = ?1",
            params![tag_action.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        remaining_action, 0,
        "deleted tag's video_tags rows must be gone"
    );

    let remaining_comedy: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM video_tags WHERE tag_id = ?1",
            params![tag_comedy.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        remaining_comedy, 2,
        "the other tag's video_tags rows must be untouched"
    );

    let tags_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tags WHERE id = ?1",
            params![tag_action.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tags_count, 0, "the deleted tag's own tags row must be gone");

    assert!(
        orphaned_video_tags_by_tag(&conn).is_empty(),
        "no video_tags row may reference a nonexistent tags.id after delete_tag"
    );
}

/// The orphan-prevention regression test for video deletion -- exercises
/// `delete_video_cascade` even though nothing calls it from a command/UI
/// yet.
#[test]
fn delete_video_cascade_removes_all_its_video_tags_rows_and_leaves_others_untouched() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_test_video(&conn, "v1", "D:\\videos\\a.mp4");
        insert_test_video(&conn, "v2", "D:\\videos\\b.mp4");
    }
    {
        let mut conn = db.writer.lock().unwrap();
        queries::assign_tag_to_video(&mut conn, "v1", "action").unwrap();
        queries::assign_tag_to_video(&mut conn, "v2", "action").unwrap();
        queries::assign_tag_to_video(&mut conn, "v1", "comedy").unwrap();
        queries::assign_tag_to_video(&mut conn, "v2", "comedy").unwrap();
    }
    {
        let conn = db.writer.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM video_tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 4);
    }

    {
        let mut conn = db.writer.lock().unwrap();
        queries::delete_video_cascade(&mut conn, "v1").unwrap();
    }

    let conn = db.writer.lock().unwrap();
    let remaining_v1: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM video_tags WHERE video_id = 'v1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        remaining_v1, 0,
        "deleted video's video_tags rows must be gone"
    );

    let remaining_v2: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM video_tags WHERE video_id = 'v2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        remaining_v2, 2,
        "the other video's video_tags rows must be untouched"
    );

    let videos_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM videos WHERE id = 'v1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        videos_count, 0,
        "the deleted video's own videos row must be gone"
    );

    assert!(
        orphaned_video_tags_by_video(&conn).is_empty(),
        "no video_tags row may reference a nonexistent videos.id after delete_video_cascade"
    );
}

#[test]
fn list_tags_for_video_returns_tags_sorted_by_name() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_test_video(&conn, "v1", "D:\\videos\\a.mp4");
    }
    {
        let mut conn = db.writer.lock().unwrap();
        queries::assign_tag_to_video(&mut conn, "v1", "zebra").unwrap();
        queries::assign_tag_to_video(&mut conn, "v1", "apple").unwrap();
    }
    let tags = queries::list_tags_for_video(&db.read_pool, "v1").unwrap();
    let names: Vec<&str> = tags.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["apple", "zebra"]);
}

#[test]
fn list_all_tags_returns_every_tag_regardless_of_assignment() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_test_video(&conn, "v1", "D:\\videos\\a.mp4");
    }
    let tag = {
        let mut conn = db.writer.lock().unwrap();
        queries::assign_tag_to_video(&mut conn, "v1", "action").unwrap()
    };
    {
        let conn = db.writer.lock().unwrap();
        queries::remove_tag_from_video(&conn, "v1", tag.id).unwrap();
    }

    let all_tags = queries::list_all_tags(&db.read_pool).unwrap();
    assert_eq!(
        all_tags.len(),
        1,
        "an unassigned-but-not-deleted tag must still be listed"
    );
    assert_eq!(all_tags[0].name, "action");
}

#[test]
fn set_rating_round_trips_and_clears_to_zero() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_test_video(&conn, "v1", "D:\\videos\\a.mp4");
    }
    let conn = db.writer.lock().unwrap();
    queries::set_rating(&conn, "v1", 5).unwrap();
    let rating: i64 = conn
        .query_row("SELECT rating FROM videos WHERE id = 'v1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(rating, 5);

    queries::set_rating(&conn, "v1", 0).unwrap();
    let cleared: i64 = conn
        .query_row("SELECT rating FROM videos WHERE id = 'v1'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(cleared, 0);
}
