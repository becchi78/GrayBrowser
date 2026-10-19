//! Reads the active log file (`logs/app_rCURRENT.log`) while the logger keeps
//! writing to it.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use gb_core::ports::log_source::{LogChunk, LogReadError, LogSource};

/// The name `flexi_logger` gives the file it is currently appending to.
pub const CURRENT_LOG_FILE_NAME: &str = "app_rCURRENT.log";

pub struct FileLogSource {
    path: PathBuf,
}

impl FileLogSource {
    pub fn new(logs_dir: &Path) -> Self {
        Self {
            path: logs_dir.join(CURRENT_LOG_FILE_NAME),
        }
    }
}

fn failed(e: std::io::Error) -> LogReadError {
    LogReadError::Failed {
        message: e.to_string(),
    }
}

impl LogSource for FileLogSource {
    fn read_from(&self, offset: Option<u64>, max_bytes: u64) -> Result<LogChunk, LogReadError> {
        // std opens files with FILE_SHARE_READ | WRITE | DELETE on Windows, so
        // the logger can keep appending and can rotate (rename) the file while
        // it is open here. The handle is short-lived anyway.
        let mut file = File::open(&self.path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                LogReadError::NotFound
            } else {
                failed(e)
            }
        })?;
        let len = file.metadata().map_err(failed)?.len();

        // `aligned` is false when reading must begin at an arbitrary byte
        // rather than at a known line start (a saved offset always is one).
        let tail = len.saturating_sub(max_bytes);
        let (start, aligned, reset) = match offset {
            None => (tail, tail == 0, false),
            // Shrunk: the logger rotated to a new file.
            Some(o) if o > len => (tail, tail == 0, true),
            // Too much accumulated to hand over in one piece.
            Some(o) if len - o > max_bytes => (tail, tail == 0, true),
            Some(o) => (o, true, false),
        };

        // When not aligned, also read the byte before `start` to tell whether
        // `start` happens to be a line start.
        let from = if aligned { start } else { start - 1 };
        file.seek(SeekFrom::Start(from)).map_err(failed)?;
        let mut buf = Vec::new();
        // Bounded by `len` so a file growing during the read cannot make the
        // chunk end in the middle of whatever was appended since.
        file.take(len - from)
            .read_to_end(&mut buf)
            .map_err(failed)?;

        let mut base = from;
        if !aligned {
            match buf.iter().position(|&b| b == b'\n') {
                Some(i) => {
                    base += i as u64 + 1;
                    buf.drain(..=i);
                }
                // A single line longer than the window: nothing usable, and
                // the fragment must not be returned as if it were a line.
                None => {
                    return Ok(LogChunk {
                        text: String::new(),
                        next_offset: from + buf.len() as u64,
                        reset,
                    });
                }
            }
        }

        // Anything after the last newline is a line still being written.
        let complete = match buf.iter().rposition(|&b| b == b'\n') {
            Some(i) => i + 1,
            None => 0,
        };
        buf.truncate(complete);
        Ok(LogChunk {
            next_offset: base + complete as u64,
            text: String::from_utf8_lossy(&buf).into_owned(),
            reset,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn setup() -> (tempfile::TempDir, FileLogSource, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let source = FileLogSource::new(dir.path());
        let path = dir.path().join(CURRENT_LOG_FILE_NAME);
        (dir, source, path)
    }

    fn append(path: &Path, text: &str) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    #[test]
    fn missing_file_is_not_found() {
        let (_dir, source, _path) = setup();
        assert_eq!(source.read_from(None, 100), Err(LogReadError::NotFound));
        assert_eq!(source.read_from(Some(5), 100), Err(LogReadError::NotFound));
    }

    #[test]
    fn first_read_returns_whole_small_file() {
        let (_dir, source, path) = setup();
        append(&path, "INFO [m] a\nINFO [m] b\n");
        let chunk = source.read_from(None, 1024).unwrap();
        assert_eq!(chunk.text, "INFO [m] a\nINFO [m] b\n");
        assert_eq!(chunk.next_offset, 22);
        assert!(!chunk.reset);
    }

    #[test]
    fn empty_file_yields_empty_chunk() {
        let (_dir, source, path) = setup();
        append(&path, "");
        let chunk = source.read_from(None, 1024).unwrap();
        assert_eq!((chunk.text.as_str(), chunk.next_offset), ("", 0));
    }

    #[test]
    fn first_read_of_large_file_starts_at_a_line_boundary() {
        let (_dir, source, path) = setup();
        append(&path, "INFO [m] one\nINFO [m] two\nINFO [m] three\n");
        // 20 bytes from the end falls in the middle of "two".
        let chunk = source.read_from(None, 20).unwrap();
        assert_eq!(chunk.text, "INFO [m] three\n");
        assert_eq!(chunk.next_offset, 41);
    }

    #[test]
    fn window_starting_exactly_on_a_line_start_keeps_that_line() {
        let (_dir, source, path) = setup();
        append(&path, "INFO [m] one\nINFO [m] two\n");
        // "INFO [m] two\n" is 13 bytes and begins right after a newline.
        let chunk = source.read_from(None, 13).unwrap();
        assert_eq!(chunk.text, "INFO [m] two\n");
    }

    #[test]
    fn window_inside_a_multibyte_character_does_not_corrupt_text() {
        let (_dir, source, path) = setup();
        append(&path, "INFO [m] 失敗しました\nINFO [m] 次\n");
        for max in 1..40 {
            let chunk = source.read_from(None, max).unwrap();
            assert!(!chunk.text.contains('\u{fffd}'), "max={max}");
        }
    }

    #[test]
    fn single_line_longer_than_the_window_is_skipped() {
        let (_dir, source, path) = setup();
        append(&path, "INFO [m] a very long line without a break");
        let chunk = source.read_from(None, 10).unwrap();
        assert_eq!(chunk.text, "");
        assert_eq!(chunk.next_offset, 41);
    }

    #[test]
    fn increments_follow_appended_lines() {
        let (_dir, source, path) = setup();
        append(&path, "INFO [m] a\n");
        let first = source.read_from(None, 1024).unwrap();
        append(&path, "WARN [m] b\n");
        let second = source.read_from(Some(first.next_offset), 1024).unwrap();
        assert_eq!(second.text, "WARN [m] b\n");
        assert!(!second.reset);
        let third = source.read_from(Some(second.next_offset), 1024).unwrap();
        assert_eq!(third.text, "");
        assert_eq!(third.next_offset, second.next_offset);
    }

    #[test]
    fn unfinished_trailing_line_waits_for_the_next_read() {
        let (_dir, source, path) = setup();
        append(&path, "INFO [m] a\nINFO [m] par");
        let first = source.read_from(None, 1024).unwrap();
        assert_eq!(first.text, "INFO [m] a\n");
        assert_eq!(first.next_offset, 11);

        let idle = source.read_from(Some(first.next_offset), 1024).unwrap();
        assert_eq!((idle.text.as_str(), idle.next_offset), ("", 11));

        append(&path, "tial\n");
        let second = source.read_from(Some(first.next_offset), 1024).unwrap();
        assert_eq!(second.text, "INFO [m] partial\n");
    }

    #[test]
    fn shrunk_file_is_read_again_from_the_start_with_reset() {
        let (_dir, source, path) = setup();
        append(&path, "INFO [m] old line one\nINFO [m] old line two\n");
        let first = source.read_from(None, 1024).unwrap();
        std::fs::write(&path, "INFO [m] new\n").unwrap();
        let second = source.read_from(Some(first.next_offset), 1024).unwrap();
        assert!(second.reset);
        assert_eq!(second.text, "INFO [m] new\n");
        assert_eq!(second.next_offset, 13);
    }

    #[test]
    fn rotation_to_an_empty_file_resets_with_no_text() {
        let (_dir, source, path) = setup();
        append(&path, "INFO [m] a\n");
        let first = source.read_from(None, 1024).unwrap();
        std::fs::write(&path, "").unwrap();
        let second = source.read_from(Some(first.next_offset), 1024).unwrap();
        assert!(second.reset);
        assert_eq!((second.text.as_str(), second.next_offset), ("", 0));
    }

    #[test]
    fn too_large_a_gap_returns_only_the_tail_with_reset() {
        let (_dir, source, path) = setup();
        append(&path, "INFO [m] a\n");
        let first = source.read_from(None, 1024).unwrap();
        append(&path, "INFO [m] b\nINFO [m] c\nINFO [m] d\n");
        let second = source.read_from(Some(first.next_offset), 12).unwrap();
        assert!(second.reset);
        assert_eq!(second.text, "INFO [m] d\n");
    }

    #[test]
    fn reads_while_the_logger_still_holds_the_file_open() {
        let (_dir, source, path) = setup();
        let mut writer = File::create(&path).unwrap();
        writer.write_all(b"INFO [m] live\n").unwrap();
        writer.flush().unwrap();
        let chunk = source.read_from(None, 1024).unwrap();
        assert_eq!(chunk.text, "INFO [m] live\n");
        writer.write_all(b"INFO [m] more\n").unwrap();
        writer.flush().unwrap();
        let next = source.read_from(Some(chunk.next_offset), 1024).unwrap();
        assert_eq!(next.text, "INFO [m] more\n");
    }
}
