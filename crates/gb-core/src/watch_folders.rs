//! Pure logic for merging newly-picked watch folders into the persisted
//! list (settings.watch_folders).

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

#[cfg(test)]
mod tests {
    use super::*;

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
