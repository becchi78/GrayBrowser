//! Real [`WbImportLog`]: one text file per import under `logs/`, next to the
//! application log but outside its rotation, plus pruning of old import logs.

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use gb_core::ports::wb_import_log::WbImportLog;
use gb_core::wb_import_log::{
    latest_log_name, log_file_name, logs_to_delete, DateTimeParts, KEEP_LOG_COUNT,
};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;

/// Attempts at finding an unused name when several imports start within the
/// same second; far more than can happen in practice.
const MAX_SEQUENCE: u32 = 100;

/// Current local date and time. Falls back to UTC if the system reports
/// something unusable, so that a clock problem can never stop an import.
pub fn local_now() -> DateTimeParts {
    // SAFETY: `GetLocalTime` only writes into the SYSTEMTIME it is given and
    // has no failure mode. An all-zero SYSTEMTIME is a valid value.
    let st = unsafe {
        let mut st = std::mem::zeroed();
        GetLocalTime(&mut st);
        st
    };
    if st.wYear >= 1970 && (1..=12).contains(&st.wMonth) && (1..=31).contains(&st.wDay) {
        return DateTimeParts {
            year: u32::from(st.wYear),
            month: u32::from(st.wMonth),
            day: u32::from(st.wDay),
            hour: u32::from(st.wHour),
            minute: u32::from(st.wMinute),
            second: u32::from(st.wSecond),
        };
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    DateTimeParts::from_unix_utc(secs)
}

/// File names present in `logs_dir` (empty if it cannot be read).
fn list_file_names(logs_dir: &Path) -> Vec<String> {
    match fs::read_dir(logs_dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// The newest import log in `logs_dir`.
pub fn latest_log_path(logs_dir: &Path) -> Option<PathBuf> {
    latest_log_name(&list_file_names(logs_dir)).map(|name| logs_dir.join(name))
}

pub struct FileWbImportLog {
    path: PathBuf,
    /// `None` when the file could not be created or a write failed; later
    /// appends are then dropped.
    file: Mutex<Option<File>>,
}

impl FileWbImportLog {
    /// A log that discards everything, for when no log directory can be
    /// determined: the import still has to run.
    pub fn unavailable() -> Self {
        Self {
            path: PathBuf::from("(import log unavailable)"),
            file: Mutex::new(None),
        }
    }

    /// Creates this import's log file and deletes import logs beyond the
    /// newest [`KEEP_LOG_COUNT`]. Never fails: when the file cannot be
    /// created the result silently discards everything, because the log is a
    /// diagnostic aid and must not stop the import.
    pub fn create(logs_dir: &Path, now: &DateTimeParts) -> Self {
        if let Err(e) = fs::create_dir_all(logs_dir) {
            log::warn!("wb import log: cannot create {}: {e}", logs_dir.display());
        }
        let mut opened = None;
        for sequence in 0..MAX_SEQUENCE {
            let path = logs_dir.join(log_file_name(now, sequence));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    opened = Some((path, file));
                    break;
                }
                // Another import started in the same second: take the next name.
                Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    log::warn!("wb import log: cannot create {}: {e}", path.display());
                    break;
                }
            }
        }
        let log = match opened {
            Some((path, file)) => Self {
                path,
                file: Mutex::new(Some(file)),
            },
            None => Self {
                path: logs_dir.join(log_file_name(now, 0)),
                file: Mutex::new(None),
            },
        };
        prune(logs_dir);
        log
    }
}

fn prune(logs_dir: &Path) {
    for name in logs_to_delete(&list_file_names(logs_dir), KEEP_LOG_COUNT) {
        let path = logs_dir.join(&name);
        if let Err(e) = fs::remove_file(&path) {
            log::warn!(
                "wb import log: cannot delete old log {}: {e}",
                path.display()
            );
        }
    }
}

impl WbImportLog for FileWbImportLog {
    fn append(&self, text: &str) {
        let mut guard = self.file.lock().unwrap();
        let Some(file) = guard.as_mut() else {
            return;
        };
        if let Err(e) = file.write_all(text.as_bytes()).and_then(|()| file.flush()) {
            log::warn!(
                "wb import log: write to {} failed: {e}; the rest of this log is dropped",
                self.path.display()
            );
            *guard = None;
        }
    }

