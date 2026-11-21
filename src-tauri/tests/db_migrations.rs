//! Integration tests against a real tempdir-backed SQLite file (not
//! `:memory:` -- WAL mode only means something for a file-backed database).

mod common;

use common::{apply_up_to, init_temp_db, latest_schema_version, sort_orders, tag_names};
use graybrowser_lib::db;

/// Names of every object of `kind` ("table" or "index") in the database.
fn object_names(conn: &rusqlite::Connection, kind: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = ?1 ORDER BY name")
        .unwrap();
    stmt.query_map([kind], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// Not an exact listing: every later migration may add tables or indexes, and
/// this must not break each time. What matters is that none of these goes
/// missing. (`sqlite_sequence`, which SQLite creates itself for the
/// AUTOINCREMENT tables, is deliberately not asserted.)
#[test]
fn creates_the_required_tables_and_indexes() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();

    let tables = object_names(&conn, "table");
    for required in [
        "path_collisions",
        "schema_version",
        "settings",
        "skipped_files",
        "tags",
        "video_tags",
        "videos",
        "wb_unmatched_thumbnails",
    ] {
        assert!(
            tables.iter().any(|t| t == required),
            "table {required} should exist, got {tables:?}"
        );
    }

    let indexes = object_names(&conn, "index");
    for required in [
        "idx_path_collisions_colliding",
        "idx_video_tags_tag",
        "idx_videos_created_at",
        "idx_videos_name",
        "idx_videos_quick_hash",
        "idx_videos_rating",
        "idx_videos_status",
    ] {
        assert!(
            indexes.iter().any(|i| i == required),
            "index {required} should exist, got {indexes:?}"
        );
    }
}

#[test]
fn journal_mode_is_wal() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode.to_lowercase(), "wal");
}

#[test]
fn foreign_keys_pragma_is_not_forced_on() {
    // Regression test for the decision to keep FK enforcement off, relying
    // on app-layer transactions for referential integrity instead. Note:
    // the bundled SQLite build defaults this pragma to *on*, so db::init
    // explicitly disables it on every connection (see
    // db::disable_foreign_keys) -- this test exists to catch a future
    // change accidentally dropping that explicit call.
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    let enabled: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(enabled, 0);
}

#[test]
fn rerunning_migrations_is_a_no_op() {
    let (_dir, db) = init_temp_db();
    {
        let mut conn = db.writer.lock().unwrap();
        db::migrations::run_migrations(&mut conn)
            .expect("re-running migrations should be a no-op, not an error");
    }
    let conn = db.writer.lock().unwrap();
    // A fresh DB lands on every migration, and re-running must not insert
    // extra schema_version rows for any of them.
    let version_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version_count, latest_schema_version());
}

/// kana/roma (v3) and the ffprobe metadata columns
/// (v4) must all exist and be nullable -- every row is NULL for kana/roma
/// until kana/roma generation runs, and NULL for the metadata columns until
/// the background probe worker fills them in.
#[test]
fn migrations_v3_and_v4_add_nullable_columns() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();

    let mut stmt = conn.prepare("PRAGMA table_info(videos)").unwrap();
    let columns: Vec<(String, i64)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();

    for expected in [
        "kana",
        "roma",
        "width",
        "height",
        "video_codec",
        "audio_codec",
        "bitrate",
        "fps",
        "probed_at",
    ] {
        let col = columns
            .iter()
            .find(|(name, _)| name == expected)
            .unwrap_or_else(|| panic!("videos.{expected} column should exist after migration v4"));
        assert_eq!(col.1, 0, "videos.{expected} must be nullable");
    }
}

