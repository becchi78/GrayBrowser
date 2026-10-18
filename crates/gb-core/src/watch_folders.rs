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
