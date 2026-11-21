//! Tag groups through `db::queries`: create / rename / delete / move, and the
//! collapsed-state setting. Writes go through the single writer connection.

mod common;

use common::{init_temp_db, insert_video, tag_names};
use gb_core::tag_layout::{Direction, ItemRef};
use graybrowser_lib::db::queries::{self, LayoutItemRow};
use graybrowser_lib::db::Db;
use rusqlite::{params, Connection};

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

fn layout(db: &Db) -> String {
    show(&queries::get_tag_layout(&db.read_pool).unwrap())
}

fn tag_id(db: &Db, name: &str) -> i64 {
    let conn: std::sync::MutexGuard<Connection> = db.writer.lock().unwrap();
    conn.query_row("SELECT id FROM tags WHERE name = ?1", [name], |r| r.get(0))
        .unwrap()
}

fn seed(db: &Db, names: &[&str]) {
    let conn = db.writer.lock().unwrap();
    for n in names {
        conn.execute("INSERT INTO tags (name) VALUES (?1)", [n])
            .unwrap();
    }
}

fn mv(db: &Db, item: ItemRef, d: Direction) -> String {
    let mut conn = db.writer.lock().unwrap();
    show(&queries::move_tag_layout_item(&mut conn, item, d).unwrap())
}

fn up(db: &Db, name: &str) -> String {
    mv(db, ItemRef::Tag(tag_id(db, name)), Direction::Up)
}

fn down(db: &Db, name: &str) -> String {
    mv(db, ItemRef::Tag(tag_id(db, name)), Direction::Down)
}

fn create(db: &Db, name: &str) -> Result<String, String> {
    let mut conn = db.writer.lock().unwrap();
    queries::create_tag_group(&mut conn, name)
        .map(|r| show(&r))
        .map_err(|e| e.to_string())
}

fn count(db: &Db, sql: &str) -> i64 {
    db.writer
        .lock()
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}

#[test]
fn a_new_group_is_created_empty_at_the_top() {
    let (_dir, db) = init_temp_db();
    seed(&db, &["a", "b"]);
    // Full-width letters and surrounding spaces are normalized like tag names.
    let out = create(&db, "  \u{FF27}rp ").unwrap();
    assert_eq!(out, "G1:Grp[] a b");
    assert_eq!(layout(&db), out);
    assert_eq!(create(&db, "second").unwrap(), "G2:second[] G1:Grp[] a b");
    assert_eq!(tag_names(&db), ["a", "b"]);
}

#[test]
fn creating_a_group_rejects_empty_and_duplicate_names_without_changes() {
    let (_dir, db) = init_temp_db();
    seed(&db, &["a"]);
    create(&db, "g").unwrap();
    assert!(create(&db, "   ").is_err());
    let err = create(&db, "g").unwrap_err();
    assert!(err.contains("既に"), "{err}");
    assert_eq!(layout(&db), "G1:g[] a");
}

#[test]
fn renaming_a_group_checks_duplicates_and_allows_the_same_name() {
    let (_dir, db) = init_temp_db();
    create(&db, "one").unwrap();
    create(&db, "two").unwrap();
    let mut conn = db.writer.lock().unwrap();

    let same = queries::rename_tag_group(&mut conn, 1, "one").unwrap();
    assert_eq!(show(&same), "G2:two[] G1:one[]");
    assert!(queries::rename_tag_group(&mut conn, 1, "two").is_err());
    assert!(queries::rename_tag_group(&mut conn, 1, " ").is_err());
    assert!(queries::rename_tag_group(&mut conn, 999, "x").is_err());
    let renamed = queries::rename_tag_group(&mut conn, 2, "TWO").unwrap();
    assert_eq!(show(&renamed), "G2:TWO[] G1:one[]");
}

#[test]
fn tags_move_in_and_out_of_groups_through_the_db() {
    let (_dir, db) = init_temp_db();
    seed(&db, &["a", "b", "c"]);
    create(&db, "g").unwrap(); // G1:g[] a b c

    assert_eq!(up(&db, "a"), "G1:g[a] b c");
    assert_eq!(up(&db, "b"), "G1:g[a,b] c");
    assert_eq!(layout(&db), "G1:g[a,b] c");
    // Up from the first member leaves the group, landing above it.
    assert_eq!(up(&db, "a"), "a G1:g[b] c");
    assert_eq!(down(&db, "a"), "G1:g[a,b] c");
    // Down from the last member leaves the group, landing below it.
    assert_eq!(down(&db, "b"), "G1:g[a] b c");
    // Edges change nothing.
    assert_eq!(down(&db, "c"), "G1:g[a] b c");
    assert_eq!(mv(&db, ItemRef::Group(1), Direction::Up), "G1:g[a] b c");
    assert_eq!(mv(&db, ItemRef::Group(1), Direction::Down), "b G1:g[a] c");
    assert_eq!(layout(&db), "b G1:g[a] c");
}

