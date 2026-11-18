//! Integration tests for the `video_tags` write path and its app-layer
//! referential-integrity guarantee: with
//! `PRAGMA foreign_keys` deliberately OFF, nothing in SQLite itself stops an
//! orphan `video_tags` row, so `assign_tag_to_video`/`delete_tag`/
//! `delete_videos_cascade` must enforce it themselves. Real tempdir-backed
//! SQLite file, matching `db_queries.rs`'s established convention.

mod common;

use common::{init_temp_db, insert_video};
use graybrowser_lib::db::queries;
use rusqlite::{params, Connection};

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
        insert_video(&conn, "v1", "D:\\videos\\a.mp4");
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
        insert_video(&conn, "v1", "D:\\videos\\a.mp4");
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
        insert_video(&conn, "v1", "D:\\videos\\a.mp4");
        insert_video(&conn, "v2", "D:\\videos\\b.mp4");
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
        insert_video(&conn, "v1", "D:\\videos\\a.mp4");
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
        insert_video(conn, n, &format!("D:\\videos\\{n}.mp4"));
    }
}

/// Single-tag convenience over the plural API, for tests that only need the row.
fn assign_one(conn: &mut Connection, video_ids: &[&str], name: &str) -> queries::TagRow {
    queries::assign_tags_to_videos(conn, &ids(video_ids), &ids(&[name]))
        .unwrap()
        .remove(0)
}

fn names_of(db: &graybrowser_lib::db::Db, id: &str) -> Vec<String> {
    queries::list_tags_for_video(&db.read_pool, id)
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect()
}

#[test]
fn bulk_assign_tags_every_video_and_creates_the_tag_once() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2", "v3"]);
    let tags =
        queries::assign_tags_to_videos(&mut conn, &ids(&["v1", "v2", "v3"]), &ids(&["action"]))
            .unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].name, "action");
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags"), 1);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 3);
    assert!(orphaned_video_tags_by_tag(&conn).is_empty());
    assert!(orphaned_video_tags_by_video(&conn).is_empty());
}

#[test]
fn bulk_assign_puts_every_tag_on_every_video() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2", "v3"]);
    let tags = queries::assign_tags_to_videos(
        &mut conn,
        &ids(&["v1", "v2"]),
        &ids(&["action", "comedy", "drama"]),
    )
    .unwrap();
    assert_eq!(
        tags.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        vec!["action", "comedy", "drama"]
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags"), 3);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 6);
    drop(conn);
    assert_eq!(names_of(&db, "v1"), vec!["action", "comedy", "drama"]);
    assert_eq!(names_of(&db, "v2"), vec!["action", "comedy", "drama"]);
    assert!(names_of(&db, "v3").is_empty());
}

#[test]
fn bulk_assign_mixes_existing_and_new_tags_without_duplicating_the_existing() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2"]);
    let existing = assign_one(&mut conn, &["v1"], "old");

    let tags =
        queries::assign_tags_to_videos(&mut conn, &ids(&["v1", "v2"]), &ids(&["old", "fresh"]))
            .unwrap();

    assert_eq!(tags[0].id, existing.id);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags"), 2);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 4);
}

#[test]
fn bulk_assign_applies_names_that_normalize_to_the_same_string_once() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1"]);
    // A full-width letter and surrounding spaces fold to the same name.
    let tags = queries::assign_tags_to_videos(
        &mut conn,
        &ids(&["v1"]),
        &ids(&["a", " a ", "\u{FF41}", "b"]),
    )
    .unwrap();
    assert_eq!(tags.len(), 2);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags"), 2);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 2);
}

#[test]
fn bulk_assign_is_idempotent() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2", "v3"]);
    assign_one(&mut conn, &["v2"], "action");

    for _ in 0..2 {
        queries::assign_tags_to_videos(&mut conn, &ids(&["v1", "v2", "v3"]), &ids(&["action"]))
            .unwrap();
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 3);
        assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags"), 1);
    }
}

#[test]
fn bulk_assign_skips_unknown_ids_and_tags_the_rest() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2"]);
    queries::assign_tags_to_videos(
        &mut conn,
        &ids(&["v1", "gone", "v2", "v1", "gone"]),
        &ids(&["action"]),
    )
    .unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 2);
    assert!(orphaned_video_tags_by_video(&conn).is_empty());
}

