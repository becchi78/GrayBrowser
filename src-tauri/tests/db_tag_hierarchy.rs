//! Integration tests for the two-level tag hierarchy against a real
//! tempdir-backed SQLite file: filtering by a parent tag, deleting a parent,
//! and the all-or-nothing `set_tag_layout`. `foreign_keys` is OFF, so the
//! "no dangling parent_id" checks are done with explicit LEFT JOINs.

use gb_core::sort::{SortDirection, SortField};
use gb_core::tag_hierarchy::LayoutItem;
use graybrowser_lib::db::{self, queries};
use rusqlite::{params, Connection};

fn init_temp_db() -> (tempfile::TempDir, db::Db) {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");
    let db = db::init(&db_path).expect("db::init should succeed against a fresh tempdir file");
    (dir, db)
}

fn item(id: i64, parent_id: Option<i64>) -> LayoutItem {
    LayoutItem { id, parent_id }
}

/// Tags: 1 genre (parent of 2 rock and 3 jazz), 4 other (top-level), 5 live
/// (top-level, no children).
/// Videos: v_genre has 1; v_rock has 2; v_jazz has 3; v_rock_live has 2+5;
/// v_other has 4; v_none has nothing.
fn seed(conn: &Connection) {
    conn.execute_batch(
        "INSERT INTO tags (id, name, parent_id) VALUES
           (1, 'genre', NULL), (2, 'rock', 1), (3, 'jazz', 1), (4, 'other', NULL), (5, 'live', NULL);",
    )
    .unwrap();
    for id in [
        "v_genre",
        "v_rock",
        "v_jazz",
        "v_rock_live",
        "v_other",
        "v_none",
    ] {
        conn.execute(
            "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status, created_at)
             VALUES (?1, ?2, ?1, 1024, 'h', 'online', '2026-01-01 00:00:00')",
            params![id, format!("D:\\{id}.mp4")],
        )
        .unwrap();
    }
    conn.execute_batch(
        "INSERT INTO video_tags (video_id, tag_id) VALUES
           ('v_genre', 1), ('v_rock', 2), ('v_jazz', 3),
           ('v_rock_live', 2), ('v_rock_live', 5), ('v_other', 4);",
    )
    .unwrap();
}

fn filtered(db: &db::Db, tag_ids: &[i64]) -> Vec<String> {
    let mut ids: Vec<String> = queries::list_videos_filtered(
        &db.read_pool,
        &[],
        SortField::FileName,
        SortDirection::Asc,
        tag_ids,
        None,
        &[],
    )
    .unwrap()
    .into_iter()
    .map(|v| v.id)
    .collect();
    ids.sort();
    ids
}

fn layout_of(db: &db::Db) -> Vec<(i64, Option<i64>, Option<i64>)> {
    let conn = db.writer.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT id, parent_id, sort_order FROM tags ORDER BY id")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// A tag whose parent_id names a tag that does not exist.
fn dangling_parents(conn: &Connection) -> Vec<i64> {
    let mut stmt = conn
        .prepare(
            "SELECT t.id FROM tags t LEFT JOIN tags p ON p.id = t.parent_id
             WHERE t.parent_id IS NOT NULL AND p.id IS NULL",
        )
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn selecting_a_parent_matches_the_parent_itself_and_any_child() {
    let (_dir, db) = init_temp_db();
    seed(&db.writer.lock().unwrap());
    assert_eq!(
        filtered(&db, &[1]),
        ["v_genre", "v_jazz", "v_rock", "v_rock_live"]
    );
}

#[test]
fn selecting_only_a_child_does_not_match_the_parent_or_siblings() {
    let (_dir, db) = init_temp_db();
    seed(&db.writer.lock().unwrap());
    assert_eq!(filtered(&db, &[2]), ["v_rock", "v_rock_live"]);
    assert_eq!(filtered(&db, &[3]), ["v_jazz"]);
}

#[test]
fn selecting_a_tag_without_children_behaves_as_before() {
    let (_dir, db) = init_temp_db();
    seed(&db.writer.lock().unwrap());
    assert_eq!(filtered(&db, &[4]), ["v_other"]);
    assert_eq!(filtered(&db, &[5]), ["v_rock_live"]);
}

#[test]
fn a_parent_and_another_tag_are_still_anded() {
    let (_dir, db) = init_temp_db();
    seed(&db.writer.lock().unwrap());
    // genre-or-child AND live: only the video that has a child and 'live'.
    assert_eq!(filtered(&db, &[1, 5]), ["v_rock_live"]);
    // genre-or-child AND other: nothing carries both.
    assert!(filtered(&db, &[1, 4]).is_empty());
}

#[test]
fn a_parent_and_one_of_its_own_children_together_equal_the_child() {
    let (_dir, db) = init_temp_db();
    seed(&db.writer.lock().unwrap());
    assert_eq!(filtered(&db, &[1, 2]), ["v_rock", "v_rock_live"]);
}

#[test]
fn two_parents_are_anded_not_ored() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        seed(&conn);
        conn.execute_batch(
            "INSERT INTO tags (id, name, parent_id) VALUES (6, 'mood', NULL), (7, 'calm', 6);
             INSERT INTO video_tags (video_id, tag_id) VALUES ('v_jazz', 7), ('v_other', 7);",
        )
        .unwrap();
    }
    // genre-or-child AND mood-or-child: only the jazz video has both.
    assert_eq!(filtered(&db, &[1, 6]), ["v_jazz"]);
}

