//! Pure logic for the persisted watch-folder list (settings.watch_folders):
//! merging newly-picked folders, reordering, and per-folder update-check
//! filtering.

/// Appends `picked` to `existing`, skipping any that are already present.
/// Preserves `existing`'s order; new entries are appended in the order given.
pub fn merge(existing: Vec<String>, picked: Vec<String>) -> Vec<String> {
    let mut merged = existing;
    for folder in picked {
        if !merged.contains(&folder) {
            merged.push(folder);
        }
    }
    merged
}

/// The parent folder of `video_path`, split on the last `\` or `/` rather than
/// via `std::path::Path::parent`, whose notion of a separator depends on the
/// host OS while stored paths may use either. Returns the folder with `/`
/// folded into `\` and no trailing separator (a bare drive such as `D:` gets
/// its `\` back, since `D:` alone means "current directory on D"). `None` for
/// a path with no folder part.
fn parent_folder(video_path: &str) -> Option<String> {
    let idx = video_path.rfind(['\\', '/'])?;
    let folder = video_path[..idx].replace('/', "\\");
    let folder = folder.trim_end_matches('\\');
    if folder.is_empty() {
        return None;
    }
    if folder.ends_with(':') {
        return Some(format!("{folder}\\"));
    }
    Some(folder.to_string())
}

/// Whether `folder` still holds a `?` standing in for a character the legacy
/// path could not represent. Windows paths cannot contain `?`, so such a
/// folder can never be listed. The `\\?\` / `//?/` extended-length prefix is
/// syntax, not a lost character, and is ignored.
fn has_lost_char_marker(folder: &str) -> bool {
    let body = folder
        .strip_prefix("\\\\?\\")
        .or_else(|| folder.strip_prefix("//?/"))
        .unwrap_or(folder);
    body.contains('?')
}

