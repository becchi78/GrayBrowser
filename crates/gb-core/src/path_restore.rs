//! Restores `?` in a legacy path to the real character.
//!
//! The old WhiteBrowser stored characters it could not convert (e.g. an em
//! dash, U+2014) as `?`, so the stored path no longer exists on disk. Windows
//! file names cannot contain `?`, which makes it a reliable marker for "one
//! character was lost here". The lost character is recovered by listing the
//! parent folder and looking for the one entry that fits.

use crate::paths::extract_drive_root;
use crate::ports::dir_lister::DirLister;

fn is_sep(c: char) -> bool {
    c == '\\' || c == '/'
}

/// Length in bytes of the leading root of `path` (`D:\`, `\\server\share\`),
/// as written in the original string so separators are preserved. `None` when
/// the path has no recognizable root followed by a separator.
fn root_len(path: &str) -> Option<usize> {
    // Validates the shape; the original text is measured below because
    // `extract_drive_root` normalizes separators and letter case.
    extract_drive_root(path)?;
    if path.trim_start().len() != path.len() {
        return None;
    }
    let first = path.chars().next()?;
    if is_sep(first) {
        // UNC: two separators, server, separator, share, separator.
        let mut end = 2;
        for _ in 0..2 {
            end += path[end..].find(is_sep)? + 1;
        }
        Some(end)
    } else {
        // Drive: `X:` followed by a separator.
        path[2..].starts_with(is_sep).then_some(3)
    }
}

/// Whether the `?`-bearing `segment` can be `name`: same number of characters,
/// every non-`?` character equal (ignoring case, as on Windows) and each `?`
/// standing for exactly one character.
fn segment_matches(segment: &str, name: &str) -> bool {
    let mut name_chars = name.chars();
    for s in segment.chars() {
        let Some(n) = name_chars.next() else {
            return false;
        };
        if s != '?' && s != n && !s.to_lowercase().eq(n.to_lowercase()) {
            return false;
        }
    }
    name_chars.next().is_none()
}

/// Resolves every `?` in `path` against the real directory contents.
///
/// Segments are resolved from the root downward, so a `?` in a folder name is
/// handled the same way as one in the file name. A segment is only replaced
/// when exactly one entry of its (already resolved) parent fits; zero or
/// several candidates, or a parent that cannot be listed, give `None` --
/// guessing could attach the metadata to a different file. Also `None` when
/// the path has no `?` to restore, has no drive/UNC root, or is a `\\?\`
/// extended-length path (its `?` is syntax, not a lost character).
///
/// The returned path keeps the original separators.
pub fn restore_path(path: &str, lister: &impl DirLister) -> Option<String> {
    if path.starts_with("\\\\?\\") || path.starts_with("//?/") {
        return None;
    }
    let root_len = root_len(path)?;
    let (root, rest) = path.split_at(root_len);
    if !rest.contains('?') {
        return None;
    }

    let mut resolved = root.to_string();
    for piece in rest.split_inclusive(is_sep) {
        let segment = piece.trim_end_matches(is_sep);
        let separator = &piece[segment.len()..];
        if segment.contains('?') {
            let dir = if resolved.len() == root.len() {
                root
            } else {
                resolved.trim_end_matches(is_sep)
            };
            let names = lister.list_names(dir)?;
            let mut matches = names.iter().filter(|n| segment_matches(segment, n));
            let found = matches.next()?;
            if matches.next().is_some() {
                return None;
            }
            resolved.push_str(found);
        } else {
            resolved.push_str(segment);
        }
        resolved.push_str(separator);
    }
    Some(resolved)
}

