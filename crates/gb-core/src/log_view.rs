//! Parsing and filtering of the application log for on-screen display.
//!
//! The log format is flexi_logger's default, which has no timestamp:
//! `LEVEL [target] message`. A message that spans several lines is written
//! as one header line followed by raw continuation lines.

use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    pub const ALL: [LogLevel; 5] = [
        LogLevel::Error,
        LogLevel::Warn,
        LogLevel::Info,
        LogLevel::Debug,
        LogLevel::Trace,
    ];

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFilter {
    pub levels: BTreeSet<LogLevel>,
    pub query: String,
}

impl LogFilter {
    pub fn all() -> Self {
        LogFilter {
            levels: LogLevel::ALL.into_iter().collect(),
            query: String::new(),
        }
    }
}

/// Entries of unknown level always pass the level check: they are fragments
/// whose level cannot be known, and hiding them would silently drop text.
pub fn entry_matches(entry: &LogEntry, filter: &LogFilter) -> bool {
    if entry.level != LogLevel::Unknown && !filter.levels.contains(&entry.level) {
        return false;
    }
    let query = filter.query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    entry.message.to_lowercase().contains(&query) || entry.target.to_lowercase().contains(&query)
}

/// Keeps only the last `max` items.
pub fn trim_to_last<T>(mut items: Vec<T>, max: usize) -> Vec<T> {
    if items.len() > max {
        items.drain(..items.len() - max);
    }
    items
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
        assert_eq!(levels, LogLevel::ALL.to_vec());
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

    fn only(levels: &[LogLevel], query: &str) -> LogFilter {
        LogFilter {
            levels: levels.iter().copied().collect(),
            query: query.to_string(),
        }
    }

    #[test]
    fn level_filter_includes_only_checked_levels() {
        let e = entry(LogLevel::Warn, "m", "x");
        assert!(entry_matches(&e, &LogFilter::all()));
        assert!(entry_matches(&e, &only(&[LogLevel::Warn], "")));
        assert!(!entry_matches(&e, &only(&[LogLevel::Error], "")));
        assert!(!entry_matches(&e, &only(&[], "")));
    }

    #[test]
    fn unknown_level_entries_pass_the_level_filter() {
        let e = entry(LogLevel::Unknown, "", "fragment");
        assert!(entry_matches(&e, &only(&[], "")));
        assert!(!entry_matches(&e, &only(&[], "other")));
    }

    #[test]
    fn query_matches_message_or_target_case_insensitively() {
        let e = entry(LogLevel::Info, "Graybrowser::Scan", "Found 3 Files");
        assert!(entry_matches(&e, &only(&LogLevel::ALL, "found 3")));
        assert!(entry_matches(&e, &only(&LogLevel::ALL, "SCAN")));
        assert!(!entry_matches(&e, &only(&LogLevel::ALL, "thumbnail")));
    }

    #[test]
    fn query_is_trimmed_and_blank_query_matches_all() {
        let e = entry(LogLevel::Info, "m", "abc");
        assert!(entry_matches(&e, &only(&LogLevel::ALL, "   ")));
        assert!(entry_matches(&e, &only(&LogLevel::ALL, " abc ")));
    }

    #[test]
    fn query_works_with_japanese() {
        let e = entry(LogLevel::Error, "m", "サムネイル生成に失敗");
        assert!(entry_matches(&e, &only(&LogLevel::ALL, "失敗")));
        assert!(!entry_matches(&e, &only(&LogLevel::ALL, "成功")));
    }

    #[test]
    fn trim_to_last_keeps_the_newest() {
        assert_eq!(trim_to_last(vec![1, 2, 3, 4, 5], 3), vec![3, 4, 5]);
        assert_eq!(trim_to_last(vec![1, 2, 3], 3), vec![1, 2, 3]);
        assert_eq!(trim_to_last(vec![1, 2], 3), vec![1, 2]);
        assert_eq!(trim_to_last(vec![1, 2], 0), Vec::<i32>::new());
        assert_eq!(trim_to_last(Vec::<i32>::new(), 3), Vec::<i32>::new());
    }
}