#[test]
fn bulk_assign_with_an_invalid_name_changes_nothing() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2"]);
    for bad in ["", "   ", "\u{3000}"] {
        let result = queries::assign_tags_to_videos(&mut conn, &ids(&["v1", "v2"]), &ids(&[bad]));
        assert!(matches!(
            result,
            Err(queries::TagMutationError::InvalidName(_))
        ));
    }
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 0);
}

#[test]
fn bulk_assign_with_one_invalid_name_among_valid_ones_changes_nothing() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2"]);
    let result = queries::assign_tags_to_videos(
        &mut conn,
        &ids(&["v1", "v2"]),
        &ids(&["action", "  ", "comedy"]),
    );
    assert!(matches!(
        result,
        Err(queries::TagMutationError::InvalidName(_))
    ));
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 0);
}

#[test]
fn bulk_assign_rolls_back_everything_on_a_mid_batch_failure() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2", "v3"]);
    // Fails the insert for the second video, after the tags and the first
    // video's assignments were already written inside the open transaction.
    conn.execute(
        "CREATE TRIGGER fail_on_v2 BEFORE INSERT ON video_tags
         WHEN NEW.video_id = 'v2'
         BEGIN SELECT RAISE(ABORT, 'simulated failure'); END;",
        [],
    )
    .unwrap();
    let result = queries::assign_tags_to_videos(
        &mut conn,
        &ids(&["v1", "v2", "v3"]),
        &ids(&["action", "comedy"]),
    );
    assert!(matches!(result, Err(queries::TagMutationError::Sqlite(_))));
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 0);
}

#[test]
fn bulk_assign_with_no_ids_or_no_names_writes_nothing() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1"]);
    queries::assign_tags_to_videos(&mut conn, &[], &ids(&["action"])).unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 0);

    let tags = queries::assign_tags_to_videos(&mut conn, &ids(&["v1"]), &[]).unwrap();
    assert!(tags.is_empty());
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 0);
}

#[test]
fn bulk_assign_does_not_touch_other_videos_or_tags() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2", "v3"]);
    queries::assign_tag_to_video(&mut conn, "v3", "other").unwrap();
    queries::assign_tag_to_video(&mut conn, "v1", "other").unwrap();

    queries::assign_tags_to_videos(&mut conn, &ids(&["v1", "v2"]), &ids(&["action"])).unwrap();
    drop(conn);

    assert_eq!(names_of(&db, "v1"), vec!["action", "other"]);
    assert_eq!(names_of(&db, "v2"), vec!["action"]);
    assert_eq!(names_of(&db, "v3"), vec!["other"]);
}

#[test]
fn remove_tag_from_video_deletes_only_the_video_tags_row_tags_master_survives() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "v1", "D:\\videos\\a.mp4");
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
        insert_video(&conn, "v1", "D:\\videos\\a.mp4");
        insert_video(&conn, "v2", "D:\\videos\\b.mp4");
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
/// `delete_videos_cascade`.
#[test]
fn delete_videos_cascade_removes_all_its_video_tags_rows_and_leaves_others_untouched() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "v1", "D:\\videos\\a.mp4");
        insert_video(&conn, "v2", "D:\\videos\\b.mp4");
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
        queries::delete_videos_cascade(&mut conn, &["v1".to_string()]).unwrap();
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
        "no video_tags row may reference a nonexistent videos.id after delete_videos_cascade"
    );
}

#[test]
fn list_tags_for_video_returns_tags_sorted_by_name() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "v1", "D:\\videos\\a.mp4");
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
        insert_video(&conn, "v1", "D:\\videos\\a.mp4");
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
fn list_all_tags_with_video_count_counts_videos_and_keeps_empty_tags() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        seed_videos(&conn, &["a", "b", "c"]);
    }
    {
        let mut conn = db.writer.lock().unwrap();
        for v in ["a", "b", "c"] {
            queries::assign_tag_to_video(&mut conn, v, "popular").unwrap();
        }
        queries::assign_tag_to_video(&mut conn, "a", "rare").unwrap();
        let empty = queries::assign_tag_to_video(&mut conn, "a", "empty").unwrap();
        queries::remove_tag_from_video(&conn, "a", empty.id).unwrap();
    }
    let rows = queries::list_all_tags_with_video_count(&db.read_pool).unwrap();
    let got: Vec<(&str, i64)> = rows
        .iter()
        .map(|t| (t.name.as_str(), t.video_count))
        .collect();
    // No sort_order set yet, so name order.
    assert_eq!(got, [("empty", 0), ("popular", 3), ("rare", 1)]);
}

