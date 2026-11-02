//! Trims a child process's stderr down to the lines that explain a failure.

/// Lines kept from the end of ffmpeg's stderr. The version banner and build
/// configuration come first and the actual cause comes last, so a few trailing
/// lines are enough to diagnose a failure without flooding logs and UI text.
pub const STDERR_TAIL_LINES: usize = 5;

/// Returns at most `max_lines` trailing non-blank lines of `stderr`, joined
/// with `\n`, each trimmed of surrounding whitespace (including `\r`).
/// Blank lines are skipped so a trailing newline doesn't eat the budget.
pub fn tail_lines(stderr: &str, max_lines: usize) -> String {
    let mut kept: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .rev()
        .take(max_lines)
        .collect();
    kept.reverse();
    kept.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_the_last_lines() {
        assert_eq!(tail_lines("a\nb\nc\nd", 2), "c\nd");
    }

    #[test]
    fn returns_everything_when_shorter_than_the_limit() {
        assert_eq!(tail_lines("a\nb", 5), "a\nb");
    }

    #[test]
    fn empty_input_yields_empty_string() {
        assert_eq!(tail_lines("", 5), "");
    }

    #[test]
    fn zero_limit_yields_empty_string() {
        assert_eq!(tail_lines("a\nb", 0), "");
    }

    #[test]
    fn blank_lines_and_crlf_are_dropped() {
        assert_eq!(tail_lines("a\r\n\r\nb\r\n\r\n", 5), "a\nb");
        assert_eq!(tail_lines("a\n\nb\n\n", 1), "b");
    }

    #[test]
    fn trims_each_line() {
        assert_eq!(tail_lines("  a  \n\tb\t", 5), "a\nb");
    }

    #[test]
    fn a_long_ffmpeg_banner_is_cut_to_the_cause() {
        let mut text = String::new();
        for i in 0..100 {
            text.push_str(&format!("banner line {i}\n"));
        }
        text.push_str("Could find no file with path 'x' and index in the range 0-4\n");
        text.push_str("Error opening input files: No such file or directory\n");
        let out = tail_lines(&text, STDERR_TAIL_LINES);
        assert_eq!(out.lines().count(), STDERR_TAIL_LINES);
        assert!(out.ends_with("Error opening input files: No such file or directory"));
        assert!(!out.contains("banner line 0"));
    }
}