/// Watch folders to register for a batch of imported videos: the distinct
/// parent folders of `video_paths`, minus those already covered by
/// `existing_folders`.
///
/// - Duplicates are collapsed case-insensitively and regardless of a trailing
///   separator or `/` vs `\`.
/// - A drive root (`D:\`) or UNC share root (`\\server\share\`) is never a
///   candidate: one stray file there must not turn a whole drive into a watch
///   folder. It is dropped before nesting is resolved so that it cannot
///   swallow the legitimate folders on the same drive.
/// - A candidate overlapping an existing watch folder in either direction
///   (equal, inside, or an ancestor of it) is dropped: it is already watched,
///   or adding it would watch the same files twice.
/// - Candidates nested inside another candidate collapse into the outer one;
///   unrelated siblings are never widened to their common parent, which could
///   pull in folders the user never meant to watch.
/// - A path whose parent folders still contain a `?` is never a candidate: it
///   would become a watch folder that can never be reached. A `?` only in the
///   file name does not matter, since the folder is still valid.
/// - Sorted ascending so the result is stable for the same input.
pub fn suggest_watch_folders(video_paths: &[String], existing_folders: &[String]) -> Vec<String> {
    let mut candidates: Vec<(String, String)> = Vec::new();
    for path in video_paths {
        let Some(folder) = parent_folder(path) else {
            continue;
        };
        if has_lost_char_marker(&folder) {
            continue;
        }
        let key = crate::paths::normalize_folder_path(&folder).to_lowercase();
        if candidates.iter().any(|(k, _)| *k == key) {
            continue;
        }
        let is_root = crate::paths::extract_drive_root(&folder)
            .is_some_and(|root| crate::paths::normalize_folder_path(&root).to_lowercase() == key);
        if is_root {
            continue;
        }
        if existing_folders
            .iter()
            .any(|existing| crate::paths::folder_paths_conflict(existing, &folder))
        {
            continue;
        }
        candidates.push((key, folder));
    }
    candidates.sort_by(|a, b| a.0.cmp(&b.0));

    // Sorted order puts an ancestor before everything inside it, so a single
    // pass against the already-kept folders is enough.
    let mut kept: Vec<String> = Vec::new();
    for (_, folder) in candidates {
        if kept
            .iter()
            .any(|outer| crate::paths::folder_paths_conflict(outer, &folder))
        {
            continue;
        }
        kept.push(folder);
    }
    kept
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReorderError {
    #[error("duplicate folder in the requested order: {0}")]
    Duplicate(String),
    #[error("folder is not registered: {0}")]
    Unknown(String),
    #[error("folder is missing from the requested order: {0}")]
    Missing(String),
}

/// Validates that `ordered` is a permutation of `existing` (same elements, no
/// duplicates, none added or dropped) and returns it. Rejecting anything else
/// keeps a stale screen from silently dropping or resurrecting folders that
/// changed since it was loaded.
pub fn reorder(existing: &[String], ordered: Vec<String>) -> Result<Vec<String>, ReorderError> {
    let mut seen: Vec<&String> = Vec::with_capacity(ordered.len());
    for folder in &ordered {
        if seen.contains(&folder) {
            return Err(ReorderError::Duplicate(folder.clone()));
        }
        if !existing.contains(folder) {
            return Err(ReorderError::Unknown(folder.clone()));
        }
        seen.push(folder);
    }
    if let Some(missing) = existing.iter().find(|f| !ordered.contains(f)) {
        return Err(ReorderError::Missing(missing.clone()));
    }
    Ok(ordered)
}

/// Returns `folders` without the ones in `disabled`, preserving order.
pub fn filter_checked(folders: &[String], disabled: &[String]) -> Vec<String> {
    folders
        .iter()
        .filter(|f| !disabled.contains(f))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn suggest_collects_distinct_parent_folders_sorted() {
        let got = suggest_watch_folders(&v(&[r"D:\b\x.mp4", r"D:\a\y.mp4", r"D:\b\z.mp4"]), &[]);
        assert_eq!(got, v(&[r"D:\a", r"D:\b"]));
    }

    #[test]
    fn suggest_dedups_case_insensitively_and_across_separators() {
        let got = suggest_watch_folders(
            &v(&[r"D:\Videos\a.mp4", r"d:\VIDEOS\b.mp4", "D:/Videos/c.mp4"]),
            &[],
        );
        assert_eq!(got, v(&[r"D:\Videos"]));
    }

    #[test]
    fn suggest_excludes_existing_folders_and_their_descendants() {
        let got = suggest_watch_folders(
            &v(&[
                r"D:\Videos\a.mp4",
                r"D:\Videos\Sub\b.mp4",
                r"d:\videos\sub\deep\c.mp4",
                r"E:\Other\d.mp4",
            ]),
            &v(&[r"D:\Videos\"]),
        );
        assert_eq!(got, v(&[r"E:\Other"]));
    }

    #[test]
    fn suggest_does_not_treat_a_sibling_sharing_a_prefix_as_existing() {
        let got = suggest_watch_folders(&v(&[r"D:\Videos2\a.mp4"]), &v(&[r"D:\Videos"]));
        assert_eq!(got, v(&[r"D:\Videos2"]));
    }

    #[test]
    fn suggest_merges_nested_candidates_into_the_outer_one() {
        let got = suggest_watch_folders(
            &v(&[r"D:\A\Sub\x.mp4", r"D:\A\y.mp4", r"D:\A\Sub\Deep\z.mp4"]),
            &[],
        );
        assert_eq!(got, v(&[r"D:\A"]));
    }

    #[test]
    fn suggest_does_not_merge_siblings_into_their_common_parent() {
        let got = suggest_watch_folders(&v(&[r"D:\A\x\1.mp4", r"D:\A\y\2.mp4"]), &[]);
        assert_eq!(got, v(&[r"D:\A\x", r"D:\A\y"]));
    }

    #[test]
    fn suggest_never_proposes_a_drive_root() {
        assert!(suggest_watch_folders(&v(&[r"D:\a.mp4"]), &[]).is_empty());
        assert!(suggest_watch_folders(&v(&["D:/a.mp4"]), &[]).is_empty());
        assert!(suggest_watch_folders(&v(&[r"d:\a.mp4"]), &[]).is_empty());
    }

    #[test]
    fn suggest_root_file_does_not_swallow_other_folders_on_the_drive() {
        let got = suggest_watch_folders(
            &v(&[r"D:\stray.mp4", r"D:\Uncensored\a.mp4", r"D:\Other\b.mp4"]),
            &[],
        );
        assert_eq!(got, v(&[r"D:\Other", r"D:\Uncensored"]));
    }

    #[test]
    fn suggest_never_proposes_a_unc_share_root() {
        assert!(suggest_watch_folders(&v(&[r"\\nas\share\a.mp4"]), &[]).is_empty());
        let got = suggest_watch_folders(&v(&[r"\\nas\share\a.mp4", r"\\nas\share\v\b.mp4"]), &[]);
        assert_eq!(got, v(&[r"\\nas\share\v"]));
    }

    #[test]
    fn suggest_drops_a_candidate_that_is_an_ancestor_of_an_existing_folder() {
        let got = suggest_watch_folders(&v(&[r"D:\A\x.mp4"]), &v(&[r"D:\A\B"]));
        assert!(got.is_empty());
    }

    #[test]
    fn suggest_still_adds_a_sibling_of_an_existing_folder() {
        let got = suggest_watch_folders(&v(&[r"D:\A\C\x.mp4"]), &v(&[r"D:\A\B"]));
        assert_eq!(got, v(&[r"D:\A\C"]));
    }

    #[test]
    fn suggest_keeps_unc_folders() {
        let got = suggest_watch_folders(&v(&[r"\\nas\share\v\a.mp4"]), &[]);
        assert_eq!(got, v(&[r"\\nas\share\v"]));
    }

    #[test]
    fn suggest_skips_paths_without_a_folder_part() {
        assert!(suggest_watch_folders(&v(&["a.mp4", "/a.mp4", ""]), &[]).is_empty());
    }

    #[test]
    fn suggest_of_empty_input_is_empty() {
        assert!(suggest_watch_folders(&[], &v(&[r"D:\x"])).is_empty());
    }

    #[test]
    fn suggest_drops_a_candidate_equal_to_an_existing_folder() {
        let got = suggest_watch_folders(&v(&[r"D:\Videos\a.mp4"]), &v(&[r"d:\videos"]));
        assert!(got.is_empty());
    }

    #[test]
    fn suggest_drops_a_path_whose_parent_folder_holds_a_question_mark() {
        assert!(suggest_watch_folders(&v(&[r"W:\a?b\c.mp4"]), &[]).is_empty());
        assert!(suggest_watch_folders(&v(&[r"W:\ok\a?b\c.mp4"]), &[]).is_empty());
        assert!(suggest_watch_folders(&v(&[r"\\nas\share\a?\c.mp4"]), &[]).is_empty());
        assert!(suggest_watch_folders(&v(&["W:/a?b/c.mp4"]), &[]).is_empty());
    }

    #[test]
    fn suggest_keeps_the_parent_when_only_the_file_name_has_a_question_mark() {
        let got = suggest_watch_folders(&v(&[r"W:\ok\a?b.mp4"]), &[]);
        assert_eq!(got, v(&[r"W:\ok"]));
    }

    #[test]
    fn suggest_treats_the_extended_length_prefix_as_syntax_not_a_lost_char() {
        let got = suggest_watch_folders(&v(&[r"\\?\D:\v\a.mp4"]), &[]);
        assert_eq!(got, v(&[r"\\?\D:\v"]));
        let got = suggest_watch_folders(&v(&["//?/D:/v/a.mp4"]), &[]);
        assert_eq!(got, v(&[r"\\?\D:\v"]));
        // A real `?` after the prefix is still a lost character.
        assert!(suggest_watch_folders(&v(&[r"\\?\D:\v?\a.mp4"]), &[]).is_empty());
    }

    #[test]
    fn suggest_question_marks_in_some_paths_do_not_affect_other_candidates() {
        let got = suggest_watch_folders(
            &v(&[
                r"W:\a?b\1.mp4",
                r"W:\good\2.mp4",
                r"W:\c?\d?\3.mp4",
                r"D:\vids\4.mp4",
                r"W:\good\sub\5.mp4",
            ]),
            &[],
        );
        assert_eq!(got, v(&[r"D:\vids", r"W:\good"]));
    }

    #[test]
    fn reorder_accepts_a_permutation_and_returns_the_new_order() {
        assert_eq!(
            reorder(&v(&["A", "B", "C"]), v(&["C", "A", "B"])).unwrap(),
            v(&["C", "A", "B"])
        );
    }

    #[test]
    fn reorder_accepts_the_unchanged_order() {
        assert_eq!(
            reorder(&v(&["A", "B"]), v(&["A", "B"])).unwrap(),
            v(&["A", "B"])
        );
    }

    #[test]
    fn reorder_of_empty_lists_is_empty() {
        assert_eq!(reorder(&[], vec![]).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn reorder_rejects_a_missing_element() {
        assert_eq!(
            reorder(&v(&["A", "B", "C"]), v(&["A", "B"])),
            Err(ReorderError::Missing("C".into()))
        );
    }

    #[test]
    fn reorder_rejects_an_empty_order_for_a_non_empty_list() {
        assert!(reorder(&v(&["A"]), vec![]).is_err());
    }

    #[test]
    fn reorder_rejects_an_unknown_element() {
        assert_eq!(
            reorder(&v(&["A", "B"]), v(&["A", "B", "X"])),
            Err(ReorderError::Unknown("X".into()))
        );
        assert!(reorder(&v(&["A", "B"]), v(&["A", "X"])).is_err());
    }

    #[test]
    fn reorder_rejects_a_duplicate_even_when_the_length_matches() {
        assert_eq!(
            reorder(&v(&["A", "B"]), v(&["A", "A"])),
            Err(ReorderError::Duplicate("A".into()))
        );
    }

    #[test]
    fn filter_checked_drops_disabled_folders_and_keeps_order() {
        assert_eq!(
            filter_checked(&v(&["A", "B", "C", "D"]), &v(&["D", "B"])),
            v(&["A", "C"])
        );
    }

    #[test]
    fn filter_checked_with_no_disabled_returns_all() {
        assert_eq!(filter_checked(&v(&["A", "B"]), &[]), v(&["A", "B"]));
    }

    #[test]
    fn filter_checked_ignores_disabled_entries_not_in_folders() {
        assert_eq!(filter_checked(&v(&["A"]), &v(&["Z"])), v(&["A"]));
    }

    #[test]
    fn filter_checked_with_everything_disabled_or_empty_is_empty() {
        assert!(filter_checked(&v(&["A", "B"]), &v(&["A", "B"])).is_empty());
        assert!(filter_checked(&[], &v(&["A"])).is_empty());
    }

    #[test]
    fn appends_new_folders_to_existing() {
        let existing = vec!["C:/Videos".to_string()];
        let picked = vec!["D:/Movies".to_string()];
        assert_eq!(merge(existing, picked), vec!["C:/Videos", "D:/Movies"]);
    }

    #[test]
    fn does_not_duplicate_already_present_folders() {
        let existing = vec!["C:/Videos".to_string()];
        let picked = vec!["C:/Videos".to_string(), "D:/Movies".to_string()];
        assert_eq!(merge(existing, picked), vec!["C:/Videos", "D:/Movies"]);
    }

    #[test]
    fn preserves_existing_order_and_appends_new_ones_in_given_order() {
        let existing = vec!["A".to_string(), "B".to_string()];
        let picked = vec!["C".to_string(), "D".to_string()];
        assert_eq!(merge(existing, picked), vec!["A", "B", "C", "D"]);
    }

    #[test]
    fn empty_picked_list_returns_existing_unchanged() {
        let existing = vec!["C:/Videos".to_string()];
        assert_eq!(merge(existing.clone(), vec![]), existing);
    }

    #[test]
    fn empty_existing_list_returns_just_the_picked_folders() {
        let picked = vec!["C:/Videos".to_string()];
        assert_eq!(merge(vec![], picked.clone()), picked);
    }
}
