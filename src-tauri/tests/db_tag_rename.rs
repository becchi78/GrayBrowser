//! Renaming a tag through `db::queries::rename_tag`: validation, the
//! untouched order/group/assignments, and the layout it returns.

mod common;

use common::{assign_tag, init_temp_db, insert_tag, insert_video, sort_orders, tag_names};
use gb_core::tag_layout::{Direction, ItemRef};
use graybrowser_lib::db::queries::{self, LayoutItemRow};
use graybrowser_lib::db::Db;
use rusqlite::Connection;

/// "a" = top-level tag, "G<id>:name[x,y]" = group.
fn show(rows: &[LayoutItemRow]) -> String {
    rows.iter()
        .map(|r| match r {
            LayoutItemRow::Tag(t) => t.name.clone(),
            LayoutItemRow::Group { id, name, tags } => format!(
                "G{id}:{name}[{}]",
                tags.iter()
                    .map(|t| t.name.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn rename(db: &Db, id: i64, name: &str) -> Result<String, String> {
    let mut conn = db.writer.lock().unwrap();
    queries::rename_tag(&mut conn, id, name)
        .map(|r| show(&r))
        .map_err(|e| e.to_string())
}

fn layout(db: &Db) -> String {
    show(&queries::get_tag_layout(&db.read_pool).unwrap())
}

/// `(video_id, tag_id)` pairs, sorted.
fn assignments(conn: &Connection) -> Vec<(String, i64)> {
    conn.prepare("SELECT video_id, tag_id FROM video_tags ORDER BY video_id, tag_id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn seed(db: &Db, names: &[&str]) -> Vec<i64> {
    let conn = db.writer.lock().unwrap();
    names.iter().map(|n| insert_tag(&conn, n)).collect()
}

#[test]
fn renaming_keeps_video_assignments_and_order() {
    let (_dir, db) = init_temp_db();
    let ids = seed(&db, &["a", "b", "c"]);
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "v1", "D:\\v\\1.mp4");
        insert_video(&conn, "v2", "D:\\v\\2.mp4");
        assign_tag(&conn, "v1", ids[1]);
        assign_tag(&conn, "v2", ids[1]);
        assign_tag(&conn, "v2", ids[0]);
    }
    // Number the orders first so a rewrite would be visible.
    {
        let mut conn = db.writer.lock().unwrap();
        queries::move_tag_layout_item(&mut conn, ItemRef::Tag(ids[2]), Direction::Up).unwrap();
        queries::move_tag_layout_item(&mut conn, ItemRef::Tag(ids[2]), Direction::Down).unwrap();
    }
    let (orders_before, assigned_before) = {
        let conn = db.writer.lock().unwrap();
        (sort_orders(&conn), assignments(&conn))
    };

    let out = rename(&db, ids[1], "B2").unwrap();

    assert_eq!(out, "a B2 c");
    assert_eq!(layout(&db), out);
    let conn = db.writer.lock().unwrap();
    assert_eq!(sort_orders(&conn), orders_before);
    assert_eq!(assignments(&conn), assigned_before);
    // The returned counts still follow the tag, not the name.
    drop(conn);
    let rows = queries::get_tag_layout(&db.read_pool).unwrap();
    let counts: Vec<(String, i64)> = rows
        .iter()
        .filter_map(|r| match r {
            LayoutItemRow::Tag(t) => Some((t.name.clone(), t.video_count)),
            _ => None,
        })
        .collect();
    assert_eq!(
        counts,
        vec![("a".into(), 1), ("B2".into(), 2), ("c".into(), 0)]
    );
}

#[test]
fn a_name_used_by_another_tag_is_rejected_without_changes() {
    let (_dir, db) = init_temp_db();
    let ids = seed(&db, &["a", "b"]);

    let err = rename(&db, ids[0], "b").unwrap_err();

    assert!(err.contains("既に"), "{err}");
    assert!(err.contains('b'), "{err}");
    // The collision is found after normalization.
    assert!(rename(&db, ids[0], "  \u{FF42} ").is_err());
    assert_eq!(layout(&db), "a b");
}

#[test]
fn renaming_to_the_current_name_is_a_no_op() {
    let (_dir, db) = init_temp_db();
    let ids = seed(&db, &["a", "b"]);
    let before = {
        let conn = db.writer.lock().unwrap();
        sort_orders(&conn)
    };

    assert_eq!(rename(&db, ids[0], "a").unwrap(), "a b");
    // Same name after normalization.
    assert_eq!(rename(&db, ids[0], " \u{FF41} ").unwrap(), "a b");

    let conn = db.writer.lock().unwrap();
    assert_eq!(sort_orders(&conn), before);
}

#[test]
fn a_name_differing_only_in_case_is_a_different_tag() {
    let (_dir, db) = init_temp_db();
    let ids = seed(&db, &["action", "Action"]);

    // A spelling that differs from every other tag only by case is free...
    assert_eq!(rename(&db, ids[0], "ACTION").unwrap(), "ACTION Action");
    // ...while the exact spelling of another tag is taken.
    assert!(rename(&db, ids[0], "Action").is_err());
}

#[test]
fn the_name_is_normalized_like_a_new_tag() {
    let (_dir, db) = init_temp_db();
    let ids = seed(&db, &["a"]);

    let out = rename(&db, ids[0], "\u{3000}\u{FF21}\u{FF43}tion \u{3000}").unwrap();

    assert_eq!(out, "Action");
    assert_eq!(tag_names(&db), ["Action"]);
}

#[test]
fn an_empty_name_is_rejected_with_a_tag_specific_message() {
    let (_dir, db) = init_temp_db();
    let ids = seed(&db, &["a"]);

    for raw in ["", "   ", "\u{3000}"] {
        let err = rename(&db, ids[0], raw).unwrap_err();
        assert!(err.contains("タグ名"), "{err}");
        assert!(!err.contains("invalid tag name"), "{err}");
    }
    assert_eq!(layout(&db), "a");
}

#[test]
fn an_unknown_tag_id_is_an_error_and_changes_nothing() {
    let (_dir, db) = init_temp_db();
    seed(&db, &["a"]);

    let err = rename(&db, 999, "x").unwrap_err();

    assert!(err.contains("見つかりません"), "{err}");
    assert_eq!(layout(&db), "a");
}

#[test]
fn renaming_a_tag_in_a_group_keeps_the_group_and_order() {
    let (_dir, db) = init_temp_db();
    let ids = seed(&db, &["a", "b", "c"]);
    {
        let mut conn = db.writer.lock().unwrap();
        queries::create_tag_group(&mut conn, "g").unwrap();
        queries::move_tag_layout_item(&mut conn, ItemRef::Tag(ids[0]), Direction::Up).unwrap();
        queries::move_tag_layout_item(&mut conn, ItemRef::Tag(ids[1]), Direction::Up).unwrap();
    }
    assert_eq!(layout(&db), "G1:g[a,b] c");
    let before = {
        let conn = db.writer.lock().unwrap();
        sort_orders(&conn)
    };

    let out = rename(&db, ids[1], "bb").unwrap();

    assert_eq!(out, "G1:g[a,bb] c");
    assert_eq!(layout(&db), out);
    let conn = db.writer.lock().unwrap();
    assert_eq!(sort_orders(&conn), before);
    let group_ids: Vec<Option<i64>> = conn
        .prepare("SELECT group_id FROM tags ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(group_ids, vec![Some(1), Some(1), None]);
}

#[test]
fn a_rename_writes_nothing_but_the_name() {
    let (_dir, db) = init_temp_db();
    let ids = seed(&db, &["a", "b"]);
    // Every sort_order is still NULL: a layout save would number them.

    rename(&db, ids[0], "z").unwrap();

    let conn = db.writer.lock().unwrap();
    assert!(sort_orders(&conn).iter().all(|(_, o)| o.is_none()));
}
