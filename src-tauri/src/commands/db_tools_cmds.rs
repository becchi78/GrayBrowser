//! Developer-mode database tools: an integrity check, size statistics and an
//! optimisation (`VACUUM`). The frontend passes no SQL and no paths; each
//! command runs one fixed operation. Only `run_vacuum` changes anything, so it
//! alone repeats the developer-mode check here.
//!
//! The decisions live in `*_impl` functions that take the connection (and the
//! disk-space port and the backup step as arguments), so they run in tests
//! without a Tauri runtime or a real disk.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use gb_core::db_tools::{
    describe_insufficient_space, format_bytes, format_integrity_result, reclaimable_bytes,
    vacuum_space_check,
};
use gb_core::ports::disk_space::DiskSpace;
use rusqlite::Connection;
use serde::Serialize;
use tauri::State;

use super::developer_cmds::ensure_developer_mode;
use crate::adapters::disk_space::RealDiskSpace;
use crate::db::{backup, Db};

/// How long `run_vacuum` waits for the writer connection. The scan and the
/// other writers hold it only for one statement or one small transaction at a
/// time, so a wait of this length means something is stuck; giving up with a
/// message beats a dialog that never returns.
const WRITER_WAIT: Duration = Duration::from_secs(30);

#[derive(Debug, Serialize)]
pub struct IntegrityDto {
    pub ok: bool,
    pub messages: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DbStatsDto {
    pub file_size: u64,
    pub file_size_text: String,
    pub page_count: u64,
    pub freelist_count: u64,
    pub page_size: u64,
    /// What `VACUUM` could give back, from the free pages.
    pub reclaimable_text: String,
}

#[derive(Debug, Serialize)]
pub struct VacuumDto {
    pub size_before: u64,
    /// `None` when the size could not be read after the `VACUUM`, which has
    /// succeeded all the same.
    pub size_after: Option<u64>,
    pub size_before_text: String,
    pub size_after_text: String,
    /// File name only, inside the app's `backups` folder.
    pub backup_file: String,
}

fn pragma_u64(conn: &Connection, pragma: &str) -> rusqlite::Result<u64> {
    let v: i64 = conn.query_row(&format!("PRAGMA {pragma}"), [], |r| r.get(0))?;
    Ok(u64::try_from(v).unwrap_or(0))
}

/// A database too damaged to answer the query at all is reported as a failed
/// check carrying SQLite's error, the same as a damaged one that answers.
fn run_integrity_check_impl(conn: &Connection) -> IntegrityDto {
    let rows: rusqlite::Result<Vec<String>> = conn
        .prepare("PRAGMA integrity_check")
        .and_then(|mut stmt| stmt.query_map([], |r| r.get(0))?.collect());
    let report = match rows {
        Ok(rows) => format_integrity_result(&rows),
        Err(e) => format_integrity_result(&[e.to_string()]),
    };
    IntegrityDto {
        ok: report.ok,
        messages: report.messages,
    }
}

fn get_db_stats_impl(conn: &Connection, file_size: u64) -> Result<DbStatsDto, String> {
    let stat = |p: &str| pragma_u64(conn, p).map_err(|e| e.to_string());
    let page_size = stat("page_size")?;
    let freelist_count = stat("freelist_count")?;
    Ok(DbStatsDto {
        file_size,
        file_size_text: format_bytes(file_size),
        page_count: stat("page_count")?,
        freelist_count,
        page_size,
        reclaimable_text: format_bytes(reclaimable_bytes(page_size, freelist_count)),
    })
}

fn lock_within(
    writer: &Mutex<Connection>,
    wait: Duration,
) -> Result<MutexGuard<'_, Connection>, String> {
    let deadline = Instant::now() + wait;
    loop {
        match writer.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err("データベースの書き込み接続が異常な状態です。".to_string())
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Err(
                        "他の処理がデータベースを使用中のため、開始できませんでした。\
                                しばらくしてからやり直してください。"
                            .to_string(),
                    );
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// Moves what the WAL holds into the main file and truncates the WAL, so the
/// main file's size reflects the real content. Best effort: a reader that is
/// open at that moment can make it return without finishing.
fn checkpoint(conn: &Connection) {
    if let Err(e) = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())) {
        log::warn!("wal checkpoint around VACUUM failed: {e}");
    }
}

