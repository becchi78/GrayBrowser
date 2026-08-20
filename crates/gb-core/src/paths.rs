//! Portable directory resolution: all app data lives in a `GrayBrowser/`
//! folder next to the executable -- except when the executable already sits
//! directly inside a folder named `GrayBrowser` (the production NSIS
//! installer's per-user `%LOCALAPPDATA%\GrayBrowser\GrayBrowser.exe` layout),
//! in which case that folder itself *is* the data directory, so it isn't
//! nested a second time.

use std::path::{Path, PathBuf};

use xxhash_rust::xxh64::xxh64;

/// Given the running executable's path, returns the portable data directory.
///
/// If the executable's parent directory is already named `GrayBrowser`
/// (case-insensitive), that directory itself is returned unchanged --
/// otherwise a `GrayBrowser` subfolder is appended, as before. Returns `None`
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
    fn is_sep(c: char) -> bool {
        c == '\\' || c == '/'
    }

    let trimmed = path.trim_start();
    let mut chars = trimmed.chars();
    let first = chars.next()?;

    if is_sep(first) {
        // UNC: \\server\share\... (or //server/share/...)
        if chars.next().is_some_and(is_sep) {
            let rest = &trimmed[2..];
            let mut parts = rest.splitn(3, is_sep);
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
pub fn normalize_folder_path(folder_path: &str) -> String {
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
/// same convention `reconcile::is_under` already uses for path comparison.
pub fn folder_paths_conflict(a: &str, b: &str) -> bool {
    let a = normalize_folder_path(a).to_lowercase();
    let b = normalize_folder_path(b).to_lowercase();
    a == b || a.starts_with(&b) || b.starts_with(&a)
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

/// Seed for `thumbnail_folder_subdir`'s `xxh64` hash. A fixed constant
/// (rather than `0`-by-convention like `hash::quick_hash`'s
/// `QUICK_HASH_SEED`) only to give this call site its own named constant --
/// there's no requirement that it match any other hash in the codebase,
/// since subdirectory names never need to compare equal to a `quick_hash`/
/// `full_hash` value.
const THUMBNAIL_SUBDIR_HASH_SEED: u64 = 0;

/// Derives the 16-hex-digit subdirectory name a registered folder's
/// thumbnails are grouped under (`thumbnails/<this>/`), so a folder can be
/// identified by a short, filesystem-safe, collision-resistant name even
/// though `settings.watch_folders` has no numeric/UUID id of its own -- the
/// path string *is* the folder's identity (see module-level context in the
/// call sites under `src-tauri/src/thumbnail/`).
///
/// Deterministic and pure: the same `folder_path` (up to the
/// case/trailing-separator differences `normalize_folder_path` already
/// treats as equivalent everywhere else in this file, e.g.
/// `folder_paths_conflict`) always yields the same subdirectory name, with
/// no dependency on registration order or any other mutable state -- unlike
/// an incrementing id, this survives folders being added/removed/re-added
/// without ever needing a migration of its own.
pub fn thumbnail_folder_subdir(folder_path: &str) -> String {
    let normalized = normalize_folder_path(folder_path).to_lowercase();
    let hash = xxh64(normalized.as_bytes(), THUMBNAIL_SUBDIR_HASH_SEED);
    format!("{hash:016x}")
}

/// Fixed subdirectory name for thumbnails belonging to a video whose
/// `file_path` doesn't currently fall under any registered watch folder
/// (e.g. the folder was removed from `settings.watch_folders` after the
/// video was scanned, but the video row itself hasn't been cleaned up yet).
/// `thumbnail_folder_subdir` always returns a lowercase 16-hex-digit string,
/// which can never equal this name, so the two namespaces never collide.
pub const THUMBNAIL_UNASSIGNED_SUBDIR: &str = "_unassigned";

/// Whether `path` falls under `folder` (including `path` being `folder`
/// itself), using the same boundary-safe, case-insensitive comparison as
/// `folder_paths_conflict`/`folder_like_prefix` -- `C:\Videos2` is not under
/// `C:\Videos` even though it shares a string prefix.
pub fn is_path_under_folder(path: &str, folder: &str) -> bool {
    let folder_norm = normalize_folder_path(folder).to_lowercase();
    let path_norm = normalize_folder_path(path).to_lowercase();
    // The equality check handles `path == folder` (the "folder itself"
    // case): normalizing both sides the same way means a plain
    // `starts_with` alone would miss it, since `path_norm` in that case is
    // exactly as long as `folder_norm`, not longer.
    path_norm == folder_norm || path_norm.starts_with(&folder_norm)
}

/// Resolves the thumbnail subdirectory `file_path` belongs to, given the
/// currently-registered `watch_folders`: the subdirectory of whichever
/// registered folder `file_path` falls under, or
/// `THUMBNAIL_UNASSIGNED_SUBDIR` if none does.
///
/// `folder_paths_conflict` is enforced at registration time to reject
/// nested watch folders, so more than one match should not normally occur
/// -- but a rename can transiently leave `file_path` matching more than one
/// entry in `watch_folders` (e.g. mid-rename bookkeeping, or a future
/// relaxation of that constraint), so this defensively picks the most
/// specific (longest normalized path) match rather than an arbitrary one.
pub fn resolve_thumbnail_subdir(watch_folders: &[String], file_path: &str) -> String {
    watch_folders
        .iter()
        .filter(|folder| is_path_under_folder(file_path, folder))
        .max_by_key(|folder| normalize_folder_path(folder).len())
        .map(|folder| thumbnail_folder_subdir(folder))
        .unwrap_or_else(|| THUMBNAIL_UNASSIGNED_SUBDIR.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // `std::path::Path`'s separator interpretation is compile-target-OS
    // dependent, and these two tests exercise Windows-shaped path strings
    // specifically -- they don't parse the same way when natively built for
    // Linux. `resolve_app_dir` itself stays OS-independent and unmodified.
    #[test]
    #[cfg(windows)]
    fn resolves_to_a_graybrowser_folder_next_to_the_exe() {
        let exe = Path::new(r"C:\app\GrayBrowser.exe");
        assert_eq!(
            resolve_app_dir(exe),
            Some(PathBuf::from(r"C:\app\GrayBrowser"))
        );
    }

    #[test]
    #[cfg(windows)]
    fn returns_none_when_the_exe_path_has_no_parent() {
        // A bare drive root has no parent directory to resolve against.
        let exe = Path::new(r"C:\");
        assert_eq!(resolve_app_dir(exe), None);
    }

    #[test]
    #[cfg(windows)]
    fn does_not_double_nest_when_the_parent_is_already_named_graybrowser() {
        // The production NSIS installer's `currentUser` layout: the exe
        // itself lives directly inside `%LOCALAPPDATA%\GrayBrowser\`.
        let exe = Path::new(r"C:\Users\me\AppData\Local\GrayBrowser\GrayBrowser.exe");
        assert_eq!(
            resolve_app_dir(exe),
            Some(PathBuf::from(r"C:\Users\me\AppData\Local\GrayBrowser"))
        );
    }

    #[test]
    #[cfg(windows)]
    fn does_not_double_nest_regardless_of_the_parent_folder_names_casing() {
        let exe = Path::new(r"C:\Users\me\AppData\Local\graybrowser\GrayBrowser.exe");
        assert_eq!(
            resolve_app_dir(exe),
            Some(PathBuf::from(r"C:\Users\me\AppData\Local\graybrowser"))
        );
    }

    #[test]
    fn extract_drive_root_handles_a_drive_letter_path() {
        assert_eq!(
            extract_drive_root(r"D:\Videos\a.mp4"),
            Some(r"D:\".to_string())
        );
    }

    #[test]
    fn extract_drive_root_uppercases_a_lowercase_drive_letter() {
        assert_eq!(
            extract_drive_root(r"d:\videos\a.mp4"),
            Some(r"D:\".to_string())
        );
    }

    #[test]
    fn extract_drive_root_handles_a_bare_drive_root() {
        assert_eq!(extract_drive_root(r"C:\"), Some(r"C:\".to_string()));
    }

    #[test]
    fn extract_drive_root_handles_forward_slashes() {
        assert_eq!(
            extract_drive_root("D:/Videos/a.mp4"),
            Some(r"D:\".to_string())
        );
    }

    #[test]
    fn extract_drive_root_handles_a_unc_path() {
        assert_eq!(
            extract_drive_root(r"\\nas\share\videos\a.mp4"),
            Some(r"\\nas\share\".to_string())
        );
    }

    #[test]
    fn extract_drive_root_handles_a_unc_path_with_forward_slashes() {
        assert_eq!(
            extract_drive_root("//nas/share/videos/a.mp4"),
            Some(r"\\nas\share\".to_string())
        );
    }

    #[test]
    fn extract_drive_root_handles_a_unc_path_with_mixed_separators() {
        assert_eq!(
            extract_drive_root(r"\\nas/share\videos/a.mp4"),
            Some(r"\\nas\share\".to_string())
        );
    }

    #[test]
    fn extract_drive_root_handles_an_already_root_unc_path() {
        assert_eq!(
            extract_drive_root(r"\\nas\share\"),
            Some(r"\\nas\share\".to_string())
        );
        assert_eq!(
            extract_drive_root(r"\\nas\share"),
            Some(r"\\nas\share\".to_string())
        );
    }

    #[test]
    fn extract_drive_root_rejects_a_unc_path_missing_a_share() {
        assert_eq!(extract_drive_root(r"\\nas"), None);
        assert_eq!(extract_drive_root(r"\\nas\"), None);
    }

    #[test]
    fn extract_drive_root_rejects_a_relative_path() {
        assert_eq!(extract_drive_root(r"Videos\a.mp4"), None);
    }

    #[test]
    fn extract_drive_root_rejects_an_empty_string() {
        assert_eq!(extract_drive_root(""), None);
    }

    #[test]
    fn extract_drive_root_rejects_a_single_letter_with_no_colon() {
        assert_eq!(extract_drive_root("D"), None);
    }

    // `escape_like_pattern` doubles *every* backslash (not just a wildcard-
    // adjacent one), since `\` is itself the SQL `ESCAPE` character -- so
    // every `\` path separator below (including the trailing one this
    // function adds) shows up doubled in the expected output. See
    // `search::escape_like_pattern`'s own tests for that escaping in
    // isolation; the tests here exercise it composed with the trailing-
    // separator normalization.

    #[test]
    fn folder_like_prefix_appends_a_separator_when_missing() {
        assert_eq!(folder_like_prefix(r"C:\Videos"), r"C:\\Videos\\");
    }

    #[test]
    fn folder_like_prefix_does_not_double_an_existing_separator() {
        // Same normalized form (and therefore same escaped output) as the
        // no-trailing-separator case above -- this is the "both shapes the
        // OS folder picker can return must behave identically" guarantee.
        assert_eq!(folder_like_prefix(r"C:\Videos\"), r"C:\\Videos\\");
    }

    #[test]
    fn folder_like_prefix_normalizes_a_bare_drive_root() {
        assert_eq!(folder_like_prefix(r"C:\"), r"C:\\");
        assert_eq!(folder_like_prefix("C:"), r"C:\\");
    }

    #[test]
    fn folder_like_prefix_escapes_underscore_so_it_is_not_a_wildcard() {
        assert_eq!(folder_like_prefix(r"C:\my_videos"), r"C:\\my\_videos\\");
    }

    #[test]
    fn folder_like_prefix_escapes_percent_so_it_is_not_a_wildcard() {
        assert_eq!(folder_like_prefix(r"C:\100%done"), r"C:\\100\%done\\");
    }

    #[test]
    fn folder_like_prefix_escapes_every_separator_not_just_the_trailing_one() {
        assert_eq!(folder_like_prefix(r"C:\Videos\Sub"), r"C:\\Videos\\Sub\\");
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

    // --- thumbnail_folder_subdir -----------------------------------------

    #[test]
    fn thumbnail_folder_subdir_returns_16_lowercase_hex_digits() {
        let subdir = thumbnail_folder_subdir(r"C:\Videos");
        assert_eq!(subdir.len(), 16);
        assert!(subdir
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn thumbnail_folder_subdir_is_the_same_regardless_of_casing() {
        assert_eq!(
            thumbnail_folder_subdir(r"C:\Videos"),
            thumbnail_folder_subdir(r"c:\videos")
        );
    }

    #[test]
    fn thumbnail_folder_subdir_is_the_same_regardless_of_a_trailing_separator() {
        assert_eq!(
            thumbnail_folder_subdir(r"C:\Videos"),
            thumbnail_folder_subdir(r"C:\Videos\")
        );
    }

    #[test]
    fn thumbnail_folder_subdir_differs_for_different_folders() {
        assert_ne!(
            thumbnail_folder_subdir(r"C:\Videos"),
            thumbnail_folder_subdir(r"D:\Movies")
        );
    }

    #[test]
    fn thumbnail_folder_subdir_never_collides_with_the_unassigned_name() {
        // Sanity-check on the invariant `THUMBNAIL_UNASSIGNED_SUBDIR`'s doc
        // comment relies on: a 16-hex-digit hash can never equal `"_unassigned"`.
        assert_ne!(
            thumbnail_folder_subdir(r"C:\Videos"),
            THUMBNAIL_UNASSIGNED_SUBDIR
        );
    }

    // --- is_path_under_folder ---------------------------------------------

    #[test]
    fn is_path_under_folder_matches_a_direct_child_file() {
        assert!(is_path_under_folder(r"C:\Videos\a.mp4", r"C:\Videos"));
    }

    #[test]
    fn is_path_under_folder_matches_a_nested_subpath() {
        assert!(is_path_under_folder(r"C:\Videos\Sub\a.mp4", r"C:\Videos"));
    }

    #[test]
    fn is_path_under_folder_matches_the_folder_itself() {
        assert!(is_path_under_folder(r"C:\Videos", r"C:\Videos"));
    }

    #[test]
    fn is_path_under_folder_is_case_insensitive() {
        assert!(is_path_under_folder(r"c:\videos\a.mp4", r"C:\Videos"));
    }

    #[test]
    fn is_path_under_folder_does_not_flag_a_sibling_with_a_shared_string_prefix() {
        // The classic boundary-safety fixture: "C:\Videos2" is not inside
        // "C:\Videos" even though it shares a string prefix.
        assert!(!is_path_under_folder(r"C:\Videos2\a.mp4", r"C:\Videos"));
    }

    #[test]
    fn is_path_under_folder_does_not_flag_an_unrelated_path() {
        assert!(!is_path_under_folder(r"D:\Other\a.mp4", r"C:\Videos"));
    }

    // --- resolve_thumbnail_subdir -------------------------------------------

    #[test]
    fn resolve_thumbnail_subdir_returns_unassigned_when_no_folder_matches() {
        let folders = vec![r"C:\Videos".to_string()];
        assert_eq!(
            resolve_thumbnail_subdir(&folders, r"D:\Other\a.mp4"),
            THUMBNAIL_UNASSIGNED_SUBDIR
        );
    }

    #[test]
    fn resolve_thumbnail_subdir_returns_unassigned_for_an_empty_watch_folder_list() {
        assert_eq!(
            resolve_thumbnail_subdir(&[], r"C:\Videos\a.mp4"),
            THUMBNAIL_UNASSIGNED_SUBDIR
        );
    }

    #[test]
    fn resolve_thumbnail_subdir_returns_the_matching_folders_hash() {
        let folders = vec![r"C:\Videos".to_string()];
        assert_eq!(
            resolve_thumbnail_subdir(&folders, r"C:\Videos\a.mp4"),
            thumbnail_folder_subdir(r"C:\Videos")
        );
    }

    #[test]
    fn resolve_thumbnail_subdir_picks_the_most_specific_match_among_nested_entries() {
        // Ordinarily `folder_paths_conflict` prevents both a parent and a
        // child folder from being registered at the same time, but this
        // exercises the defensive "most specific wins" tie-break for the
        // rare transient case where it happens anyway (e.g. mid-rename).
        let folders = vec![r"C:\Videos".to_string(), r"C:\Videos\Sub".to_string()];
        assert_eq!(
            resolve_thumbnail_subdir(&folders, r"C:\Videos\Sub\a.mp4"),
            thumbnail_folder_subdir(r"C:\Videos\Sub")
        );
    }
}