/// Reproduces upgrading a real database file at schema_version=1 (no `mtime`
/// column) or 2 (no kana/roma/metadata columns, v5 indexes, v6
/// path_collisions table, or v7 retry-attempt columns) all the way to the
/// current schema, rather than relying on a fresh DB always applying every
/// migration together -- the actual upgrade path a returning user's `app.db`
/// takes, from the oldest possible starting points.
#[test]
fn migrating_from_a_real_v1_or_v2_database_reaches_the_current_version_cleanly() {
    for start_version in [1, 2] {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let db_path = dir.path().join("app.db");
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            apply_up_to(&conn, start_version);
        }

        let db = db::init(&db_path).unwrap_or_else(|e| {
            panic!(
                "db::init should upgrade a v{start_version} database to the current version: {e}"
            )
        });
        let conn = db.writer.lock().unwrap();

        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, latest_schema_version(), "from v{start_version}");

        // Each added column or table must be usable, not just present in the
        // schema.
        conn.execute("UPDATE videos SET mtime = 123 WHERE id = 'nonexistent'", [])
            .expect("mtime column should be queryable/writable after the upgrade");

        conn.execute(
            "UPDATE videos SET kana = 'test', probed_at = CURRENT_TIMESTAMP WHERE id = 'nonexistent'",
            [],
        )
        .expect("kana/probed_at columns should be queryable/writable after the upgrade");

        conn.execute(
            "INSERT INTO path_collisions (video_id, colliding_video_id, attempted_path) VALUES ('a', 'b', 'C:/x.mp4')",
            [],
        )
        .expect("path_collisions table should be usable after the upgrade");

        conn.execute(
            "UPDATE videos SET thumbnail_attempts = 1, metadata_attempts = 1 WHERE id = 'nonexistent'",
            [],
        )
        .expect("thumbnail_attempts/metadata_attempts columns should be queryable/writable after the upgrade");

        conn.execute(
            "UPDATE videos SET thumbnail_ready = 1 WHERE id = 'nonexistent'",
            [],
        )
        .expect("thumbnail_ready column should be queryable/writable after the upgrade");
    }
}

/// Reproduces upgrading a real database file at schema_version=6 (every
/// migration through v6's path_collisions table already applied, but before
/// v7's thumbnail_attempts/metadata_attempts columns exist) -- the actual
/// upgrade path an existing user's app.db takes. Confirms both that
/// migration 0007 applies cleanly on top of a real v6 database and that the
/// two new columns' `DEFAULT 0` means an already-registered video (inserted
/// before this migration ever ran) reads back as "no attempts yet" rather
/// than NULL.
#[test]
fn migrating_from_a_real_v6_database_reaches_v7_and_initializes_attempts_columns_to_zero() {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");

    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        apply_up_to(&conn, 6);

        // A video row registered before migration 0007 ever ran -- its two
        // new columns must still read back as 0 (the `DEFAULT 0` backfill),
        // not NULL, once the upgrade below runs.
        conn.execute(
            "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status)
             VALUES ('pre-v7', 'D:\\videos\\a.mp4', 'a.mp4', 1024, 'qh', 'online')",
            [],
        )
        .unwrap();
    }

    let db = db::init(&db_path)
        .expect("db::init should upgrade a v6-only database to the current version");
    let conn = db.writer.lock().unwrap();

    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        version,
        latest_schema_version(),
        "should also pick up every later migration"
    );

    let (thumbnail_attempts, metadata_attempts): (i64, i64) = conn
        .query_row(
            "SELECT thumbnail_attempts, metadata_attempts FROM videos WHERE id = 'pre-v7'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        thumbnail_attempts, 0,
        "thumbnail_attempts must default to 0 for a pre-migration row"
    );
    assert_eq!(
        metadata_attempts, 0,
        "metadata_attempts must default to 0 for a pre-migration row"
    );
}

/// Reproduces upgrading a real database file at schema_version=7 (every
/// migration through v7's thumbnail_attempts/metadata_attempts columns
/// already applied, but before v8's thumbnail_ready column exists) -- the
/// actual upgrade path an existing user's app.db takes.
/// Confirms migration 0008 applies cleanly on top of a real v7 database, and
/// that a video row registered (and already having a generated thumbnail on
/// disk) before this migration ever ran still reads `thumbnail_ready` back
/// as 0 (the `DEFAULT 0` backfill), not NULL or 1 -- correcting that is
/// `thumbnail::worker::list_videos_missing_thumbnails`'s job at the next
/// scan/startup, not this migration's.
#[test]
fn migrating_from_a_real_v7_database_reaches_v8_and_initializes_thumbnail_ready_to_zero() {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");

    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        apply_up_to(&conn, 7);

        conn.execute(
            "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status)
             VALUES ('pre-v8', 'D:\\videos\\a.mp4', 'a.mp4', 1024, 'qh', 'online')",
            [],
        )
        .unwrap();
    }

    let db = db::init(&db_path)
        .expect("db::init should upgrade a v7-only database to the current version");
    let conn = db.writer.lock().unwrap();

    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, latest_schema_version());

    let thumbnail_ready: i64 = conn
        .query_row(
            "SELECT thumbnail_ready FROM videos WHERE id = 'pre-v8'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        thumbnail_ready, 0,
        "thumbnail_ready must default to 0 for a pre-migration row, even if its thumbnail file already exists on disk"
    );

    conn.execute(
        "UPDATE videos SET thumbnail_ready = 1 WHERE id = 'pre-v8'",
        [],
    )
    .expect("thumbnail_ready column should be queryable/writable after the upgrade");
}

