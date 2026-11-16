//! Portable directory resolution: all app data lives in a `GrayBrowser/`
//! folder next to the executable -- except when the executable already sits
//! directly inside a folder named `GrayBrowser` (the production NSIS
//! installer's per-user `%LOCALAPPDATA%\GrayBrowser\GrayBrowser.exe` layout),
//! in which case that folder itself *is* the data directory, so it isn't
//! nested a second time.

use std::path::{Path, PathBuf};

/// Given the running executable's path, returns the portable data directory.
///
/// If the executable's parent directory is already named `GrayBrowser`
/// (case-insensitive), that directory itself is returned unchanged --
/// otherwise a `GrayBrowser` subfolder is appended. Returns `None`
/// if `exe_path` has no parent directory.
pub fn resolve_app_dir(exe_path: &Path) -> Option<PathBuf> {
    let dir = exe_path.parent()?;
    let already_named_graybrowser = dir
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_lowercase() == "graybrowser");
    if already_named_graybrowser {
        Some(dir.to_path_buf())
    } else {
        Some(dir.join("GrayBrowser"))
    }
}

/// Characters that separate path components; Windows accepts both.
pub(crate) const PATH_SEPARATORS: [char; 2] = ['\\', '/'];

pub fn is_path_sep(c: char) -> bool {
    PATH_SEPARATORS.contains(&c)
}

/// Extracts the drive root of a Windows path, for drive-type detection: a
/// drive-letter path (`D:\Videos\a.mp4`) becomes
/// `D:\`, a UNC path (`\\nas\share\videos\a.mp4`) becomes `\\nas\share\`
/// (server+share is the smallest unit `GetDriveTypeW` can classify).
/// Already-a-root input passes through unchanged; `/` and `\` are treated as
/// equivalent separators. Returns `None` for a UNC path missing a server or
/// share component, or a path that isn't drive-letter/UNC shaped at all
/// (e.g. a relative path).
///
/// Pure string processing only -- deliberately kept in `gb-core` even though
/// it's Windows-path-shaped, to confine the actual `GetDriveTypeW` FFI call
/// to the adapter layer (`src-tauri::adapters::drive_type`) while keeping
/// this decomposition logic OS-independent and unit-testable here.
pub fn extract_drive_root(path: &str) -> Option<String> {
    let trimmed = path.trim_start();
    let mut chars = trimmed.chars();
    let first = chars.next()?;

    if is_path_sep(first) {
        // UNC: \\server\share\... (or //server/share/...)
        if chars.next().is_some_and(is_path_sep) {
            let rest = &trimmed[2..];
            let mut parts = rest.splitn(3, is_path_sep);
            let server = parts.next().filter(|s| !s.is_empty())?;
            let share = parts.next().filter(|s| !s.is_empty())?;
            return Some(format!("\\\\{server}\\{share}\\"));
        }
        return None;
    }

    // Drive letter: X:... (with or without a trailing separator)
    if first.is_ascii_alphabetic() && chars.next() == Some(':') {
        return Some(format!("{}:\\", first.to_ascii_uppercase()));
    }

    None
}

/// Builds the (unescaped-wildcard, unfinished) `LIKE` prefix for "every path
/// under `folder_path`" folder-sidebar filtering, honoring folder
/// *boundaries*: a plain prefix match
/// (`file_path LIKE 'folder_path%'`) would wrongly match `C:\Videos2\a.mp4`
/// when filtering on `C:\Videos`, since it never requires a separator after
/// the prefix. This normalizes `folder_path` to end in exactly one `\`
/// separator -- regardless of whether the caller's input already had a
/// trailing one (both shapes occur among the OS folder-picker's actual
/// return values: a plain folder like `C:\Videos` comes back without one,
/// while a drive root like `C:\` comes back with one) -- and escapes the
/// result via `search::escape_like_pattern` so a literal `%`/`_`/`\` inside
/// the folder path itself is matched literally rather than reinterpreted as
/// a `LIKE` wildcard or escape character.
///
/// The caller must append `'%'` to the returned string and use
/// `ESCAPE '\'` in the SQL -- this function only builds the prefix portion.
pub fn folder_like_prefix(folder_path: &str) -> String {
    crate::search::escape_like_pattern(&normalize_folder_path(folder_path))
}

/// Normalizes `folder_path` to end in exactly one trailing `\` separator,
/// regardless of whether the input already had one -- the same
/// normalization `folder_like_prefix` applies before escaping it into a SQL
/// `LIKE` pattern, but exposed unescaped here for callers that need to do
/// literal string work (prefix comparison/replacement) rather than build a
/// `LIKE` pattern. See `folder_like_prefix`'s docs for why both trailing-
/// separator shapes occur among real inputs (a plain folder like
/// `C:\Videos` vs. a drive root like `C:\`).
pub(crate) fn normalize_folder_path(folder_path: &str) -> String {
    format!("{}\\", folder_path.trim_end_matches('\\'))
}

