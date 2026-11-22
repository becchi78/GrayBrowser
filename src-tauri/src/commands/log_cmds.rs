//! Commands behind the in-app log viewer, and the developer-mode controls for
//! the log level and the log folder.

use std::path::{Path, PathBuf};

use gb_core::log_level::{effective_log_level, LogLevel};
use gb_core::log_view::parse_log_text;
use gb_core::ports::log_source::{LogReadError, LogSource};
use rusqlite::Connection;
use serde::Serialize;
use tauri::State;
use tauri_plugin_opener::OpenerExt;

use super::developer_cmds::ensure_developer_mode;
use crate::adapters::log_source::FileLogSource;
use crate::db::{queries, Db};
use crate::logging::LogLevelControl;

/// How much of the log's tail the first read returns. Larger logs (up to
/// 10MB) are never loaded whole into the UI.
const MAX_READ_BYTES: u64 = 512 * 1024;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct LogEntryDto {
    /// `ERROR` / `WARN` / `INFO` / `DEBUG` / `TRACE` / `UNKNOWN`.
    pub level: String,
    pub target: String,
    pub message: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct LogChunkDto {
    pub entries: Vec<LogEntryDto>,
    pub next_offset: u64,
    /// The caller must drop the entries it already holds before appending.
    pub reset: bool,
}

fn read_log_impl(
    source: &dyn LogSource,
    offset: Option<u64>,
    max_bytes: u64,
) -> Result<LogChunkDto, String> {
    let chunk = source.read_from(offset, max_bytes).map_err(|e| match e {
        LogReadError::NotFound => "ログファイルがまだありません。".to_string(),
        LogReadError::Failed { message } => format!("ログを読み取れませんでした: {message}"),
    })?;
    let entries = parse_log_text(&chunk.text)
        .into_iter()
        .map(|e| LogEntryDto {
            level: e.level.as_str().to_string(),
            target: e.target,
            message: e.message,
        })
        .collect();
    Ok(LogChunkDto {
        entries,
        next_offset: chunk.next_offset,
        reset: chunk.reset,
    })
}

pub(crate) fn logs_dir() -> Result<PathBuf, String> {
    crate::paths::app_data_dir()
        .map(|dir| dir.join("logs"))
        .map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn read_log(offset: Option<u64>) -> Result<LogChunkDto, String> {
    let source = FileLogSource::new(&logs_dir()?);
    read_log_impl(&source, offset, MAX_READ_BYTES)
}

#[tauri::command]
pub fn get_log_dir() -> Result<String, String> {
    logs_dir().map(|p| p.display().to_string())
}

/// Applies the level in force -- the configured one in developer mode,
/// `info` otherwise. Called at startup and whenever either input changes.
pub(crate) fn apply_effective_log_level(
    conn: &Connection,
    apply: impl FnOnce(LogLevel),
) -> anyhow::Result<()> {
    let developer_mode = queries::get_developer_mode(conn)?;
    let configured = queries::get_log_level(conn)?;
    apply(effective_log_level(developer_mode, configured));
    Ok(())
}

/// The value is checked before anything is stored, and developer mode is
/// checked here rather than trusted to the menu, because a verbose level
/// writes far more (including file paths) to the log.
fn set_log_level_impl(
    conn: &Connection,
    level: &str,
    apply: impl FnOnce(LogLevel),
) -> Result<(), String> {
    let level =
        LogLevel::parse(level).ok_or_else(|| format!("未対応のログレベルです: \"{level}\""))?;
    ensure_developer_mode(conn)?;
    queries::set_log_level(conn, level).map_err(|e| e.to_string())?;
    apply_effective_log_level(conn, apply).map_err(|e| e.to_string())
}

/// Opens only the fixed log folder: the frontend never supplies a path.
fn open_log_dir_impl(
    conn: &Connection,
    logs_dir: &Path,
    open: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    ensure_developer_mode(conn)?;
    // Logging may have failed to start, so the folder might not exist yet.
    std::fs::create_dir_all(logs_dir).map_err(|e| e.to_string())?;
    open(logs_dir)
}

/// The stored choice, which is not necessarily the level in force (outside
/// developer mode the level is always `info`).
#[tauri::command]
pub fn get_log_level(db: State<Db>) -> Result<String, String> {
    let conn = db.read_pool.get().map_err(|e| e.to_string())?;
    queries::get_log_level(&conn)
        .map(|l| l.as_str().to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_log_level(
    db: State<Db>,
    control: State<LogLevelControl>,
    level: String,
) -> Result<(), String> {
    let conn = db.writer.lock().unwrap();
    set_log_level_impl(&conn, &level, |l| control.apply(l))
}

/// `async` so it runs on a worker thread rather than the UI thread; the shell
/// call needs COM set up on that thread first.
#[tauri::command(async)]
pub fn open_log_dir(app: tauri::AppHandle, db: State<Db>) -> Result<(), String> {
    let conn = db.read_pool.get().map_err(|e| e.to_string())?;
    open_log_dir_impl(&conn, &logs_dir()?, |dir| {
        let dir_str = dir
            .to_str()
            .ok_or_else(|| format!("path is not valid UTF-8: {}", dir.display()))?;
        crate::adapters::player::init_com_for_shell();
        app.opener()
            .open_path(dir_str, None::<&str>)
            .map_err(|e| e.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gb_core::ports::log_source::LogChunk;
    use gb_core::testing::fake_log_source::FakeLogSource;

    mod level_and_folder {
        use super::*;
        use crate::db::test_support::init_temp_db;
        use std::cell::{Cell, RefCell};

        fn set_dev(db: &Db, on: bool) {
            queries::set_developer_mode(&db.writer.lock().unwrap(), on).unwrap();
        }

        #[test]
        fn setting_a_level_is_refused_when_developer_mode_is_off() {
            let (_d, db) = init_temp_db();
            let conn = db.writer.lock().unwrap();
            let applied = Cell::new(None);
            let err = set_log_level_impl(&conn, "debug", |l| applied.set(Some(l))).unwrap_err();
            assert!(err.contains("開発者モード"));
            assert_eq!(applied.get(), None);
            assert_eq!(queries::get_log_level(&conn).unwrap(), LogLevel::Info);
        }

        #[test]
        fn an_invalid_level_is_rejected_without_storing_or_applying() {
            let (_d, db) = init_temp_db();
            set_dev(&db, true);
            let conn = db.writer.lock().unwrap();
            let applied = Cell::new(None);
            assert!(set_log_level_impl(&conn, "verbose", |l| applied.set(Some(l))).is_err());
            assert_eq!(applied.get(), None);
            assert_eq!(queries::get_log_level(&conn).unwrap(), LogLevel::Info);
        }

        #[test]
        fn a_valid_level_in_developer_mode_is_stored_and_applied() {
            let (_d, db) = init_temp_db();
            set_dev(&db, true);
            let conn = db.writer.lock().unwrap();
            let applied = Cell::new(None);
            set_log_level_impl(&conn, "trace", |l| applied.set(Some(l))).unwrap();
            assert_eq!(applied.get(), Some(LogLevel::Trace));
            assert_eq!(queries::get_log_level(&conn).unwrap(), LogLevel::Trace);
        }

        #[test]
        fn the_effective_level_follows_developer_mode() {
            let (_d, db) = init_temp_db();
            set_dev(&db, true);
            queries::set_log_level(&db.writer.lock().unwrap(), LogLevel::Debug).unwrap();
            let at = |db: &Db| {
                let got = Cell::new(None);
                apply_effective_log_level(&db.writer.lock().unwrap(), |l| got.set(Some(l)))
                    .unwrap();
                got.get().unwrap()
            };
            assert_eq!(at(&db), LogLevel::Debug);
            // The stored choice survives, but the level in force drops to info.
            set_dev(&db, false);
            assert_eq!(at(&db), LogLevel::Info);
            assert_eq!(
                queries::get_log_level(&db.writer.lock().unwrap()).unwrap(),
                LogLevel::Debug
            );
        }

        #[test]
        fn opening_the_log_folder_is_refused_when_developer_mode_is_off() {
            let (dir, db) = init_temp_db();
            let conn = db.writer.lock().unwrap();
            let opened = Cell::new(false);
            let err = open_log_dir_impl(&conn, &dir.path().join("logs"), |_| {
                opened.set(true);
                Ok(())
            })
            .unwrap_err();
            assert!(err.contains("開発者モード"));
            assert!(!opened.get());
        }

        #[test]
        fn opening_the_log_folder_opens_exactly_that_folder() {
            let (dir, db) = init_temp_db();
            set_dev(&db, true);
            let conn = db.writer.lock().unwrap();
            let logs = dir.path().join("logs");
            let seen = RefCell::new(None);
            open_log_dir_impl(&conn, &logs, |p| {
                *seen.borrow_mut() = Some(p.to_path_buf());
                Ok(())
            })
            .unwrap();
            assert_eq!(seen.into_inner(), Some(logs.clone()));
            assert!(logs.is_dir());
        }
    }

    #[test]
    fn parses_the_chunk_and_passes_arguments_through() {
        let fake = FakeLogSource::with_script(vec![Ok(LogChunk {
            text: "ERROR [m] boom\n  detail\nINFO [m] ok\n".into(),
            next_offset: 36,
            reset: true,
        })]);
        let dto = read_log_impl(&fake, Some(7), 99).unwrap();
        assert_eq!(dto.next_offset, 36);
        assert!(dto.reset);
        assert_eq!(dto.entries.len(), 2);
        assert_eq!(dto.entries[0].level, "ERROR");
        assert_eq!(dto.entries[0].message, "boom\n  detail");
        assert_eq!(dto.entries[1].level, "INFO");
        assert_eq!(*fake.calls.lock().unwrap(), vec![(Some(7), 99)]);
    }

    #[test]
    fn maps_read_errors_to_messages() {
        let fake = FakeLogSource::with_script(vec![
            Err(LogReadError::NotFound),
            Err(LogReadError::Failed {
                message: "denied".into(),
            }),
        ]);
        assert!(read_log_impl(&fake, None, 1)
            .unwrap_err()
            .contains("まだありません"));
        assert!(read_log_impl(&fake, None, 1)
            .unwrap_err()
            .contains("denied"));
    }
}
