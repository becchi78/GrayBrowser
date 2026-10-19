//! Validation of a user-typed new file name for renaming an unregistered
//! file in place. Pure logic: no filesystem access, no `#[cfg(windows)]`.
//!
//! The rules are Windows' file-name rules, applied on every platform so the
//! result does not depend on where the tests run. Whether a same-named file
//! already exists is *not* decided here (that needs the filesystem and must
//! be atomic with the rename itself); see `ports::file_rename`.

use crate::filename_validation::is_machine_dependent_filename;
use crate::scan_pipeline::is_video_file;

/// Windows' limit for one path component, in UTF-16 code units.
const MAX_FILE_NAME_UTF16_UNITS: usize = 255;

const INVALID_CHARS: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum RenameRejection {
    #[error("新しい名前を入力してください。")]
    Empty,
    #[error("元の名前と同じです。")]
    Unchanged,
    #[error("ファイル名に使えない文字が含まれています: {0:?}")]
    InvalidChar(char),
    #[error("ファイル名の末尾に「.」や空白は使えません。")]
    TrailingDotOrSpace,
    #[error("「{0}」はWindowsの予約名のため使えません。")]
    ReservedName(String),
    #[error("ファイル名が長すぎます（255文字まで）。")]
    TooLong,
    #[error("機種依存文字が残っています: {0}")]
    MachineDependentChar(char),
    #[error("動画ファイルの拡張子ではなくなるため変更できません。")]
    NotVideoExtension,
}

/// Returns the trimmed new name when `input` is acceptable as a replacement
/// for `old_name`.
///
/// Rejecting every path separator (and `:`) is what keeps the rename inside
/// the original folder; callers join the result onto the original parent
/// without further checks.
pub fn validate_new_file_name(old_name: &str, input: &str) -> Result<String, RenameRejection> {
    let name = input.trim();
    if name.is_empty() {
        return Err(RenameRejection::Empty);
    }
    if name == old_name {
        return Err(RenameRejection::Unchanged);
    }
    if let Some(c) = name
        .chars()
        .find(|c| c.is_control() || INVALID_CHARS.contains(c))
    {
        return Err(RenameRejection::InvalidChar(c));
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(RenameRejection::TrailingDotOrSpace);
    }
    // Windows reserves the device name even when an extension follows
    // ("con.mp4"), and ignores spaces before the extension dot.
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    if RESERVED_NAMES.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        return Err(RenameRejection::ReservedName(stem.to_string()));
    }
    if name.encode_utf16().count() > MAX_FILE_NAME_UTF16_UNITS {
        return Err(RenameRejection::TooLong);
    }
    if let Some(c) = is_machine_dependent_filename(name) {
        return Err(RenameRejection::MachineDependentChar(c));
    }
    if !is_video_file(name) {
        return Err(RenameRejection::NotVideoExtension);
    }
    Ok(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "movie①.mp4";

    fn check(input: &str) -> Result<String, RenameRejection> {
        validate_new_file_name(OLD, input)
    }

    #[test]
    fn accepts_a_clean_video_name_and_returns_it_trimmed() {
        assert_eq!(check("  movie1.mp4  "), Ok("movie1.mp4".to_string()));
    }

    #[test]
    fn rejects_empty_and_whitespace_only() {
        assert_eq!(check(""), Err(RenameRejection::Empty));
        assert_eq!(check("   \u{3000} "), Err(RenameRejection::Empty));
    }

    #[test]
    fn rejects_the_unchanged_name_even_with_surrounding_spaces() {
        assert_eq!(check(OLD), Err(RenameRejection::Unchanged));
        assert_eq!(check(" movie①.mp4 "), Err(RenameRejection::Unchanged));
    }

    #[test]
    fn rejects_each_invalid_char() {
        for c in ['\\', '/', ':', '*', '?', '"', '<', '>', '|'] {
            let input = format!("a{c}b.mp4");
            assert_eq!(check(&input), Err(RenameRejection::InvalidChar(c)), "{c}");
        }
    }

    #[test]
    fn rejects_control_characters() {
        assert_eq!(
            check("a\u{0}b.mp4"),
            Err(RenameRejection::InvalidChar('\0'))
        );
        assert_eq!(check("a\tb.mp4"), Err(RenameRejection::InvalidChar('\t')));
    }

    #[test]
    fn rejects_path_traversal_attempts() {
        assert_eq!(
            check("..\\evil.mp4"),
            Err(RenameRejection::InvalidChar('\\'))
        );
        assert_eq!(check("../evil.mp4"), Err(RenameRejection::InvalidChar('/')));
        assert_eq!(check(".."), Err(RenameRejection::TrailingDotOrSpace));
    }

    #[test]
    fn rejects_trailing_dot() {
        assert_eq!(
            check("movie.mp4."),
            Err(RenameRejection::TrailingDotOrSpace)
        );
    }

    #[test]
    fn reserved_names_are_rejected_case_insensitively_with_or_without_extension() {
        for input in [
            "CON.mp4",
            "con.mp4",
            "Nul.mkv",
            "com1.mp4",
            "LPT9.mp4",
            "aux.x.mp4",
            "con .mp4",
        ] {
            assert!(
                matches!(check(input), Err(RenameRejection::ReservedName(_))),
                "{input}"
            );
        }
    }

    #[test]
    fn names_merely_containing_or_near_a_reserved_name_are_accepted() {
        for input in [
            "console.mp4",
            "com10.mp4",
            "com0.mp4",
            "lpt.mp4",
            "a.con.mp4",
        ] {
            assert!(check(input).is_ok(), "{input}");
        }
    }

    #[test]
    fn length_boundary_is_255_utf16_units() {
        let ok = format!("{}.mp4", "a".repeat(251));
        assert_eq!(ok.encode_utf16().count(), 255);
        assert!(check(&ok).is_ok());
        let too_long = format!("{}.mp4", "a".repeat(252));
        assert_eq!(check(&too_long), Err(RenameRejection::TooLong));
    }

    #[test]
    fn length_counts_utf16_units_not_chars() {
        // U+20BB7 is one char but two UTF-16 units.
        let name = format!("{}.mp4", "\u{20BB7}".repeat(126));
        assert_eq!(name.chars().count(), 130);
        assert_eq!(name.encode_utf16().count(), 256);
        assert_eq!(check(&name), Err(RenameRejection::TooLong));
    }

    #[test]
    fn rejects_a_remaining_machine_dependent_char() {
        assert_eq!(
            check("movie②.mp4"),
            Err(RenameRejection::MachineDependentChar('②'))
        );
    }

    #[test]
    fn rejects_a_non_video_or_missing_extension() {
        assert_eq!(check("movie1.txt"), Err(RenameRejection::NotVideoExtension));
        assert_eq!(check("movie1"), Err(RenameRejection::NotVideoExtension));
        assert_eq!(
            check("movie1.mp4.bak"),
            Err(RenameRejection::NotVideoExtension)
        );
    }

    #[test]
    fn extension_check_is_case_insensitive() {
        assert_eq!(check("movie1.MP4"), Ok("movie1.MP4".to_string()));
    }

    #[test]
    fn display_messages_are_japanese() {
        assert_eq!(
            RenameRejection::Empty.to_string(),
            "新しい名前を入力してください。"
        );
        assert!(RenameRejection::MachineDependentChar('①')
            .to_string()
            .contains('①'));
    }
}
