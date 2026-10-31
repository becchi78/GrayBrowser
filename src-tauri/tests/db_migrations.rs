//! Integration tests against a real tempdir-backed SQLite file (not
//! `:memory:` -- WAL mode only means something for a file-backed database).

use graybrowser_lib::db;

fn init_temp_db() -> (tempfile::TempDir, db::Db) {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");
    let db = db::init(&db_path).expect("db::init should succeed against a fresh tempdir file");
    (dir, db)
}

#[test]
fn creates_all_seven_tables_and_ten_indexes() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();

    // sqlite_sequence is SQLite's own internal bookkeeping table, auto-created
    // because tags/skipped_files/path_collisions use AUTOINCREMENT -- not one
    // of our 7.
    let mut table_stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name != 'sqlite_sequence' ORDER BY name")
        .unwrap();
    let tables: Vec<String> = table_stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        tables,
        vec![
            "path_collisions",
            "schema_version",
            "settings",
            "skipped_files",
            "tags",
            "video_tags",
            "videos"
        ]
    );

    let mut index_stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND name LIKE 'idx_%' ORDER BY name")
        .unwrap();
    let indexes: Vec<String> = index_stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        indexes,
        vec![
            "idx_path_collisions_colliding",
            "idx_path_collisions_video",
            "idx_video_tags_tag",
            "idx_videos_created_at",
            "idx_videos_full_hash",
            "idx_videos_name",
            "idx_videos_path",
            "idx_videos_quick_hash",
            "idx_videos_rating",
            "idx_videos_status",
        ]
    );
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
    // Ten migrations exist (v1 initial schema, v2
    // mtime, v3 kana/roma, v4 video metadata, v5 sort indexes, v6
    // path_collisions, v7 generation retry columns, v8 thumbnail_ready,
    // v9 skipped_files.dismissed, v10 tags.sort_order) -- a fresh DB should land on all ten, and
    // re-running must not insert extra schema_version rows for any of them.
    let version_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM schema_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version_count, 10);
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

/// Reproduces upgrading a real database file at schema_version=2 (no
/// kana/roma/metadata columns, v5 indexes, v6 path_collisions table, or v7
/// retry-attempt columns) to confirm the full v3-v7 upgrade path a returning
/// user's app.db actually takes, not just a fresh DB applying every migration
/// together.
#[test]
fn migrating_from_a_real_v2_database_reaches_the_current_version_cleanly() {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");

    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(include_str!("../migrations/0001_initial.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0002_add_mtime.sql"))
            .unwrap();
        conn.execute("INSERT INTO schema_version (version) VALUES (1)", [])
            .unwrap();
        conn.execute("INSERT INTO schema_version (version) VALUES (2)", [])
            .unwrap();
    }

    let db = db::init(&db_path)
        .expect("db::init should upgrade a v2-only database to the current version");
    let conn = db.writer.lock().unwrap();

    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 10);

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
        conn.execute_batch(include_str!("../migrations/0001_initial.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0002_add_mtime.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0003_add_kana_roma.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0004_add_video_metadata.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0005_add_sort_indexes.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0006_add_path_collisions.sql"))
            .unwrap();
        for version in 1..=6 {
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [version],
            )
            .unwrap();
        }

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
    assert_eq!(version, 10, "should also pick up v8 to v10");

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
        conn.execute_batch(include_str!("../migrations/0001_initial.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0002_add_mtime.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0003_add_kana_roma.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0004_add_video_metadata.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0005_add_sort_indexes.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0006_add_path_collisions.sql"))
            .unwrap();
        conn.execute_batch(include_str!(
            "../migrations/0007_add_generation_retry_columns.sql"
        ))
        .unwrap();
        for version in 1..=7 {
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [version],
            )
            .unwrap();
        }

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
    assert_eq!(version, 10);

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
        conn.execute_batch(include_str!("../migrations/0001_initial.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0002_add_mtime.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0003_add_kana_roma.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0004_add_video_metadata.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0005_add_sort_indexes.sql"))
            .unwrap();
        conn.execute_batch(include_str!("../migrations/0006_add_path_collisions.sql"))
            .unwrap();
        conn.execute_batch(include_str!(
            "../migrations/0007_add_generation_retry_columns.sql"
        ))
        .unwrap();
        conn.execute_batch(include_str!("../migrations/0008_add_thumbnail_ready.sql"))
            .unwrap();
        for version in 1..=8 {
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [version],
            )
            .unwrap();
        }
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
    assert_eq!(version, 10);
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