#[test]
fn a_tag_can_enter_an_empty_group() {
    let (_dir, db) = init_temp_db();
    seed(&db, &["a"]);
    create(&db, "g").unwrap();
    mv(&db, ItemRef::Group(1), Direction::Down);
    assert_eq!(layout(&db), "a G1:g[]");
    assert_eq!(
        up(&db, "a"),
        "a G1:g[]",
        "a is above the group: up is the edge"
    );
    assert_eq!(down(&db, "a"), "G1:g[a]");
}

#[test]
fn every_save_renumbers_tags_and_groups_consecutively() {
    let (_dir, db) = init_temp_db();
    seed(&db, &["a", "b", "c"]);
    create(&db, "g").unwrap(); // G1[] a b c
    up(&db, "a"); // G1[a] b c
    up(&db, "b"); // G1[a,b] c

    let conn = db.writer.lock().unwrap();
    let rows: Vec<(String, Option<i64>, Option<i64>)> = conn
        .prepare("SELECT name, group_id, sort_order FROM tags ORDER BY name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            ("a".to_string(), Some(1), Some(0)),
            ("b".to_string(), Some(1), Some(1)),
            ("c".to_string(), None, Some(1)),
        ]
    );
    let group_order: Option<i64> = conn
        .query_row("SELECT sort_order FROM tag_groups WHERE id = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(group_order, Some(0));
}

#[test]
fn deleting_a_group_keeps_tags_and_video_tags_and_leaves_no_orphans() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "v1", "D:\\v\\a.mp4");
        insert_video(&conn, "v2", "D:\\v\\b.mp4");
    }
    {
        let mut conn = db.writer.lock().unwrap();
        queries::assign_tag_to_video(&mut conn, "v1", "a").unwrap();
        queries::assign_tag_to_video(&mut conn, "v2", "a").unwrap();
        queries::assign_tag_to_video(&mut conn, "v1", "b").unwrap();
        queries::assign_tag_to_video(&mut conn, "v1", "z").unwrap();
    }
    create(&db, "g").unwrap(); // G1:g[] a b z
    up(&db, "a");
    up(&db, "b");
    assert_eq!(layout(&db), "G1:g[a,b] z");

    let before = count(&db, "SELECT COUNT(*) FROM video_tags");
    let out = {
        let mut conn = db.writer.lock().unwrap();
        queries::delete_tag_group(&mut conn, 1).unwrap()
    };
    assert_eq!(show(&out), "a b z");
    assert!(matches!(&out[0], LayoutItemRow::Tag(t) if t.video_count == 2));

    assert_eq!(count(&db, "SELECT COUNT(*) FROM video_tags"), before);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM tags"), 3);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM tag_groups"), 0);
    let orphans = count(
        &db,
        "SELECT COUNT(*) FROM tags t LEFT JOIN tag_groups g ON g.id = t.group_id \
         WHERE t.group_id IS NOT NULL AND g.id IS NULL",
    );
    assert_eq!(orphans, 0);
}

#[test]
fn deleting_an_unknown_group_changes_nothing() {
    let (_dir, db) = init_temp_db();
    seed(&db, &["a"]);
    create(&db, "g").unwrap();
    let mut conn = db.writer.lock().unwrap();
    let out = queries::delete_tag_group(&mut conn, 999).unwrap();
    assert_eq!(show(&out), "G1:g[] a");
}

#[test]
fn a_dangling_group_id_reads_as_top_level_and_is_repaired_on_next_save() {
    let (_dir, db) = init_temp_db();
    seed(&db, &["a", "b"]);
    db.writer
        .lock()
        .unwrap()
        .execute("UPDATE tags SET group_id = 42 WHERE name = 'a'", [])
        .unwrap();
    assert_eq!(layout(&db), "a b");
    assert_eq!(up(&db, "b"), "b a");
    assert_eq!(
        count(&db, "SELECT COUNT(*) FROM tags WHERE group_id IS NOT NULL"),
        0
    );
}

#[test]
fn list_all_tags_lists_group_members_where_the_group_sits() {
    let (_dir, db) = init_temp_db();
    seed(&db, &["a", "b", "c", "d"]);
    create(&db, "g").unwrap(); // G1[] a b c d
    up(&db, "a");
    up(&db, "b"); // G1[a,b] c d
    assert_eq!(mv(&db, ItemRef::Group(1), Direction::Down), "c G1:g[a,b] d");
    assert_eq!(tag_names(&db), ["c", "a", "b", "d"]);
}

