//! Executes database backups: `VACUUM INTO` a temp file, verify it, publish it
//! atomically, then prune old generations. Naming and retention decisions are
//! the pure functions in `gb_core::backup`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use gb_core::backup::{backup_file_name, select_for_deletion, BackupKind};

/// Backups taken automatically right before a schema migration.
pub const KEEP_PREMIGRATE: usize = 3;
/// Backups taken on user request. Counted separately from `KEEP_PREMIGRATE`
/// so repeated manual backups cannot evict the pre-migration safety copies.
pub const KEEP_MANUAL: usize = 10;

const BACKUPS_DIR_NAME: &str = "backups";
const TEMP_SUFFIX: &str = ".tmp";

pub fn backups_dir(app_dir: &Path) -> PathBuf {
    app_dir.join(BACKUPS_DIR_NAME)
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Writes a consistent snapshot of `conn`'s database into `dir` and prunes old
/// generations. Returns the path of the new backup.
///
/// The snapshot is built under a `.tmp` name and only renamed into place once
/// `PRAGMA quick_check` passes on it, so a failed or corrupt backup is never
/// visible as a generation and never counts against (or evicts) a good one.
/// Pruning runs only after the new backup has been published.
pub fn create_backup(
    conn: &rusqlite::Connection,
    dir: &Path,
    kind: BackupKind,
    now_unix_secs: i64,
) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("failed to create backup directory {}", dir.display()))?;

    let final_name = backup_file_name(kind, now_unix_secs);
    let final_path = dir.join(&final_name);
    if final_path.exists() {
        bail!("backup {} already exists", final_path.display());
    }
    let temp_path = dir.join(format!("{final_name}{TEMP_SUFFIX}"));
    // A leftover from a crashed earlier attempt would make VACUUM INTO fail.
    let _ = std::fs::remove_file(&temp_path);

    let result = write_and_verify(conn, &temp_path)
        .and_then(|()| std::fs::rename(&temp_path, &final_path).map_err(Into::into));
    if let Err(e) = result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e.context(format!("failed to create backup {final_name}")));
    }

    if let Err(e) = prune(dir) {
        log::warn!("backup pruning failed: {e}");
    }
    Ok(final_path)
}

fn write_and_verify(conn: &rusqlite::Connection, temp_path: &Path) -> anyhow::Result<()> {
    let dest = temp_path
        .to_str()
        .context("backup path is not valid UTF-8")?;
    conn.execute("VACUUM INTO ?1", [dest])
        .context("VACUUM INTO failed")?;

    let backup = rusqlite::Connection::open_with_flags(
        temp_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let verdict: String = backup.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if verdict != "ok" {
        bail!("integrity check of the written backup failed: {verdict}");
    }
    Ok(())
}

fn prune(dir: &Path) -> anyhow::Result<()> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        if let Some(name) = entry?.file_name().to_str() {
            names.push(name.to_string());
        }
    }
    for name in select_for_deletion(&names, KEEP_PREMIGRATE, KEEP_MANUAL) {
        std::fs::remove_file(dir.join(&name))
            .with_context(|| format!("failed to delete old backup {name}"))?;
    }
    Ok(())
}

/// Highest applied schema version, or 0 for a database without any.
pub fn current_schema_version(conn: &rusqlite::Connection) -> rusqlite::Result<i64> {
    let has_table: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_version')",
        [],
        |row| row.get(0),
    )?;
    if !has_table {
        return Ok(0);
    }
    conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get(0),
    )
}