#[test]
fn list_all_tags_with_video_count_follows_sort_order_then_name() {
    let (_dir, db) = init_temp_db();
    let ids: Vec<i64> = {
        let mut conn = db.writer.lock().unwrap();
        for n in ["alpha", "beta", "gamma", "delta"] {
            conn.execute("INSERT INTO tags (name) VALUES (?1)", params![n])
                .unwrap();
        }
        let ids = conn
            .prepare("SELECT id FROM tags ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<Vec<i64>, _>>()
            .unwrap();
        // ids by name: alpha, beta, delta, gamma
        queries::set_tag_order(&mut conn, &[ids[3], ids[0]]).unwrap();
        ids
    };
    let rows = queries::list_all_tags_with_video_count(&db.read_pool).unwrap();
    let names: Vec<&str> = rows.iter().map(|t| t.name.as_str()).collect();
    // Ordered tags first (gamma, alpha), then the rest by name.
    assert_eq!(names, ["gamma", "alpha", "beta", "delta"]);
    let listed: Vec<i64> = queries::list_all_tags(&db.read_pool)
        .unwrap()
        .iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(listed, rows.iter().map(|t| t.id).collect::<Vec<_>>());
    assert_eq!(rows[0].id, ids[3]);
}

#[test]
fn set_rating_round_trips_and_clears_to_zero() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "v1", "D:\\videos\\a.mp4");
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

fn tag_pairs(conn: &Connection) -> Vec<(String, i64)> {
    let mut stmt = conn
        .prepare("SELECT video_id, tag_id FROM video_tags ORDER BY video_id, tag_id")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn bulk_remove_detaches_the_tag_from_every_listed_video_and_keeps_the_tags_row() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2"]);
    let tag = assign_one(&mut conn, &["v1", "v2"], "action");

    queries::remove_tags_from_videos(&mut conn, &ids(&["v1", "v2"]), &[tag.id]).unwrap();

    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags"), 1);
}

#[test]
fn bulk_remove_leaves_unlisted_videos_and_other_tags_alone() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2", "v3"]);
    let action = assign_one(&mut conn, &["v1", "v2", "v3"], "action");
    let other = assign_one(&mut conn, &["v1", "v2", "v3"], "other");

    queries::remove_tags_from_videos(&mut conn, &ids(&["v1", "v2"]), &[action.id]).unwrap();

    assert_eq!(
        tag_pairs(&conn),
        vec![
            ("v1".to_string(), other.id),
            ("v2".to_string(), other.id),
            ("v3".to_string(), action.id),
            ("v3".to_string(), other.id),
        ]
    );
}

#[test]
fn bulk_remove_only_detaches_from_videos_that_carry_the_tag() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2", "v3"]);
    let tag = assign_one(&mut conn, &["v1", "v3"], "action");

    queries::remove_tags_from_videos(&mut conn, &ids(&["v1", "v2"]), &[tag.id]).unwrap();

    assert_eq!(tag_pairs(&conn), vec![("v3".to_string(), tag.id)]);
}

#[test]
fn bulk_remove_rolls_back_everything_on_a_mid_batch_failure() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2", "v3"]);
    let all = ids(&["v1", "v2", "v3"]);
    let tag = assign_one(&mut conn, &["v1", "v2", "v3"], "action");
    conn.execute(
        "CREATE TRIGGER fail_on_v3 BEFORE DELETE ON video_tags
         WHEN OLD.video_id = 'v3'
         BEGIN SELECT RAISE(ABORT, 'simulated failure'); END;",
        [],
    )
    .unwrap();

    let result = queries::remove_tags_from_videos(&mut conn, &all, &[tag.id]);

    assert!(result.is_err());
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 3);
}

#[test]
fn bulk_remove_with_no_ids_or_unknown_ids_or_unknown_tag_changes_nothing() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1"]);
    let tag = queries::assign_tag_to_video(&mut conn, "v1", "action").unwrap();

    queries::remove_tags_from_videos(&mut conn, &[], &[tag.id]).unwrap();
    queries::remove_tags_from_videos(&mut conn, &ids(&["ghost"]), &[tag.id]).unwrap();
    queries::remove_tags_from_videos(&mut conn, &ids(&["v1"]), &[tag.id + 1000]).unwrap();

    assert_eq!(tag_pairs(&conn), vec![("v1".to_string(), tag.id)]);
}