/// Upgrading a real v8 database: rows recorded before `dismissed` existed must
/// stay visible (DEFAULT 0), otherwise the upgrade would silently empty the
/// user's unregistered-files list.
#[test]
fn migrating_from_a_real_v8_database_keeps_existing_skipped_rows_visible() {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");

    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        apply_up_to(&conn, 8);
        conn.execute(
            "INSERT INTO skipped_files (file_path, file_name, reason, detected_char)
             VALUES ('D:\\v\\a\u{2460}.mp4', 'a\u{2460}.mp4', 'machine_dependent_char', '\u{2460}')",
            [],
        )
        .unwrap();
    }

    let db = db::init(&db_path)
        .expect("db::init should upgrade a v8-only database to the current version");

    let conn = db.writer.lock().unwrap();
    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, latest_schema_version());
    let dismissed: i64 = conn
        .query_row("SELECT dismissed FROM skipped_files", [], |r| r.get(0))
        .unwrap();
    assert_eq!(dismissed, 0);
    let payload: Option<String> = conn
        .query_row("SELECT wb_payload FROM skipped_files", [], |r| r.get(0))
        .unwrap();
    assert_eq!(payload, None);
    drop(conn);

    let rows = db::queries::list_skipped_files(&db.read_pool).unwrap();
    assert_eq!(rows.len(), 1);
}

#[test]
fn migration_v2_adds_a_nullable_mtime_column() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();

    let mut stmt = conn.prepare("PRAGMA table_info(videos)").unwrap();
    let columns: Vec<(String, i64)> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    // column 1 = name, column 3 = notnull (PRAGMA table_info's own columns)
    let mtime_col = columns
        .iter()
        .find(|(name, _)| name == "mtime")
        .expect("videos.mtime column should exist after migration v2");
    assert_eq!(mtime_col.1, 0, "videos.mtime must be nullable");
}

/// Builds a database with migrations 1..=9 applied (the last schema before
/// `tags.sort_order`), seeded with `tags` and an optional raw
/// `tag_bar_pinned_tags` setting value, then upgrades it through `db::init`.
fn upgrade_v9_with(
    tags: &[(i64, &str)],
    pinned_setting: Option<&str>,
) -> (tempfile::TempDir, db::Db) {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        apply_up_to(&conn, 9);
        for (id, name) in tags {
            conn.execute(
                "INSERT INTO tags (id, name) VALUES (?1, ?2)",
                rusqlite::params![id, name],
            )
            .unwrap();
        }
        if let Some(value) = pinned_setting {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('tag_bar_pinned_tags', ?1)",
                [value],
            )
            .unwrap();
        }
    }
    let db = db::init(&db_path).expect("db::init should upgrade a v9 database");
    (dir, db)
}