/// Reproduces upgrading a real database file at schema_version=1 (no
/// `mtime` column) all the way to the current schema, rather than relying
/// on a fresh DB always applying every migration together -- an oldest-
/// possible-starting-point variant of the actual upgrade path a returning
/// user's `app.db` takes (see also
/// `migrating_from_a_real_v2_database_reaches_the_current_version_cleanly`
/// for the v2-start variant).
#[test]
fn migrating_from_a_real_v1_database_reaches_the_current_version_cleanly() {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");

    {
        // 0001_initial.sql itself creates schema_version (IF NOT EXISTS,
        // since run_migrations normally bootstraps it separately before the
        // first migration ever runs) -- so it already exists after this
        // batch, and only needs its v1 row inserted.
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(include_str!("../migrations/0001_initial.sql"))
            .unwrap();
        conn.execute("INSERT INTO schema_version (version) VALUES (1)", [])
            .unwrap();
    }

    let db = db::init(&db_path)
        .expect("db::init should upgrade a v1-only database to the current version");
    let conn = db.writer.lock().unwrap();

    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 10);

    // The column must be usable, not just present in the schema.
    conn.execute("UPDATE videos SET mtime = 123 WHERE id = 'nonexistent'", [])
        .expect("mtime column should be queryable/writable after the upgrade");
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
        for sql in [
            include_str!("../migrations/0001_initial.sql"),
            include_str!("../migrations/0002_add_mtime.sql"),
            include_str!("../migrations/0003_add_kana_roma.sql"),
            include_str!("../migrations/0004_add_video_metadata.sql"),
            include_str!("../migrations/0005_add_sort_indexes.sql"),
            include_str!("../migrations/0006_add_path_collisions.sql"),
            include_str!("../migrations/0007_add_generation_retry_columns.sql"),
            include_str!("../migrations/0008_add_thumbnail_ready.sql"),
            include_str!("../migrations/0009_add_skipped_files_dismissed.sql"),
        ] {
            conn.execute_batch(sql).unwrap();
        }
        for version in 1..=9 {
            conn.execute(
                "INSERT INTO schema_version (version) VALUES (?1)",
                [version],
            )
            .unwrap();
        }
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

fn sort_orders(db: &db::Db) -> Vec<(i64, Option<i64>)> {
    let conn = db.writer.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT id, sort_order FROM tags ORDER BY id")
        .unwrap();
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    rows
}

fn tag_names(db: &db::Db) -> Vec<String> {
    db::queries::list_all_tags(&db.read_pool)
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect()
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
fn migration_v10_bundled_sqlite_has_json_each_and_json_valid() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    let sum: i64 = conn
        .query_row(
            "SELECT SUM(value) FROM json_each('[1,2,3]') WHERE json_valid('[1,2,3]')",
            [],
            |r| r.get(0),
        )
        .expect("the bundled SQLite must provide json_each and json_valid");
    assert_eq!(sum, 6);
}

#[test]
fn migration_v10_without_pinned_setting_leaves_every_sort_order_null() {
    let (_dir, db) = upgrade_v9_with(&TAGS, None);
    assert_eq!(
        sort_orders(&db),
        vec![(1, None), (2, None), (3, None), (4, None)]
    );
}

#[test]
fn migration_v10_carries_pinned_tags_over_in_pinned_order_and_removes_the_setting() {
    let (_dir, db) = upgrade_v9_with(&TAGS, Some("[3,1]"));
    assert_eq!(
        sort_orders(&db),
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
            sort_orders(&db),
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
        sort_orders(&db),
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