#[test]
fn usage_for_videos_follows_the_layout_order() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "v1", "D:\\v\\a.mp4");
    }
    {
        let mut conn = db.writer.lock().unwrap();
        for n in ["a", "b", "c"] {
            queries::assign_tag_to_video(&mut conn, "v1", n).unwrap();
        }
    }
    create(&db, "g").unwrap();
    up(&db, "a");
    up(&db, "b"); // G1[a,b] c
    mv(&db, ItemRef::Group(1), Direction::Down); // c G1[a,b]
    let names: Vec<String> = queries::list_tag_usage_for_videos(&db.read_pool, &["v1".to_string()])
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, ["c", "a", "b"]);
}

fn db_with_collapsed(raw: Option<&str>) -> (tempfile::TempDir, Db) {
    let (dir, db) = init_temp_db();
    if let Some(raw) = raw {
        db.writer
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO settings (key, value) VALUES ('tag_groups_collapsed', ?1)",
                params![raw],
            )
            .unwrap();
    }
    (dir, db)
}

#[test]
fn collapsed_groups_read_the_stored_ids_and_fall_back_to_all_open() {
    let cases: [(Option<&str>, Vec<i64>); 5] = [
        (None, vec![]),
        (Some("{not json"), vec![]),
        (Some(r#"{"a":1}"#), vec![]),
        (Some(r#"["x"]"#), vec![]),
        (Some("[3,1]"), vec![3, 1]),
    ];
    for (raw, expected) in cases {
        let (_dir, db) = db_with_collapsed(raw);
        let conn = db.writer.lock().unwrap();
        assert_eq!(
            queries::get_collapsed_tag_groups(&conn).unwrap(),
            expected,
            "stored {raw:?}"
        );
    }
}

#[test]
fn collapsed_groups_round_trip_and_drop_duplicates() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    queries::set_collapsed_tag_groups(&conn, &[2, 1, 2]).unwrap();
    assert_eq!(queries::get_collapsed_tag_groups(&conn).unwrap(), [2, 1]);
    queries::set_collapsed_tag_groups(&conn, &[]).unwrap();
    assert!(queries::get_collapsed_tag_groups(&conn).unwrap().is_empty());
}

type TagState = Vec<(String, Option<i64>, Option<i64>)>;
type GroupState = Vec<(String, Option<i64>)>;

/// `(name, group_id, sort_order)` of every tag plus `(name, sort_order)` of
/// every group, to detect any write.
fn stored_state(db: &Db) -> (TagState, GroupState) {
    let conn = db.writer.lock().unwrap();
    let tags = conn
        .prepare("SELECT name, group_id, sort_order FROM tags ORDER BY name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let groups = conn
        .prepare("SELECT name, sort_order FROM tag_groups ORDER BY name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    (tags, groups)
}

#[test]
fn an_edge_move_writes_nothing() {
    let (_dir, db) = init_temp_db();
    // Every sort_order is still NULL: a save would number them.
    seed(&db, &["a", "b"]);
    let before = stored_state(&db);
    assert_eq!(up(&db, "a"), "a b");
    assert_eq!(down(&db, "b"), "a b");
    assert_eq!(mv(&db, ItemRef::Tag(9999), Direction::Up), "a b");
    assert_eq!(stored_state(&db), before);
    assert!(before.0.iter().all(|t| t.2.is_none()));
}

#[test]
fn failed_group_edits_leave_the_stored_state_unchanged() {
    let (_dir, db) = init_temp_db();
    seed(&db, &["a", "b"]);
    {
        let mut conn = db.writer.lock().unwrap();
        // Group rows exist but nothing has been saved yet: NULL orders.
        conn.execute("INSERT INTO tag_groups (name) VALUES ('g'), ('h')", [])
            .unwrap();
        assert!(queries::rename_tag_group(&mut conn, 999, "x").is_err());
        assert!(queries::rename_tag_group(&mut conn, 1, "h").is_err());
        assert!(queries::rename_tag_group(&mut conn, 1, "  ").is_err());
        assert!(queries::create_tag_group(&mut conn, "g").is_err());
        assert!(queries::create_tag_group(&mut conn, "").is_err());
    }
    let (tags, groups) = stored_state(&db);
    assert!(tags.iter().all(|t| t.1.is_none() && t.2.is_none()));
    assert_eq!(
        groups,
        vec![("g".to_string(), None), ("h".to_string(), None)]
    );
}

#[test]
fn an_empty_group_name_gets_a_group_specific_message() {
    let (_dir, db) = init_temp_db();
    let err = create(&db, "  ").unwrap_err();
    assert!(err.contains("グループ名"), "{err}");
    assert!(!err.contains("invalid tag name"), "{err}");
    create(&db, "g").unwrap();
    let mut conn = db.writer.lock().unwrap();
    let err = queries::rename_tag_group(&mut conn, 1, "").unwrap_err();
    assert!(err.to_string().contains("グループ名"));
}