    fn location(&self) -> String {
        self.path.display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(second: u32) -> DateTimeParts {
        DateTimeParts {
            year: 2026,
            month: 10,
            day: 28,
            hour: 11,
            minute: 51,
            second,
        }
    }

    fn file_names(dir: &Path) -> Vec<String> {
        let mut names = list_file_names(dir);
        names.sort();
        names
    }

    #[test]
    fn creates_the_directory_and_writes_appended_text() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("logs");
        let log = FileWbImportLog::create(&dir, &at(0));
        log.append("one\n");
        log.append("二\n");
        let path = dir.join("wb-import-20261028-115100.log");
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\n二\n");
        assert_eq!(log.location(), path.display().to_string());
    }

    #[test]
    fn two_imports_in_one_second_do_not_overwrite_each_other() {
        let tmp = tempfile::tempdir().unwrap();
        let first = FileWbImportLog::create(tmp.path(), &at(0));
        let second = FileWbImportLog::create(tmp.path(), &at(0));
        first.append("first\n");
        second.append("second\n");
        assert_ne!(first.location(), second.location());
        assert_eq!(fs::read_to_string(first.location()).unwrap(), "first\n");
        assert_eq!(fs::read_to_string(second.location()).unwrap(), "second\n");
    }

    #[test]
    fn keeps_only_the_newest_logs_and_never_touches_other_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        for s in 0..12 {
            fs::write(dir.join(log_file_name(&at(s), 0)), "old").unwrap();
        }
        for other in ["app_rCURRENT.log", "app_r00001.log", "notes.txt"] {
            fs::write(dir.join(other), "keep").unwrap();
        }
        let log = FileWbImportLog::create(dir, &at(30));
        log.append("x");

        let names = file_names(dir);
        let imports = names.iter().filter(|n| n.starts_with("wb-import-")).count();
        assert_eq!(imports, KEEP_LOG_COUNT);
        // The new one and the 9 newest older ones survive; the oldest 3 go.
        assert!(names.contains(&"wb-import-20261028-115130.log".to_string()));
        assert!(!names.contains(&"wb-import-20261028-115100.log".to_string()));
        assert!(!names.contains(&"wb-import-20261028-115102.log".to_string()));
        assert!(names.contains(&"wb-import-20261028-115103.log".to_string()));
        for other in ["app_rCURRENT.log", "app_r00001.log", "notes.txt"] {
            assert!(names.contains(&other.to_string()), "{other} was deleted");
        }
    }

    #[test]
    fn an_uncreatable_log_swallows_writes_without_panicking() {
        let tmp = tempfile::tempdir().unwrap();
        // A file where the directory should be: neither mkdir nor create works.
        let blocker = tmp.path().join("logs");
        fs::write(&blocker, "not a directory").unwrap();
        let log = FileWbImportLog::create(&blocker, &at(0));
        log.append("lost\n");
        assert!(log.location().contains("wb-import-20261028-115100.log"));
    }

    #[test]
    fn latest_log_path_picks_the_newest_import_log_only() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        assert_eq!(latest_log_path(dir), None);
        fs::write(dir.join("app_rCURRENT.log"), "").unwrap();
        assert_eq!(latest_log_path(dir), None);
        fs::write(dir.join("wb-import-20261001-000000.log"), "").unwrap();
        fs::write(dir.join("wb-import-20261002-000000.log"), "").unwrap();
        assert_eq!(
            latest_log_path(dir),
            Some(dir.join("wb-import-20261002-000000.log"))
        );
        assert_eq!(latest_log_path(&dir.join("missing")), None);
    }

    #[test]
    fn local_now_is_plausible() {
        let now = local_now();
        assert!(now.year >= 2024);
        assert!((1..=12).contains(&now.month));
        assert!(now.hour < 24 && now.minute < 60 && now.second < 62);
    }
}
