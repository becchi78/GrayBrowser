//! SQLite access: a single writer connection (WAL mode, migrated at startup)
//! plus an `r2d2` read-only connection pool.

pub mod backup;
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
    let existed = db_path.is_file();
    // Before anything that writes (the WAL switch, the backup, migrations):
    // a database newer than this app must be left byte-for-byte untouched.
    // The check uses its own read-only connection, closed before the writer
    // opens: a read-write open of a WAL database can checkpoint on close and
    // rewrite the main file. `immutable=1` is not an option because it would
    // ignore uncheckpointed WAL content and read a stale version.
    if existed {
        let probe = rusqlite::Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let current = backup::current_schema_version(&probe)?;
        gb_core::migrations::check_schema_compat(
            current,
            gb_core::migrations::latest_version(migrations::MIGRATIONS),
        )
        .map_err(anyhow::Error::new)?;
    }
    let mut conn = rusqlite::Connection::open(db_path)?;
    disable_foreign_keys(&mut conn)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    if existed {
        backup_before_migration(&conn, db_path);
    }
    migrations::run_migrations(&mut conn)?;
    let writer = Arc::new(Mutex::new(conn));

    let manager =
        r2d2_sqlite::SqliteConnectionManager::file(db_path).with_init(disable_foreign_keys);
    let read_pool = r2d2::Pool::builder().build(manager)?;

    Ok(Db { writer, read_pool })
}

/// Takes a `premigrate` backup when `conn` holds an existing database with
/// migrations still to apply. A failure is only logged: the migration runs in
/// a transaction and rolls back on error, and refusing to start (e.g. because
/// the disk is full) would be worse than starting without the extra copy.
fn backup_before_migration(conn: &rusqlite::Connection, db_path: &Path) {
    let current = match backup::current_schema_version(conn) {
        Ok(v) => v,
        Err(e) => {
            log::error!("pre-migration backup skipped: cannot read schema version: {e}");
            return;
        }
    };
    // Version 0 is an empty file with nothing worth preserving.
    if current == 0
        || gb_core::migrations::pending_migrations(current, migrations::MIGRATIONS).is_empty()
    {
        return;
    }
    let Some(app_dir) = db_path.parent() else {
        return;
    };
    match backup::create_backup(
        conn,
        &backup::backups_dir(app_dir),
        gb_core::backup::BackupKind::Premigrate,
        backup::unix_now(),
    ) {
        Ok(path) => log::info!("pre-migration backup written: {}", path.display()),
        Err(e) => log::error!("pre-migration backup failed: {e:#}"),
    }
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
/// with a minimal `videos` row. Shared by the `#[cfg(test)] mod tests` of
/// `scan`, `watch`, `thumbnail::worker`, `metadata::worker`, `dedup`,
/// `wb_import::pipeline`, `db::queries`, and
/// `commands::generation_retry_cmds` via `use crate::db::test_support::*;`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use gb_core::migrations::SchemaTooNew;

    fn newer_db(dir: &Path, version: i64) -> std::path::PathBuf {
        let path = dir.join("app.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            [version],
        )
        .unwrap();
        path
    }

    #[test]
    fn init_refuses_a_newer_database_without_touching_it() {
        let dir = tempfile::tempdir().unwrap();
        let latest = gb_core::migrations::latest_version(migrations::MIGRATIONS);
        let path = newer_db(dir.path(), latest + 1);
        let before = std::fs::read(&path).unwrap();
        let files_before: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();

        let err = init(&path).err().expect("must refuse");
        let too_new = err.downcast_ref::<SchemaTooNew>().expect("SchemaTooNew");
        assert_eq!(too_new.db_version, latest + 1);
        assert_eq!(too_new.app_version, latest);

        assert_eq!(std::fs::read(&path).unwrap(), before);
        let files_after: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(files_after, files_before);
        assert!(!backup::backups_dir(dir.path()).exists());
    }

    #[test]
    fn init_refuses_a_newer_wal_database_without_touching_the_main_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        let latest = gb_core::migrations::latest_version(migrations::MIGRATIONS);
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        // Keep the connection open so the version row stays in the -wal file.
        conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at TEXT);",
        )
        .unwrap();
        conn.execute("INSERT INTO schema_version (version) VALUES (?1)", [999])
            .unwrap();
        std::mem::forget(conn);
        let before = std::fs::read(&path).unwrap();

        let err = init(&path).err().expect("must refuse");
        let too_new = err.downcast_ref::<SchemaTooNew>().expect("SchemaTooNew");
        assert_eq!(too_new.db_version, 999);
        assert_eq!(too_new.app_version, latest);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!backup::backups_dir(dir.path()).exists());
    }

    #[test]
    fn init_opens_current_and_empty_databases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        init(&path).unwrap();
        // Reopening a database at the latest version is still fine.
        init(&path).unwrap();

        let empty = dir.path().join("empty.db");
        rusqlite::Connection::open(&empty).unwrap();
        init(&empty).unwrap();
    }
}