/// Whether two watch-folder paths overlap: exact duplicates, or a
/// parent/child (containment) relationship in either direction. Used by the
/// folder-management dialog's path-edit feature to reject a new path that
/// would overlap an already-registered watch folder.
///
/// Boundary-safe like `folder_like_prefix`: both paths are normalized to a
/// single trailing separator first, so `C:\Videos` and `C:\Videos2` do
/// *not* conflict (different folders that merely share a string prefix),
/// while `C:\Videos` and `C:\Videos\Sub` (either direction) do. Comparison
/// is case-insensitive (ASCII-folded), matching NTFS path semantics --
/// same convention `reconcile` already uses for path comparison.
pub fn folder_paths_conflict(a: &str, b: &str) -> bool {
    let a = normalize_folder_path(a).to_lowercase();
    let b = normalize_folder_path(b).to_lowercase();
    a == b || a.starts_with(&b) || b.starts_with(&a)
}

/// Lowercased with `/` folded into `\`, the form in which two path strings
/// are compared. NTFS ignores case and accepts either separator, while the
/// strings in the DB and in the watch-folder list may have been written with
/// either.
fn comparable_path(s: &str) -> String {
    s.to_lowercase().replace('/', "\\")
}

/// Whether `path` lies strictly inside `folder`: a case-insensitive prefix
/// match that requires the prefix to end on a separator boundary, so
/// `W:\video2\a.mp4` is *not* under `W:\video` even though the strings share
/// a prefix. `folder` may or may not carry a trailing separator (a drive
/// root such as `W:\` and a plain folder such as `W:\video` both occur).
/// Works on the plain (non-`\\?\`) form that `videos.file_path` and the
/// watch-folder list are stored in. An empty `folder` contains nothing.
pub fn is_under_folder(path: &str, folder: &str) -> bool {
    let folder = comparable_path(folder);
    let folder = folder.trim_end_matches('\\');
    if folder.trim().is_empty() {
        return false;
    }
    let mut prefix = String::with_capacity(folder.len() + 1);
    prefix.push_str(folder);
    prefix.push('\\');
    comparable_path(path).starts_with(&prefix)
}

/// The entry of `folders` that `path` lies under, or `None`. When registered
/// folders nest (they are not supposed to, see `folder_paths_conflict`) the
/// longest, i.e. most specific, one wins.
pub fn owning_folder<'a>(path: &str, folders: &'a [String]) -> Option<&'a str> {
    folders
        .iter()
        .filter(|folder| is_under_folder(path, folder))
        .max_by_key(|folder| folder.trim_end_matches(['\\', '/']).len())
        .map(String::as_str)
}

/// Whether `path` lies under any of `folders`.
pub fn is_under_any_folder(path: &str, folders: &[String]) -> bool {
    folders.iter().any(|folder| is_under_folder(path, folder))
}