/// The last `\`- or `/`-separated component of `path`.
pub fn last_segment(path: &str) -> &str {
    path.rsplit(is_sep).next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fake_dir_lister::FakeDirLister;

    const DASH: &str = "\u{2014}";

    #[test]
    fn restores_the_single_matching_name() {
        let lister = FakeDirLister::default().with_dir(
            "D:\\v",
            &[&format!("a{DASH}b.mp4"), "other.mp4", "a-bb.mp4x"],
        );
        assert_eq!(
            restore_path("D:\\v\\a?b.mp4", &lister),
            Some(format!("D:\\v\\a{DASH}b.mp4"))
        );
    }

    #[test]
    fn several_matches_give_none() {
        let lister =
            FakeDirLister::default().with_dir("D:\\v", &[&format!("a{DASH}b.mp4"), "a-b.mp4"]);
        assert_eq!(restore_path("D:\\v\\a?b.mp4", &lister), None);
    }

    #[test]
    fn no_match_gives_none() {
        let lister = FakeDirLister::default().with_dir("D:\\v", &["x.mp4", "a--b.mp4"]);
        assert_eq!(restore_path("D:\\v\\a?b.mp4", &lister), None);
    }

    #[test]
    fn an_unlistable_parent_gives_none() {
        let lister = FakeDirLister::default();
        assert_eq!(restore_path("D:\\v\\a?b.mp4", &lister), None);
    }

    #[test]
    fn two_question_marks_in_one_segment_each_stand_for_one_character() {
        let lister = FakeDirLister::default().with_dir(
            "D:\\v",
            &[&format!("a{DASH}b{DASH}c.mp4"), "a-bc.mp4", "a-b-c-.mp4"],
        );
        assert_eq!(
            restore_path("D:\\v\\a?b?c.mp4", &lister),
            Some(format!("D:\\v\\a{DASH}b{DASH}c.mp4"))
        );
    }

    #[test]
    fn question_marks_in_different_segments_are_resolved_in_order() {
        let folder = format!("f{DASH}1");
        let lister = FakeDirLister::default()
            .with_dir("D:\\v", &[&folder])
            .with_dir(&format!("D:\\v\\{folder}"), &[&format!("a{DASH}b.mp4")]);
        assert_eq!(
            restore_path("D:\\v\\f?1\\a?b.mp4", &lister),
            Some(format!("D:\\v\\{folder}\\a{DASH}b.mp4"))
        );
    }

    #[test]
    fn a_question_mark_in_a_parent_folder_name_is_restored() {
        let folder = format!("f{DASH}1");
        let lister = FakeDirLister::default()
            .with_dir("D:\\", &[&folder, "other"])
            .with_dir(&format!("D:\\{folder}"), &["a.mp4"]);
        assert_eq!(
            restore_path("D:\\f?1\\a.mp4", &lister),
            Some(format!("D:\\{folder}\\a.mp4"))
        );
        assert_eq!(lister.call_count_for("D:\\"), 1);
    }

    #[test]
    fn a_failed_later_segment_discards_the_whole_restoration() {
        let folder = format!("f{DASH}1");
        let lister = FakeDirLister::default().with_dir("D:\\v", &[&folder]);
        assert_eq!(restore_path("D:\\v\\f?1\\a?b.mp4", &lister), None);
    }

    #[test]
    fn non_question_mark_characters_compare_case_insensitively() {
        let lister = FakeDirLister::default().with_dir("D:\\v", &[&format!("ABC{DASH}Def.MP4")]);
        assert_eq!(
            restore_path("D:\\v\\abc?def.mp4", &lister),
            Some(format!("D:\\v\\ABC{DASH}Def.MP4"))
        );
    }

    #[test]
    fn a_name_of_a_different_length_does_not_match() {
        let lister = FakeDirLister::default().with_dir("D:\\v", &["ab.mp4", "a--b.mp4"]);
        assert_eq!(restore_path("D:\\v\\a?b.mp4", &lister), None);
    }

    #[test]
    fn a_path_without_question_mark_is_none_and_lists_nothing() {
        let lister = FakeDirLister::default().with_dir("D:\\v", &["a.mp4"]);
        assert_eq!(restore_path("D:\\v\\a.mp4", &lister), None);
        assert!(lister.calls().is_empty());
    }

    #[test]
    fn segments_without_question_mark_are_not_listed() {
        let lister = FakeDirLister::default().with_dir("D:\\a\\b", &[&format!("x{DASH}.mp4")]);
        assert!(restore_path("D:\\a\\b\\x?.mp4", &lister).is_some());
        assert_eq!(lister.calls(), vec!["D:\\a\\b".to_string()]);
    }

    #[test]
    fn extended_length_prefix_is_none() {
        let lister = FakeDirLister::default().with_dir("\\\\?\\D:\\v", &["a.mp4"]);
        assert_eq!(restore_path("\\\\?\\D:\\v\\a?.mp4", &lister), None);
        assert!(lister.calls().is_empty());
    }

    #[test]
    fn a_path_without_a_root_is_none() {
        let lister = FakeDirLister::default();
        assert_eq!(restore_path("v\\a?.mp4", &lister), None);
        assert_eq!(restore_path("D:a?.mp4", &lister), None);
    }

    #[test]
    fn forward_slash_separators_are_supported_and_preserved() {
        let lister = FakeDirLister::default().with_dir("D:/v", &[&format!("a{DASH}b.mp4")]);
        assert_eq!(
            restore_path("D:/v/a?b.mp4", &lister),
            Some(format!("D:/v/a{DASH}b.mp4"))
        );
    }

    #[test]
    fn unc_paths_are_restored_with_the_share_as_root() {
        let lister =
            FakeDirLister::default().with_dir("\\\\nas\\share\\v", &[&format!("a{DASH}b.mp4")]);
        assert_eq!(
            restore_path("\\\\nas\\share\\v\\a?b.mp4", &lister),
            Some(format!("\\\\nas\\share\\v\\a{DASH}b.mp4"))
        );
    }

    #[test]
    fn a_file_directly_under_a_unc_share_lists_the_share_root() {
        let lister =
            FakeDirLister::default().with_dir("\\\\nas\\share\\", &[&format!("a{DASH}.mp4")]);
        assert_eq!(
            restore_path("\\\\nas\\share\\a?.mp4", &lister),
            Some(format!("\\\\nas\\share\\a{DASH}.mp4"))
        );
    }

    #[test]
    fn last_segment_splits_on_both_separators() {
        assert_eq!(last_segment("D:\\v\\a.mp4"), "a.mp4");
        assert_eq!(last_segment("D:/v/a.mp4"), "a.mp4");
        assert_eq!(last_segment("a.mp4"), "a.mp4");
    }
}