fn file_size(path: &Path) -> Result<u64, String> {
    std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| e.to_string())
}

/// The order is the safety argument: developer mode, free space, a verified
/// backup, and only then `VACUUM`. A failure at any earlier step returns
/// before the database is touched. The writer lock is taken once and kept
/// through the backup and the `VACUUM`, so the backup is exactly the state
/// that is about to be rewritten and no write can slip in between; other
/// writes (scans, edits) wait meanwhile.
///
/// Holding the lock for the whole run also means a synchronous write command
/// (e.g. tag assignment, which runs on the main thread) would block until the
/// run ends. That is acceptable only because the dialog shows a modal busy
/// state that stops the user from starting such a command while this runs; no
/// maintenance flag is kept for callers that skip the dialog.
///
/// The free-space estimate assumes the database and `backups` share a drive
/// and that the WAL is emptied by the checkpoint just before the size is read.
/// The drive of SQLite's temporary files (TEMP) is not checked, and if the
/// checkpoint cannot empty the WAL the reported size change may not reflect
/// the real change.
fn run_vacuum_impl(
    writer: &Mutex<Connection>,
    wait: Duration,
    db_file: &Path,
    disk: &dyn DiskSpace,
    backup_step: impl FnOnce(&Connection) -> anyhow::Result<PathBuf>,
) -> Result<VacuumDto, String> {
    run_vacuum_with_sizes(writer, wait, db_file, disk, backup_step, &file_size)
}

fn run_vacuum_with_sizes(
    writer: &Mutex<Connection>,
    wait: Duration,
    db_file: &Path,
    disk: &dyn DiskSpace,
    backup_step: impl FnOnce(&Connection) -> anyhow::Result<PathBuf>,
    size_of: &dyn Fn(&Path) -> Result<u64, String>,
) -> Result<VacuumDto, String> {
    let conn = lock_within(writer, wait)?;
    ensure_developer_mode(&conn)?;

    checkpoint(&conn);
    let size_before = size_of(db_file)?;
    let dir = db_file.parent().unwrap_or(Path::new("."));
    let free = disk.available_bytes(dir).map_err(|e| {
        format!("ディスクの空き容量を確認できなかったため、最適化を行いません。({e})")
    })?;
    vacuum_space_check(size_before, free).map_err(|e| describe_insufficient_space(&e))?;

    let backup_path = backup_step(&conn).map_err(|e| {
        log::error!("backup before VACUUM failed: {e:#}");
        format!("事前のバックアップに失敗したため、最適化を行いませんでした。({e:#})")
    })?;

    conn.execute_batch("VACUUM").map_err(|e| {
        log::error!("VACUUM failed: {e}");
        format!("最適化に失敗しました。事前のバックアップは残っています。({e})")
    })?;
    checkpoint(&conn);
    // The VACUUM is done; failing to read the size afterwards must not turn
    // that success into an error.
    let size_after = match size_of(db_file) {
        Ok(size) => Some(size),
        Err(e) => {
            log::warn!("VACUUM done, but the size afterwards is unknown: {e}");
            None
        }
    };
    log::info!("VACUUM done: {size_before} -> {size_after:?} bytes");
    Ok(VacuumDto {
        size_before,
        size_after,
        size_before_text: format_bytes(size_before),
        size_after_text: size_after
            .map_or_else(|| "サイズを取得できませんでした".to_string(), format_bytes),
        backup_file: backup_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
    })
}

fn db_file() -> Result<PathBuf, String> {
    Ok(crate::paths::app_data_dir()
        .map_err(|e| e.to_string())?
        .join("app.db"))
}

