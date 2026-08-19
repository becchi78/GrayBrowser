//! SQLite access: a single writer connection (WAL mode, migrated at startup)
//! plus an `r2d2` read-only connection pool.

pub mod migrations;
pub mod queries;

use std::path::Path;
use std::sync::{Arc, Mutex};

/// `writer` and `read_pool` are both cheap to clone (an `Arc<Mutex<..>>` and
/// an internally-`Arc`-based `r2d2::Pool`), so `Db` itself derives `Clone` --
/// this lets background work outside a command handler's lifetime (the
/// thumbnail worker pool, spawned from `.setup()` or after `start_scan`)
/// hold its own handle to the same underlying connections.
#[derive(Clone)]
pub struct Db {
    pub writer: Arc<Mutex<rusqlite::Connection>>,
    pub read_pool: r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
}

/// Opens (creating if needed) the database at `db_path`, sets WAL mode,
/// applies any pending migrations, and builds the read-only connection pool.
///
/// Ordering matters: WAL is set on the writer connection *before* migrations
/// run, the writer connection is kept open (never closed/reopened) as the
/// single writer for the app's lifetime, and the read pool is only built
/// *after* migrations succeed -- a half-migrated database should never be
/// handed out to readers.
pub fn init(db_path: &Path) -> anyhow::Result<Db> {
    let mut conn = rusqlite::Connection::open(db_path)?;
    disable_foreign_keys(&mut conn)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    migrations::run_migrations(&mut conn)?;
    let writer = Arc::new(Mutex::new(conn));

    let manager =
        r2d2_sqlite::SqliteConnectionManager::file(db_path).with_init(disable_foreign_keys);
    let read_pool = r2d2::Pool::builder().build(manager)?;

    Ok(Db { writer, read_pool })
}

/// Explicitly turns `PRAGMA foreign_keys` off.
///
/// This project's design intent is to leave FK enforcement disabled
/// permanently (referential integrity is enforced at the application layer
/// instead) -- but the bundled SQLite build pulled in
/// by `rusqlite`'s `bundled` feature turned out to default this pragma to
/// **on**, not SQLite's classic off-by-default. `db_migrations.rs`'s
/// `foreign_keys_pragma_is_not_forced_on` test caught this. Since "don't
/// enable foreign_keys" was the actual intent (not "don't touch the
/// pragma"), every connection -- writer and each pooled reader -- explicitly
/// sets it off rather than relying on the (here, wrong) assumption that
/// leaving it untouched means off.
fn disable_foreign_keys(conn: &mut rusqlite::Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "foreign_keys", false)
}

/// Shared test-only helpers for standing up a throwaway `Db` and seeding it
/// with a minimal `videos` row, previously copy-pasted (with minor naming
/// drift, e.g. `temp_db` vs. `init_temp_db`) across the `#[cfg(test)] mod
/// tests` of `scan`, `watch`, `thumbnail::worker`, `metadata::worker`,
/// `dedup`, `wb_import::pipeline`, `db::queries`, and
/// `commands::generation_retry_cmds`. Consolidated here so every one of
/// those modules can `use crate::db::test_support::*;` instead.
#[cfg(test)]
pub mod test_support {
    use super::Db;

    /// Opens a fresh, migrated `Db` backed by a temp directory. The
    /// `TempDir` must be kept alive by the caller for as long as `db` is
    /// used -- it deletes the directory (and the SQLite file within it) on
    /// drop.
    pub fn init_temp_db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::init(&dir.path().join("app.db")).unwrap();
        (dir, db)
    }

    /// Inserts a minimal `online` video row (`file_name` fixed to `v.mp4`,
    /// `file_size` `1`, `quick_hash` `'h'`) with the given `id`/`file_path`,
    /// for tests that only care about a video's identity/path, not its
    /// other columns.
    pub fn insert_test_video(db: &Db, id: &str, file_path: &str) {
        let conn = db.writer.lock().unwrap();
        conn.execute(
            "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status) VALUES (?1, ?2, 'v.mp4', 1, 'h', 'online')",
            rusqlite::params![id, file_path],
        )
        .unwrap();
    }
}