#[test]
fn an_unknown_tag_matches_nothing_and_no_tags_matches_everything() {
    let (_dir, db) = init_temp_db();
    seed(&db.writer.lock().unwrap());
    assert!(filtered(&db, &[999]).is_empty());
    assert_eq!(filtered(&db, &[]).len(), 6);
}

#[test]
fn deleting_a_parent_returns_its_children_to_top_level_and_keeps_their_videos() {
    let (_dir, db) = init_temp_db();
    seed(&db.writer.lock().unwrap());
    {
        let mut conn = db.writer.lock().unwrap();
        queries::delete_tag(&mut conn, 1).unwrap();
    }
    let layout = layout_of(&db);
    assert_eq!(
        layout.iter().map(|t| (t.0, t.1)).collect::<Vec<_>>(),
        [(2, None), (3, None), (4, None), (5, None)]
    );
    // The children keep their own tag assignments; only the parent's are gone.
    assert_eq!(filtered(&db, &[2]), ["v_rock", "v_rock_live"]);
    assert_eq!(filtered(&db, &[3]), ["v_jazz"]);
    let conn = db.writer.lock().unwrap();
    assert!(dangling_parents(&conn).is_empty());
    let orphans: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM video_tags vt LEFT JOIN tags t ON t.id = vt.tag_id WHERE t.id IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(orphans, 0);
}

#[test]
fn deleting_a_child_leaves_the_parent_and_siblings_alone() {
    let (_dir, db) = init_temp_db();
    seed(&db.writer.lock().unwrap());
    {
        let mut conn = db.writer.lock().unwrap();
        queries::delete_tag(&mut conn, 2).unwrap();
    }
    let layout = layout_of(&db);
    assert_eq!(
        layout.iter().map(|t| (t.0, t.1)).collect::<Vec<_>>(),
        [(1, None), (3, Some(1)), (4, None), (5, None)]
    );
}

#[test]
fn set_tag_layout_stores_order_and_parents() {
    let (_dir, db) = init_temp_db();
    seed(&db.writer.lock().unwrap());
    {
        let mut conn = db.writer.lock().unwrap();
        // 4 first, then genre with jazz before rock, then live under other.
        queries::set_tag_layout(
            &mut conn,
            &[
                item(4, None),
                item(5, Some(4)),
                item(1, None),
                item(3, Some(1)),
                item(2, Some(1)),
            ],
        )
        .unwrap();
    }
    assert_eq!(
        layout_of(&db),
        [
            (1, None, Some(2)),
            (2, Some(1), Some(4)),
            (3, Some(1), Some(3)),
            (4, None, Some(0)),
            (5, Some(4), Some(1)),
        ]
    );
    let conn = db.writer.lock().unwrap();
    assert!(dangling_parents(&conn).is_empty());
}

#[test]
fn set_tag_layout_can_clear_a_parent() {
    let (_dir, db) = init_temp_db();
    seed(&db.writer.lock().unwrap());
    {
        let mut conn = db.writer.lock().unwrap();
        queries::set_tag_layout(&mut conn, &[item(2, None)]).unwrap();
    }
    let layout = layout_of(&db);
    assert_eq!(layout[1], (2, None, Some(0)));
    // Unlisted tags keep their parent but lose their place in the order.
    assert_eq!(layout[2], (3, Some(1), None));
}