/// Read-only, on a pooled connection; a large database makes this slow, so it
/// runs on a blocking worker.
#[tauri::command]
pub async fn run_integrity_check(db: State<'_, Db>) -> Result<IntegrityDto, String> {
    let pool = db.read_pool.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| e.to_string())?;
        Ok(run_integrity_check_impl(&conn))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn get_db_stats(db: State<'_, Db>) -> Result<DbStatsDto, String> {
    let pool = db.read_pool.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let size = file_size(&db_file()?)?;
        let conn = pool.get().map_err(|e| e.to_string())?;
        get_db_stats_impl(&conn, size)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn run_vacuum(db: State<'_, Db>) -> Result<VacuumDto, String> {
    let writer = db.writer.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let file = db_file()?;
        let backups = backup::backups_dir(file.parent().unwrap_or(Path::new(".")));
        run_vacuum_impl(&writer, WRITER_WAIT, &file, &RealDiskSpace, |conn| {
            backup::create_backup(
                conn,
                &backups,
                gb_core::backup::BackupKind::Manual,
                backup::unix_now(),
            )
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::queries;
    use crate::db::test_support::init_temp_db;
    use gb_core::testing::fake_disk_space::FakeDiskSpace;

    const PLENTY: u64 = 1 << 40;

    fn enable_developer_mode(db: &Db) {
        queries::set_developer_mode(&db.writer.lock().unwrap(), true).unwrap();
    }

    /// Many rows, then all deleted: free pages for `VACUUM` to give back.
    fn bloat(db: &Db) {
        let conn = db.writer.lock().unwrap();
        conn.execute_batch("CREATE TABLE junk (id INTEGER PRIMARY KEY, pad TEXT)")
            .unwrap();
        for _ in 0..2000 {
            conn.execute("INSERT INTO junk (pad) VALUES (?1)", ["x".repeat(2000)])
                .unwrap();
        }
        conn.execute("DELETE FROM junk", []).unwrap();
        checkpoint(&conn);
    }

    fn freelist(db: &Db) -> u64 {
        pragma_u64(&db.writer.lock().unwrap(), "freelist_count").unwrap()
    }

    fn real_backup(dir: &Path) -> impl FnOnce(&Connection) -> anyhow::Result<PathBuf> + '_ {
        move |conn| {
            backup::create_backup(
                conn,
                &backup::backups_dir(dir),
                gb_core::backup::BackupKind::Manual,
                backup::unix_now(),
            )
        }
    }

    #[test]
    fn integrity_check_of_a_healthy_database_is_ok() {
        let (_d, db) = init_temp_db();
        let r = run_integrity_check_impl(&db.read_pool.get().unwrap());
        assert!(r.ok);
        assert!(r.messages.is_empty());
    }

    #[test]
    fn integrity_check_reports_a_damaged_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT); CREATE INDEX t_v ON t (v);",
            )
            .unwrap();
            for i in 0..3000 {
                conn.execute("INSERT INTO t (v) VALUES (?1)", [format!("value-{i:05}")])
                    .unwrap();
            }
        }
        // Overwrite a stretch of pages past the header with garbage.
        let mut bytes = std::fs::read(&path).unwrap();
        assert!(bytes.len() > 4096 * 6);
        for b in &mut bytes[4096 * 3..4096 * 5] {
            *b = 0xA5;
        }
        std::fs::write(&path, bytes).unwrap();

        let conn = Connection::open(&path).unwrap();
        let r = run_integrity_check_impl(&conn);
        assert!(!r.ok);
        assert!(!r.messages.is_empty());
    }

    #[test]
    fn stats_report_the_file_size_and_free_pages() {
        let (_d, db) = init_temp_db();
        bloat(&db);
        let conn = db.read_pool.get().unwrap();
        let s = get_db_stats_impl(&conn, 123_456).unwrap();
        assert_eq!(s.file_size, 123_456);
        assert!(s.page_count > 0 && s.freelist_count > 0);
        assert!(s.page_size >= 512);
        assert_eq!(s.file_size_text, "120.6 KB");
        assert_ne!(s.reclaimable_text, "0 B");
    }

    #[test]
    fn vacuum_is_refused_when_developer_mode_is_off() {
        let (d, db) = init_temp_db();
        bloat(&db);
        let before = freelist(&db);
        let r = run_vacuum_impl(
            &db.writer,
            Duration::from_secs(1),
            &d.path().join("app.db"),
            &FakeDiskSpace::with_free(PLENTY),
            |_| panic!("no backup may be taken when the mode is off"),
        );
        assert_eq!(
            r.unwrap_err(),
            super::super::developer_cmds::DEVELOPER_MODE_REQUIRED
        );
        assert_eq!(freelist(&db), before);
    }

    #[test]
    fn vacuum_backs_up_first_then_shrinks_the_database() {
        let (d, db) = init_temp_db();
        enable_developer_mode(&db);
        bloat(&db);
        assert!(freelist(&db) > 0);

        let r = run_vacuum_impl(
            &db.writer,
            Duration::from_secs(1),
            &d.path().join("app.db"),
            &FakeDiskSpace::with_free(PLENTY),
            real_backup(d.path()),
        )
        .unwrap();

        assert_eq!(freelist(&db), 0);
        assert!(r.size_after.unwrap() < r.size_before);
        let backups: Vec<_> = std::fs::read_dir(backup::backups_dir(d.path()))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(backups, vec![r.backup_file.clone()]);
        assert!(
            backup::backups_dir(d.path())
                .join(&r.backup_file)
                .metadata()
                .unwrap()
                .len()
                > 0
        );
    }

    #[test]
    fn a_size_that_cannot_be_read_afterwards_does_not_fail_a_finished_vacuum() {
        let (d, db) = init_temp_db();
        enable_developer_mode(&db);
        bloat(&db);
        let calls = std::cell::Cell::new(0);
        let size_of = |p: &Path| {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                file_size(p)
            } else {
                Err("gone".to_string())
            }
        };

        let r = run_vacuum_with_sizes(
            &db.writer,
            Duration::from_secs(1),
            &d.path().join("app.db"),
            &FakeDiskSpace::with_free(PLENTY),
            real_backup(d.path()),
            &size_of,
        )
        .unwrap();

        assert_eq!(freelist(&db), 0, "the VACUUM itself ran");
        assert_eq!(r.size_after, None);
        assert!(r.size_after_text.contains("取得できませんでした"));
    }

    #[test]
    fn vacuum_does_not_run_when_the_backup_fails() {
        let (d, db) = init_temp_db();
        enable_developer_mode(&db);
        bloat(&db);
        let before = freelist(&db);

        let r = run_vacuum_impl(
            &db.writer,
            Duration::from_secs(1),
            &d.path().join("app.db"),
            &FakeDiskSpace::with_free(PLENTY),
            |_| Err(anyhow::anyhow!("disk exploded")),
        );

        assert!(r.unwrap_err().contains("バックアップに失敗"));
        assert_eq!(freelist(&db), before, "the database must be untouched");
    }

    #[test]
    fn vacuum_is_refused_without_enough_space_and_takes_no_backup() {
        let (d, db) = init_temp_db();
        enable_developer_mode(&db);
        bloat(&db);
        let before = freelist(&db);

        let r = run_vacuum_impl(
            &db.writer,
            Duration::from_secs(1),
            &d.path().join("app.db"),
            &FakeDiskSpace::with_free(1024),
            |_| panic!("no backup may be taken when there is no room"),
        );

        assert!(r.unwrap_err().contains("空き容量"));
        assert_eq!(freelist(&db), before);
    }

    #[test]
    fn vacuum_is_refused_when_the_free_space_cannot_be_read() {
        let (d, db) = init_temp_db();
        enable_developer_mode(&db);
        let r = run_vacuum_impl(
            &db.writer,
            Duration::from_secs(1),
            &d.path().join("app.db"),
            &FakeDiskSpace::failing("no volume"),
            |_| panic!("no backup may be taken when the space is unknown"),
        );
        assert!(r.unwrap_err().contains("空き容量を確認できなかった"));
    }

    #[test]
    fn vacuum_gives_up_when_the_writer_stays_busy() {
        let (d, db) = init_temp_db();
        enable_developer_mode(&db);
        let _held = db.writer.lock().unwrap();
        let r = run_vacuum_impl(
            &db.writer,
            Duration::from_millis(60),
            &d.path().join("app.db"),
            &FakeDiskSpace::with_free(PLENTY),
            |_| panic!("not reached"),
        );
        assert!(r.unwrap_err().contains("使用中"));
    }
}