#[test]
fn bulk_remove_accepts_duplicate_ids() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2"]);
    let tag = assign_one(&mut conn, &["v1", "v2"], "action");

    queries::remove_tags_from_videos(&mut conn, &ids(&["v1", "v1", "v1"]), &[tag.id]).unwrap();

    assert_eq!(tag_pairs(&conn), vec![("v2".to_string(), tag.id)]);
}

#[test]
fn bulk_remove_detaches_every_selected_tag_and_keeps_the_tags_rows() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2", "v3"]);
    let a = assign_one(&mut conn, &["v1", "v2", "v3"], "a");
    let b = assign_one(&mut conn, &["v1", "v2", "v3"], "b");
    let keep = assign_one(&mut conn, &["v1", "v2", "v3"], "keep");

    queries::remove_tags_from_videos(&mut conn, &ids(&["v1", "v2"]), &[a.id, b.id]).unwrap();

    assert_eq!(count(&conn, "SELECT COUNT(*) FROM tags"), 3);
    assert_eq!(
        tag_pairs(&conn),
        vec![
            ("v1".to_string(), keep.id),
            ("v2".to_string(), keep.id),
            ("v3".to_string(), a.id),
            ("v3".to_string(), b.id),
            ("v3".to_string(), keep.id),
        ]
    );
}

#[test]
fn bulk_remove_handles_tags_carried_by_only_some_of_the_videos() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2", "v3"]);
    let a = assign_one(&mut conn, &["v1"], "a");
    let b = assign_one(&mut conn, &["v2", "v3"], "b");

    queries::remove_tags_from_videos(&mut conn, &ids(&["v1", "v2"]), &[a.id, b.id]).unwrap();

    assert_eq!(tag_pairs(&conn), vec![("v3".to_string(), b.id)]);
}

#[test]
fn bulk_remove_rolls_back_every_tag_on_a_mid_batch_failure() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2"]);
    let a = assign_one(&mut conn, &["v1", "v2"], "a");
    let b = assign_one(&mut conn, &["v1", "v2"], "b");
    // The first tag is deleted before the second tag's delete fails.
    conn.execute(
        &format!(
            "CREATE TRIGGER fail_on_b BEFORE DELETE ON video_tags
             WHEN OLD.tag_id = {}
             BEGIN SELECT RAISE(ABORT, 'simulated failure'); END;",
            b.id
        ),
        [],
    )
    .unwrap();

    let result = queries::remove_tags_from_videos(&mut conn, &ids(&["v1", "v2"]), &[a.id, b.id]);

    assert!(result.is_err());
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 4);
}

#[test]
fn bulk_remove_accepts_duplicate_tag_ids_and_no_tag_ids() {
    let (_dir, db) = init_temp_db();
    let mut conn = db.writer.lock().unwrap();
    seed_videos(&conn, &["v1", "v2"]);
    let a = assign_one(&mut conn, &["v1", "v2"], "a");

    queries::remove_tags_from_videos(&mut conn, &ids(&["v1", "v2"]), &[]).unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM video_tags"), 2);

    queries::remove_tags_from_videos(&mut conn, &ids(&["v1"]), &[a.id, a.id, a.id]).unwrap();
    assert_eq!(tag_pairs(&conn), vec![("v2".to_string(), a.id)]);
}

#[test]
fn tag_usage_counts_per_tag_within_the_selection_in_tag_order() {
    let (_dir, db) = init_temp_db();
    {
        let mut conn = db.writer.lock().unwrap();
        seed_videos(&conn, &["v1", "v2", "v3"]);
        assign_one(&mut conn, &["v1", "v2"], "all");
        assign_one(&mut conn, &["v1"], "some");
        assign_one(&mut conn, &["v3"], "unselected");
    }

    let usage = queries::list_tag_usage_for_videos(&db.read_pool, &ids(&["v1", "v2", "v1"]))
        .unwrap()
        .into_iter()
        .map(|t| (t.name, t.video_count))
        .collect::<Vec<_>>();

    assert_eq!(usage, vec![("all".to_string(), 2), ("some".to_string(), 1)]);
}

#[test]
fn tag_usage_is_empty_for_no_ids_unknown_ids_or_untagged_videos() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        seed_videos(&conn, &["v1"]);
    }
    let usage = |v: &[String]| queries::list_tag_usage_for_videos(&db.read_pool, v).unwrap();
    assert!(usage(&[]).is_empty());
    assert!(usage(&ids(&["ghost"])).is_empty());
    assert!(usage(&ids(&["v1"])).is_empty());
}