#[test]
fn set_tag_layout_skips_unknown_ids_without_a_gap_and_keeps_first_duplicate() {
    let (_dir, db) = init_temp_db();
    seed(&db.writer.lock().unwrap());
    {
        let mut conn = db.writer.lock().unwrap();
        queries::set_tag_layout(
            &mut conn,
            &[
                item(99, None),
                item(5, None),
                item(5, Some(4)),
                item(4, None),
            ],
        )
        .unwrap();
    }
    let layout = layout_of(&db);
    assert_eq!(layout[4], (5, None, Some(0)));
    assert_eq!(layout[3], (4, None, Some(1)));
}

#[test]
fn set_tag_layout_changes_what_the_listings_return() {
    let (_dir, db) = init_temp_db();
    seed(&db.writer.lock().unwrap());
    {
        let mut conn = db.writer.lock().unwrap();
        queries::set_tag_layout(&mut conn, &[item(4, None), item(1, None), item(2, Some(1))])
            .unwrap();
    }
    let rows = queries::list_all_tags_with_video_count(&db.read_pool).unwrap();
    let got: Vec<(i64, Option<i64>)> = rows.iter().map(|t| (t.id, t.parent_id)).collect();
    // Listed tags in the saved order, then the unlisted ones by name.
    assert_eq!(
        got,
        [(4, None), (1, None), (2, Some(1)), (3, Some(1)), (5, None)]
    );
}

fn assert_layout_rejected(items: &[LayoutItem], expect_tag: i64) {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        seed(&conn);
        conn.execute("UPDATE tags SET sort_order = id * 10", [])
            .unwrap();
    }
    let before = layout_of(&db);
    let err = {
        let mut conn = db.writer.lock().unwrap();
        queries::set_tag_layout(&mut conn, items).unwrap_err()
    };
    match err {
        queries::TagLayoutError::Invalid { tag_id, .. } => assert_eq!(tag_id, expect_tag),
        other => panic!("expected a rule violation, got {other}"),
    }
    assert_eq!(
        layout_of(&db),
        before,
        "a rejected layout must change nothing"
    );
}

#[test]
fn set_tag_layout_rejects_a_self_parent_and_changes_nothing() {
    // Valid-looking earlier items must not have been written either.
    assert_layout_rejected(&[item(4, None), item(5, Some(5))], 5);
}

#[test]
fn set_tag_layout_rejects_a_missing_parent_and_changes_nothing() {
    assert_layout_rejected(&[item(4, None), item(5, Some(99))], 5);
}

#[test]
fn set_tag_layout_rejects_a_child_as_parent_and_changes_nothing() {
    assert_layout_rejected(&[item(4, None), item(5, Some(2))], 5);
}

#[test]
fn set_tag_layout_rejects_a_tag_with_children_becoming_a_child_and_changes_nothing() {
    assert_layout_rejected(&[item(4, None), item(1, Some(4))], 1);
}

#[test]
fn set_tag_layout_rejects_a_cycle_and_changes_nothing() {
    assert_layout_rejected(&[item(1, Some(2)), item(2, Some(1))], 1);
}

#[test]
fn a_failure_midway_through_writing_rolls_back_the_whole_layout() {
    let (_dir, db) = init_temp_db();
    seed(&db.writer.lock().unwrap());
    let before = layout_of(&db);
    {
        let mut conn = db.writer.lock().unwrap();
        // Make the second UPDATE fail after the first one and the sort_order
        // reset already ran inside the transaction.
        conn.execute_batch(
            "CREATE TRIGGER fail_on_tag_5 BEFORE UPDATE OF parent_id ON tags
             WHEN NEW.id = 5
             BEGIN SELECT RAISE(ABORT, 'forced failure'); END;",
        )
        .unwrap();
        let err =
            queries::set_tag_layout(&mut conn, &[item(4, None), item(5, Some(4))]).unwrap_err();
        assert!(matches!(err, queries::TagLayoutError::Sqlite(_)));
    }
    assert_eq!(layout_of(&db), before);
}
