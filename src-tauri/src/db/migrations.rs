//! Migration runner: applies `gb_core::migrations`-ordered SQL against a real
//! `rusqlite::Connection`. The ordering/selection logic itself lives in
//! `gb_core::migrations::pending_migrations` (pure); this module is the
//! adapter that actually executes SQL and owns the transaction.

pub const MIGRATIONS: &[gb_core::migrations::Migration] = &[
    gb_core::migrations::Migration {
        version: 1,
        description: "initial schema: videos, tags, video_tags, schema_version, skipped_files, settings + 6 indexes",
        sql: include_str!("../../migrations/0001_initial.sql"),
    },
    gb_core::migrations::Migration {
        version: 2,
        description: "add videos.mtime for cheap change-detection filtering",
        sql: include_str!("../../migrations/0002_add_mtime.sql"),
    },
    gb_core::migrations::Migration {
        version: 3,
        description: "add videos.kana/roma, a nullable receiving bin for .wb import",
        sql: include_str!("../../migrations/0003_add_kana_roma.sql"),
    },
    gb_core::migrations::Migration {
        version: 4,
        description: "add ffprobe-derived video metadata columns + probed_at",
        sql: include_str!("../../migrations/0004_add_video_metadata.sql"),
    },
    gb_core::migrations::Migration {
        version: 5,
        description: "add sort-order indexes on created_at/rating, verified used via EXPLAIN QUERY PLAN",
        sql: include_str!("../../migrations/0005_add_sort_indexes.sql"),
    },
    gb_core::migrations::Migration {
        version: 6,
        description: "add path_collisions table to persist duplicate candidates from both 経路X (register_new_path's UNIQUE path collision) and 経路Y (reconcile_known_path's coincidental rehash match)",
        sql: include_str!("../../migrations/0006_add_path_collisions.sql"),
    },
    gb_core::migrations::Migration {
        version: 7,
        description: "add thumbnail_attempts/metadata_attempts to videos for retry-limit tracking",
        sql: include_str!("../../migrations/0007_add_generation_retry_columns.sql"),
    },
    gb_core::migrations::Migration {
        version: 8,
        description: "add videos.thumbnail_ready to remove the per-row filesystem stat() call from list_videos's hot path",
        sql: include_str!("../../migrations/0008_add_thumbnail_ready.sql"),
    },
];

/// Applies any pending migrations to `conn` inside a single transaction.
/// Re-running this against an already-migrated connection is a no-op.
pub fn run_migrations(conn: &mut rusqlite::Connection) -> rusqlite::Result<()> {
    // Bootstrap: schema_version must exist before we can read the current
    // version from it, even on a brand-new empty database file. The initial
    // migration's own `CREATE TABLE IF NOT EXISTS schema_version` (in
    // 0001_initial.sql) is therefore a no-op the first time it runs.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER PRIMARY KEY, applied_at DATETIME DEFAULT CURRENT_TIMESTAMP);",
    )?;

    let current: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get(0),
    )?;

    let pending = gb_core::migrations::pending_migrations(current, MIGRATIONS);
    if pending.is_empty() {
        return Ok(());
    }

    let tx = conn.transaction()?;
    for migration in &pending {
        tx.execute_batch(migration.sql)?;
        tx.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            [migration.version],
        )?;
    }
    // If anything above returned Err, `tx` is dropped here without being
    // committed and rusqlite rolls it back automatically -- no partial
    // schema is left behind.
    tx.commit()?;
    Ok(())
}