/// Rewrites `path`'s leading `old_folder` component to `new_folder`,
/// preserving everything under it unchanged -- the core computation behind
/// the folder-management dialog's path-edit feature: renaming a watched
/// folder must carry every video's relative position
/// under it forward untouched (UUID/tags/rating/created_at are never
/// touched by the caller either, only `file_path`).
///
/// The prefix match is case-insensitive (ASCII-folded, matching NTFS
/// semantics), so callers that pre-select rows via SQLite's own default
/// ASCII-caseless `LIKE` (`folder_like_prefix`) get consistent results here.
/// Returns `None` if `path` does not fall under `old_folder` at all -- for a
/// caller that already filtered by `folder_like_prefix`, this should not
/// happen, but is handled defensively rather than panicking or silently
/// mis-rewriting an unrelated path.
pub fn replace_folder_prefix(path: &str, old_folder: &str, new_folder: &str) -> Option<String> {
    let old_prefix = normalize_folder_path(old_folder);
    let new_prefix = normalize_folder_path(new_folder);
    let head = path.get(..old_prefix.len())?;
    if head.eq_ignore_ascii_case(&old_prefix) {
        Some(format!("{new_prefix}{}", &path[old_prefix.len()..]))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_drive_root_resolves_drive_letter_and_unc_paths() {
        let cases = [
            // Drive-letter paths, with the letter upper-cased.
            (r"D:\Videos\a.mp4", Some(r"D:\")),
            (r"d:\videos\a.mp4", Some(r"D:\")),
            (r"C:\", Some(r"C:\")),
            ("D:/Videos/a.mp4", Some(r"D:\")),
            // UNC paths reduce to `\\server\share\` whatever the separators.
            (r"\\nas\share\videos\a.mp4", Some(r"\\nas\share\")),
            ("//nas/share/videos/a.mp4", Some(r"\\nas\share\")),
            (r"\\nas/share\videos/a.mp4", Some(r"\\nas\share\")),
            // Already a root, with and without the trailing separator.
            (r"\\nas\share\", Some(r"\\nas\share\")),
            (r"\\nas\share", Some(r"\\nas\share\")),
        ];
        for (path, expected) in cases {
            assert_eq!(
                extract_drive_root(path),
                expected.map(str::to_string),
                "{path:?}"
            );
        }
    }

    #[test]
    fn extract_drive_root_rejects_paths_that_are_not_drive_or_unc_shaped() {
        for path in [
            r"\\nas",        // UNC missing the share
            r"\\nas\",       // UNC missing the share, with a separator
            r"Videos\a.mp4", // relative
            "",
            "D", // a letter with no colon
        ] {
            assert_eq!(extract_drive_root(path), None, "{path:?}");
        }
    }

    // `escape_like_pattern` doubles *every* backslash (not just a wildcard-
    // adjacent one), since `\` is itself the SQL `ESCAPE` character -- so
    // every `\` path separator below (including the trailing one this
    // function adds) shows up doubled in the expected output. See
    // `search::escape_like_pattern`'s own tests for that escaping in
    // isolation; the tests here exercise it composed with the trailing-
    // separator normalization.

    #[test]
    fn folder_like_prefix_normalizes_separators_and_escapes_like_metacharacters() {
        let cases = [
            // A missing trailing separator is appended...
            (r"C:\Videos", r"C:\\Videos\\"),
            // ...and an existing one is not doubled: both shapes the OS
            // folder picker can return must behave identically.
            (r"C:\Videos\", r"C:\\Videos\\"),
            (r"C:\", r"C:\\"),
            ("C:", r"C:\\"),
            // `_` and `%` must not act as LIKE wildcards.
            (r"C:\my_videos", r"C:\\my\_videos\\"),
            (r"C:\100%done", r"C:\\100\%done\\"),
            // Every separator is escaped, not just the trailing one.
            (r"C:\Videos\Sub", r"C:\\Videos\\Sub\\"),
        ];
        for (folder, expected) in cases {
            assert_eq!(folder_like_prefix(folder), expected, "{folder:?}");
        }
    }

    // --- folder_paths_conflict -----------------------------------------

    #[test]
    fn folder_paths_conflict_detects_an_exact_duplicate() {
        assert!(folder_paths_conflict(r"C:\Videos", r"C:\Videos"));
    }

    #[test]
    fn folder_paths_conflict_ignores_a_trailing_separator_difference() {
        assert!(folder_paths_conflict(r"C:\Videos", r"C:\Videos\"));
    }

    #[test]
    fn folder_paths_conflict_is_case_insensitive() {
        assert!(folder_paths_conflict(r"C:\Videos", r"c:\videos"));
    }

    #[test]
    fn folder_paths_conflict_detects_a_child_folder() {
        assert!(folder_paths_conflict(r"C:\Videos", r"C:\Videos\Sub"));
    }

    #[test]
    fn folder_paths_conflict_detects_a_parent_folder_the_other_way_round() {
        assert!(folder_paths_conflict(r"C:\Videos\Sub", r"C:\Videos"));
    }

    #[test]
    fn folder_paths_conflict_does_not_flag_a_sibling_with_a_shared_string_prefix() {
        // The classic boundary-safety fixture: "C:\Videos2" is not inside
        // "C:\Videos" even though it shares a string prefix.
        assert!(!folder_paths_conflict(r"C:\Videos", r"C:\Videos2"));
    }

    #[test]
    fn folder_paths_conflict_does_not_flag_unrelated_folders() {
        assert!(!folder_paths_conflict(r"C:\Videos", r"D:\Other"));
    }

    // --- replace_folder_prefix -------------------------------------------

    #[test]
    fn replace_folder_prefix_rewrites_a_direct_child_file() {
        assert_eq!(
            replace_folder_prefix(r"C:\Videos\a.mp4", r"C:\Videos", r"D:\Movies"),
            Some(r"D:\Movies\a.mp4".to_string())
        );
    }

    #[test]
    fn replace_folder_prefix_preserves_a_nested_subpath() {
        assert_eq!(
            replace_folder_prefix(r"C:\Videos\Sub\a.mp4", r"C:\Videos", r"D:\Movies"),
            Some(r"D:\Movies\Sub\a.mp4".to_string())
        );
    }

    #[test]
    fn replace_folder_prefix_handles_trailing_separators_on_either_input() {
        assert_eq!(
            replace_folder_prefix(r"C:\Videos\a.mp4", r"C:\Videos\", r"D:\Movies\"),
            Some(r"D:\Movies\a.mp4".to_string())
        );
    }

    #[test]
    fn replace_folder_prefix_is_case_insensitive_on_the_prefix_only() {
        // The folder-portion casing follows `new_folder`; the un-replaced
        // suffix (file/subfolder names) keeps its original casing untouched.
        assert_eq!(
            replace_folder_prefix(r"c:\videos\A.mp4", r"C:\Videos", r"D:\Movies"),
            Some(r"D:\Movies\A.mp4".to_string())
        );
    }

    #[test]
    fn replace_folder_prefix_returns_none_for_a_sibling_folder_with_a_shared_prefix() {
        assert_eq!(
            replace_folder_prefix(r"C:\Videos2\a.mp4", r"C:\Videos", r"D:\Movies"),
            None
        );
    }

    #[test]
    fn replace_folder_prefix_returns_none_for_an_unrelated_path() {
        assert_eq!(
            replace_folder_prefix(r"D:\Other\a.mp4", r"C:\Videos", r"D:\Movies"),
            None
        );
    }

    #[test]
    fn replace_folder_prefix_handles_a_drive_root_rename() {
        assert_eq!(
            replace_folder_prefix(r"C:\a.mp4", r"C:\", r"D:\"),
            Some(r"D:\a.mp4".to_string())
        );
    }

    /// `(path, folder, expected, why)`.
    #[test]
    fn is_under_folder_matches_only_strict_descendants() {
        let cases: &[(&str, &str, bool, &str)] = &[
            (r"C:\Videos\a.mp4", r"C:\Videos", true, "direct child"),
            (r"C:\Videos\Sub\a.mp4", r"C:\Videos", true, "nested"),
            (r"c:\videos\a.mp4", r"C:\Videos", true, "case-insensitive"),
            (
                r"W:\video\a.mp4",
                r"W:\video\",
                true,
                "folder with trailing separator",
            ),
            (
                r"C:\Videos2\a.mp4",
                r"C:\Videos",
                false,
                "shared string prefix",
            ),
            (
                r"W:\videos\a.mp4",
                r"W:\video\",
                false,
                "shared prefix, trailing separator",
            ),
            (r"D:\Other\a.mp4", r"C:\Videos", false, "unrelated"),
            (r"D:\v", r"D:\v", false, "the folder itself"),
            ("D:/v/a.mp4", r"D:\v", true, "slash path, backslash folder"),
            (r"D:\v\a.mp4", "D:/v/", true, "backslash path, slash folder"),
            ("D:/v2/a.mp4", "D:/v", false, "forward-slash shared prefix"),
            (r"W:\a.mp4", r"W:\", true, "drive root"),
            (r"W:\x\a.mp4", "W:", true, "drive without separator"),
            (r"V:\a.mp4", r"W:\", false, "other drive"),
            (
                r"D:\",
                r"D:\",
                true,
                "drive root itself (quirk: trailing separator counts as inside)",
            ),
            ("D:", r"D:\", false, "drive without separator itself"),
            (r"\\nas\share\v\a.mp4", r"\\NAS\share", true, "UNC"),
            (r"\\nas\share2\a.mp4", r"\\nas\share", false, "UNC boundary"),
            (r"\\nas\share\a.mp4", "", false, "empty folder"),
            (r"C:\a.mp4", "  ", false, "blank folder"),
            (r"C:\a.mp4", r"\", false, "separator-only folder"),
            (r"\a.mp4", r"\", false, "separator-only folder, rooted path"),
        ];
        for (path, folder, expected, why) in cases {
            assert_eq!(
                is_under_folder(path, folder),
                *expected,
                "{path} / {folder}: {why}"
            );
        }
    }

    #[test]
    fn owning_folder_picks_the_matching_and_most_specific_folder() {
        let folders = vec![
            r"W:\video".to_string(),
            r"V:\".to_string(),
            r"W:\video\sub".to_string(),
        ];
        assert_eq!(
            owning_folder(r"W:\video\a.mp4", &folders),
            Some(r"W:\video")
        );
        assert_eq!(
            owning_folder(r"W:\video\sub\a.mp4", &folders),
            Some(r"W:\video\sub")
        );
        assert_eq!(owning_folder(r"V:\x.mp4", &folders), Some(r"V:\"));
        assert_eq!(owning_folder(r"W:\video2\a.mp4", &folders), None);
        assert_eq!(owning_folder(r"C:\a.mp4", &[]), None);
    }

    #[test]
    fn is_under_any_folder_checks_every_entry() {
        let folders = vec![r"W:\video".to_string(), r"V:\".to_string()];
        assert!(is_under_any_folder(r"V:\a.mp4", &folders));
        assert!(!is_under_any_folder(r"W:\video2\a.mp4", &folders));
        assert!(!is_under_any_folder(r"W:\video\a.mp4", &[]));
    }
}