fn pinned_setting_rows(db: &db::Db) -> i64 {
    let conn = db.writer.lock().unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM settings WHERE key = 'tag_bar_pinned_tags'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

const TAGS: [(i64, &str); 4] = [(1, "a"), (2, "b"), (3, "c"), (4, "d")];

#[test]
fn migration_v10_without_pinned_setting_leaves_every_sort_order_null() {
    let (_dir, db) = upgrade_v9_with(&TAGS, None);
    assert_eq!(
        sort_orders(&db.writer.lock().unwrap()),
        vec![(1, None), (2, None), (3, None), (4, None)]
    );
}

#[test]
fn migration_v10_carries_pinned_tags_over_in_pinned_order_and_removes_the_setting() {
    let (_dir, db) = upgrade_v9_with(&TAGS, Some("[3,1]"));
    assert_eq!(
        sort_orders(&db.writer.lock().unwrap()),
        vec![(1, Some(1)), (2, None), (3, Some(0)), (4, None)]
    );
    assert_eq!(pinned_setting_rows(&db), 0);
    assert_eq!(tag_names(&db), ["c", "a", "b", "d"]);
}

#[test]
fn migration_v10_ignores_a_malformed_or_non_array_pinned_setting() {
    for raw in ["not json", "{\"a\":1}", "3", "\"[3,1]\"", "", "null"] {
        let (_dir, db) = upgrade_v9_with(&TAGS, Some(raw));
        assert_eq!(
            sort_orders(&db.writer.lock().unwrap()),
            vec![(1, None), (2, None), (3, None), (4, None)],
            "setting {raw:?}"
        );
        assert_eq!(pinned_setting_rows(&db), 0, "setting {raw:?}");
    }
}

#[test]
fn migration_v10_skips_unknown_ids_and_non_integers_and_keeps_first_of_duplicates() {
    let (_dir, db) = upgrade_v9_with(&TAGS, Some("[99,3,\"1\",3,null,2.5,1,4]"));
    // Positions: 99@0 (no tag), 3@1, "1"@2 (string), 3@3 (dup), null@4,
    // 2.5@5, 1@6, 4@7. Only the order has to be preserved, not contiguity.
    assert_eq!(
        sort_orders(&db.writer.lock().unwrap()),
        vec![(1, Some(6)), (2, None), (3, Some(1)), (4, Some(7))]
    );
    assert_eq!(tag_names(&db), ["c", "a", "d", "b"]);
}

#[test]
fn migration_v10_adds_a_nullable_sort_order_column_without_default() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    let mut stmt = conn.prepare("PRAGMA table_info(tags)").unwrap();
    let cols: Vec<(String, i64, Option<String>)> = stmt
        .query_map([], |r| Ok((r.get(1)?, r.get(3)?, r.get(4)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let col = cols
        .iter()
        .find(|(name, _, _)| name == "sort_order")
        .expect("tags.sort_order should exist");
    assert_eq!((col.1, &col.2), (0, &None));
}

/// Upgrading a real v10 database adds the new table without touching the data
/// that was already there.
#[test]
fn migrating_from_a_real_v10_database_adds_wb_unmatched_thumbnails_and_keeps_data() {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        apply_up_to(&conn, 10);
        conn.execute("INSERT INTO tags (id, name) VALUES (1, 'keep')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status)
             VALUES ('pre-v11', 'D:/videos/a.mp4', 'a.mp4', 1024, 'qh', 'online')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO video_tags (video_id, tag_id) VALUES ('pre-v11', 1)",
            [],
        )
        .unwrap();
    }

    let db = db::init(&db_path).expect("db::init should upgrade a v10 database");
    let conn = db.writer.lock().unwrap();

    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, latest_schema_version());

    let tag: String = conn
        .query_row("SELECT name FROM tags WHERE id = 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tag, "keep");
    let video: String = conn
        .query_row(
            "SELECT file_path FROM videos WHERE id = 'pre-v11'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(video, "D:/videos/a.mp4");
    let links: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM video_tags WHERE video_id = 'pre-v11' AND tag_id = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(links, 1);

    conn.execute(
        "INSERT INTO wb_unmatched_thumbnails (folder_path, file_name, kind, thum_layout)
         VALUES ('D:/thumb', 'a.#.jpg', 'empty_hash', NULL)",
        [],
    )
    .expect("wb_unmatched_thumbnails should be writable and thum_layout nullable");
    let duplicate = conn.execute(
        "INSERT INTO wb_unmatched_thumbnails (folder_path, file_name, kind)
         VALUES ('D:/thumb', 'a.#.jpg', 'empty_hash')",
        [],
    );
    assert!(
        duplicate.is_err(),
        "(folder_path, file_name) must be the primary key"
    );
}

/// Upgrading a real v11 database drops the three redundant indexes and leaves
/// the data and both UNIQUE constraints (which the automatic indexes enforce)
/// intact.
#[test]
fn migrating_from_a_real_v11_database_drops_unused_indexes_and_keeps_data_and_constraints() {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        apply_up_to(&conn, 11);
        let before = object_names(&conn, "index");
        for dropped in DROPPED_INDEXES {
            assert!(
                before.iter().any(|i| i == dropped),
                "{dropped} should exist before the upgrade, got {before:?}"
            );
        }
        for (id, path) in [("a", "D:/v/a.mp4"), ("b", "D:/v/b.mp4")] {
            conn.execute(
                "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, full_hash, status)
                 VALUES (?1, ?2, 'n.mp4', 1024, 'qh', 'fh', 'online')",
                [id, path],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO path_collisions (video_id, colliding_video_id, attempted_path)
             VALUES ('a', 'b', 'D:/v/b.mp4')",
            [],
        )
        .unwrap();
    }

    let db = db::init(&db_path).expect("db::init should upgrade a v11 database");
    let conn = db.writer.lock().unwrap();

    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, latest_schema_version());

    let indexes = object_names(&conn, "index");
    for dropped in DROPPED_INDEXES {
        assert!(
            !indexes.iter().any(|i| i == dropped),
            "{dropped} should be gone, got {indexes:?}"
        );
    }
    for kept in ["idx_videos_status", "idx_videos_name"] {
        assert!(
            indexes.iter().any(|i| i == kept),
            "{kept} should be kept, got {indexes:?}"
        );
    }

    let videos: i64 = conn
        .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
        .unwrap();
    assert_eq!(videos, 2);
    let full_hash: String = conn
        .query_row("SELECT full_hash FROM videos WHERE id = 'a'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(full_hash, "fh");
    let collisions: i64 = conn
        .query_row("SELECT COUNT(*) FROM path_collisions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(collisions, 1);

    let duplicate_path = conn.execute(
        "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status)
         VALUES ('c', 'D:/v/a.mp4', 'n.mp4', 1, 'q', 'online')",
        [],
    );
    assert!(
        duplicate_path.is_err(),
        "videos.file_path must stay UNIQUE without idx_videos_path"
    );
    let duplicate_pair = conn.execute(
        "INSERT INTO path_collisions (video_id, colliding_video_id, attempted_path)
         VALUES ('a', 'b', 'x')",
        [],
    );
    assert!(
        duplicate_pair.is_err(),
        "(video_id, colliding_video_id) must stay UNIQUE without idx_path_collisions_video"
    );
}

const DROPPED_INDEXES: [&str; 3] = [
    "idx_path_collisions_video",
    "idx_videos_full_hash",
    "idx_videos_path",
];

/// Upgrading a real v12 database adds tag_groups and tags.group_id without
/// moving any existing tag.
#[test]
fn migrating_from_a_real_v12_database_adds_tag_groups_and_keeps_tag_order() {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        apply_up_to(&conn, 12);
        for (id, name, order) in [(1, "a", None), (2, "b", Some(1)), (3, "c", Some(0))] {
            conn.execute(
                "INSERT INTO tags (id, name, sort_order) VALUES (?1, ?2, ?3)",
                rusqlite::params![id, name, order],
            )
            .unwrap();
        }
    }

    let db = db::init(&db_path).expect("db::init should upgrade a v12 database");
    assert_eq!(tag_names(&db), ["c", "b", "a"]);
    let conn = db.writer.lock().unwrap();

    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, latest_schema_version());
    assert_eq!(version, 13);

    let ungrouped: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tags WHERE group_id IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ungrouped, 3, "existing tags must stay at the top level");
    assert_eq!(
        sort_orders(&conn),
        vec![(1, None), (2, Some(1)), (3, Some(0))]
    );

    conn.execute("INSERT INTO tag_groups (name) VALUES ('g')", [])
        .unwrap();
    assert!(
        conn.execute("INSERT INTO tag_groups (name) VALUES ('g')", [])
            .is_err(),
        "group names must be unique"
    );
    let sort: Option<i64> = conn
        .query_row(
            "SELECT sort_order FROM tag_groups WHERE name = 'g'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sort, None);
}

/// A deleted group's id must not be handed out again: collapsed state is
/// stored by group id.
#[test]
fn tag_group_ids_are_never_reused() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    conn.execute("INSERT INTO tag_groups (name) VALUES ('a')", [])
        .unwrap();
    let first = conn.last_insert_rowid();
    conn.execute("DELETE FROM tag_groups WHERE id = ?1", [first])
        .unwrap();
    conn.execute("INSERT INTO tag_groups (name) VALUES ('b')", [])
        .unwrap();
    assert!(conn.last_insert_rowid() > first);
}
