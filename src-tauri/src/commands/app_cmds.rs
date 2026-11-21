//! Commands behind the in-app menu that must run in Rust: quitting and the
//! manual database backup. The `#[tauri::command]` wrappers
//! are thin; the decisions live in plain functions so they can be tested
//! without a Tauri runtime.

use std::path::{Path, PathBuf};

use tauri::{Manager, WebviewWindow};

use crate::db::{self, Db};
use crate::paths;

const BACKUP_DIALOG_TITLE: &str = "データベースをバックアップ";

/// Whether a backup result is shown as information or as an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReportKind {
    Info,
    Error,
}

/// Creates a manual backup from a read connection: `VACUUM INTO` only needs
/// to read, so the single writer is never held up by a backup.
fn perform_manual_backup(db: &Db, app_dir: &Path, now_unix_secs: i64) -> anyhow::Result<PathBuf> {
    let conn = db.read_pool.get()?;
    db::backup::create_backup(
        &conn,
        &db::backup::backups_dir(app_dir),
        gb_core::backup::BackupKind::Manual,
        now_unix_secs,
    )
}

/// The message the user sees once a backup has finished or failed.
fn backup_report(outcome: &anyhow::Result<PathBuf>) -> (String, ReportKind) {
    match outcome {
        Ok(path) => (
            format!("バックアップを作成しました。\n{}", path.display()),
            ReportKind::Info,
        ),
        Err(e) => (
            format!("バックアップに失敗しました。\n{e:#}"),
            ReportKind::Error,
        ),
    }
}

/// Starts a manual backup and returns immediately. The backup runs on its own
/// thread so the UI is not blocked by a large database, and the result is
/// reported through a native message dialog (there is no frontend modal for
/// it).
#[tauri::command]
pub fn run_manual_backup(app_handle: tauri::AppHandle) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

    std::thread::spawn(move || {
        let outcome = paths::app_data_dir().and_then(|dir| {
            perform_manual_backup(&app_handle.state::<Db>(), &dir, db::backup::unix_now())
        });
        if let Err(e) = &outcome {
            log::error!("manual database backup failed: {e:#}");
        }
        let (message, kind) = backup_report(&outcome);
        app_handle
            .dialog()
            .message(message)
            .title(BACKUP_DIALOG_TITLE)
            .kind(match kind {
                ReportKind::Info => MessageDialogKind::Info,
                ReportKind::Error => MessageDialogKind::Error,
            })
            .show(|_| {});
    });
}

/// Quits by closing the main window rather than calling `exit()`: `close()`
/// raises `CloseRequested` just like the title bar's close button, so the
/// window-size save in `main_window` runs on this path too. Closing the last
/// window ends the app.
#[tauri::command]
pub fn quit_app(window: WebviewWindow) -> Result<(), String> {
    window.close().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::init_temp_db;

    #[test]
    fn a_manual_backup_is_written_under_the_backups_dir() {
        let (dir, db) = init_temp_db();
        let path = perform_manual_backup(&db, dir.path(), 1_700_000_000).unwrap();
        assert!(path.exists());
        assert_eq!(path.parent().unwrap(), db::backup::backups_dir(dir.path()));
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("manual"));
    }

    #[test]
    fn a_second_backup_in_the_same_second_fails_without_hiding_the_first() {
        let (dir, db) = init_temp_db();
        let first = perform_manual_backup(&db, dir.path(), 1_700_000_000).unwrap();
        assert!(perform_manual_backup(&db, dir.path(), 1_700_000_000).is_err());
        assert!(first.exists());
    }

    #[test]
    fn a_success_is_reported_as_info_with_the_path() {
        let (msg, kind) = backup_report(&Ok(PathBuf::from("C:/x/backup.db")));
        assert_eq!(kind, ReportKind::Info);
        assert!(msg.starts_with("バックアップを作成しました。"));
        assert!(msg.contains("backup.db"));
    }

    #[test]
    fn a_failure_is_reported_as_an_error_with_the_cause() {
        let (msg, kind) = backup_report(&Err(anyhow::anyhow!("disk full")));
        assert_eq!(kind, ReportKind::Error);
        assert!(msg.starts_with("バックアップに失敗しました。"));
        assert!(msg.contains("disk full"));
    }
}
