//! Parsing of the application log for on-screen display.
//!
//! The log format is flexi_logger's default, which has no timestamp:
//! `LEVEL [target] message`. A message that spans several lines is written
//! as one header line followed by raw continuation lines.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
    /// A line that could not be attributed to any header, e.g. the first line
    /// of an incremental read that lands in the middle of a multi-line message.
    Unknown,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Error => "ERROR",
            LogLevel::Warn => "WARN",
            LogLevel::Info => "INFO",
            LogLevel::Debug => "DEBUG",
            LogLevel::Trace => "TRACE",
            LogLevel::Unknown => "UNKNOWN",
        }
    }

    fn from_token(token: &str) -> Option<LogLevel> {
        match token {
            "ERROR" => Some(LogLevel::Error),
            "WARN" => Some(LogLevel::Warn),
            "INFO" => Some(LogLevel::Info),
            "DEBUG" => Some(LogLevel::Debug),
            "TRACE" => Some(LogLevel::Trace),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub level: LogLevel,
    pub target: String,
    /// Continuation lines are joined with `\n`.
    pub message: String,
}

/// Splits a line into `(level, target, message)` when it is an entry header.
///
/// The `[target]` part is required: without it a continuation line that merely
/// starts with a word like "INFO" would be mistaken for a new entry.
fn parse_header(line: &str) -> Option<(LogLevel, &str, &str)> {
    let (token, rest) = line.split_once(' ')?;
    let level = LogLevel::from_token(token)?;
    let rest = rest.strip_prefix('[')?;
    let (target, message) = rest.split_once(']')?;
    let message = message.strip_prefix(' ').unwrap_or(message);
    Some((level, target, message))
}

/// Parses log text into entries. Lines that are not a header continue the
/// previous entry; blank lines carry no information and are dropped.
pub fn parse_log_text(text: &str) -> Vec<LogEntry> {
    let mut entries: Vec<LogEntry> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.trim().is_empty() {
            continue;
        }
        if let Some((level, target, message)) = parse_header(line) {
            entries.push(LogEntry {
                level,
                target: target.to_string(),
                message: message.to_string(),
            });
        } else if let Some(last) = entries.last_mut() {
            last.message.push('\n');
            last.message.push_str(line);
        } else {
            entries.push(LogEntry {
                level: LogLevel::Unknown,
                target: String::new(),
                message: line.to_string(),
            });
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(level: LogLevel, target: &str, message: &str) -> LogEntry {
        LogEntry {
            level,
            target: target.to_string(),
            message: message.to_string(),
        }
    }

    #[test]
    fn parses_every_level() {
        let text = "ERROR [a] e\nWARN [b] w\nINFO [c] i\nDEBUG [d] d\nTRACE [e] t\n";
        let entries = parse_log_text(text);
        let levels: Vec<_> = entries.iter().map(|e| e.level).collect();
        assert_eq!(
            levels,
            vec![
                LogLevel::Error,
                LogLevel::Warn,
                LogLevel::Info,
                LogLevel::Debug,
                LogLevel::Trace,
            ]
        );
        assert_eq!(entries[2], entry(LogLevel::Info, "c", "i"));
    }

    #[test]
    fn continuation_lines_join_the_previous_entry() {
        let text = "ERROR [m] failed:\n  caused by: x\n  caused by: y\nINFO [m] next\n";
        let entries = parse_log_text(text);
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].message,
            "failed:\n  caused by: x\n  caused by: y"
        );
        assert_eq!(entries[1].message, "next");
    }

    #[test]
    fn a_continuation_that_starts_with_a_level_word_is_not_a_header() {
        let entries = parse_log_text("INFO [m] a\nINFO without target\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, "a\nINFO without target");
    }

    #[test]
    fn blank_lines_and_empty_text_produce_nothing() {
        assert!(parse_log_text("").is_empty());
        assert!(parse_log_text("\n\n  \n").is_empty());
        let entries = parse_log_text("INFO [m] a\n\n\nINFO [m] b\n");
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn headerless_text_at_the_start_becomes_unknown_entry() {
        let entries = parse_log_text("tail of a message\nmore\nINFO [m] ok\n");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].level, LogLevel::Unknown);
        assert_eq!(entries[0].message, "tail of a message\nmore");
        assert_eq!(entries[1].level, LogLevel::Info);
    }

    #[test]
    fn handles_crlf_and_missing_trailing_newline() {
        let entries = parse_log_text("INFO [m] a\r\nWARN [m] b");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "a");
        assert_eq!(entries[1].message, "b");
    }

    #[test]
    fn broken_headers_are_treated_as_text() {
        // lowercase level, missing closing bracket, level only
        let entries = parse_log_text("INFO [m] a\ninfo [m] b\nINFO [broken\nINFO\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, "a\ninfo [m] b\nINFO [broken\nINFO");
    }

    #[test]
    fn keeps_japanese_and_brackets_inside_the_message() {
        let entries = parse_log_text("WARN [graybrowser_lib::scan] 読み取り失敗 [x] ]\n");
        assert_eq!(
            entries[0],
            entry(
                LogLevel::Warn,
                "graybrowser_lib::scan",
                "読み取り失敗 [x] ]"
            )
        );
    }

    #[test]
    fn empty_message_and_empty_target_are_allowed() {
        let entries = parse_log_text("INFO [m]\nINFO [] x\n");
        assert_eq!(entries[0], entry(LogLevel::Info, "m", ""));
        assert_eq!(entries[1], entry(LogLevel::Info, "", "x"));
    }
}
