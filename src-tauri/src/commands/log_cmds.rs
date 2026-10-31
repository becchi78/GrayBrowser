//! Commands behind the in-app log viewer.

use std::path::PathBuf;

use gb_core::log_view::parse_log_text;
use gb_core::ports::log_source::{LogReadError, LogSource};
use serde::Serialize;

use crate::adapters::log_source::FileLogSource;

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

#[cfg(test)]
mod tests {
    use super::*;
    use gb_core::ports::log_source::LogChunk;
    use gb_core::testing::fake_log_source::FakeLogSource;

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
