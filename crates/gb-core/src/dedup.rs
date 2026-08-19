//! Duplicate detection grouping logic.
//!
//! Duplicate detection is a two-stage process:
//!
//! 1. Group `status='online'` videos by `(quick_hash, file_size)`. Groups with
//!    only one member cannot be duplicates and are discarded. This stage is
//!    cheap: `quick_hash` is already stored on every online video.
//! 2. For each remaining candidate group, compute `full_hash` (BLAKE3) lazily
//!    -- only for the videos in that group, never eagerly for the whole
//!    library -- and sub-group by `full_hash` to confirm true duplicates.
//!
//! This module contains only the pure grouping logic. It does not touch the
//! database, read files, or decide which videos are `status='online'`; the
//! caller is expected to have already filtered to online videos and to have
//! populated `full_hash` (via `crate::hash::full_hash`) for the videos it
//! passes to `confirm_duplicates_by_full_hash`.

use std::collections::HashMap;

/// Minimal per-video info needed for duplicate grouping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoHashInfo {
    pub id: String,
    pub quick_hash: String,
    pub file_size: i64,
    pub full_hash: Option<String>,
}

/// Groups `videos` by `(quick_hash, file_size)`, returning only groups with
/// two or more members (single-member groups cannot be duplicates).
///
/// Videos with an empty `quick_hash` (`""`) are unconditionally excluded from
/// grouping, even from each other: an empty `quick_hash` is a placeholder
/// used for offline videos (see wb_import pipeline), not a real hash value,
/// so treating two empty-hash rows as "matching" would be a false positive.
///
/// Callers are expected to have already filtered `videos` to
/// `status='online'` rows; this function does not know about `status` and
/// only defends against the empty-`quick_hash` placeholder case.
pub fn group_candidates_by_quick_hash(videos: &[VideoHashInfo]) -> Vec<Vec<&VideoHashInfo>> {
    let mut groups: HashMap<(&str, i64), Vec<&VideoHashInfo>> = HashMap::new();

    for video in videos {
        if video.quick_hash.is_empty() {
            continue;
        }
        groups
            .entry((video.quick_hash.as_str(), video.file_size))
            .or_default()
            .push(video);
    }

    groups
        .into_values()
        .filter(|group| group.len() >= 2)
        .collect()
}

/// Given one `quick_hash`+`file_size` candidate group (as produced by
/// `group_candidates_by_quick_hash`), sub-groups the members by `full_hash`
/// and returns only the confirmed-duplicate sub-groups (two or more members
/// sharing the same `full_hash`), as lists of video ids.
///
/// Videos whose `full_hash` is `None` (not yet computed) are excluded from
/// every confirmed group; they neither block nor join a confirmation.
pub fn confirm_duplicates_by_full_hash(group: &[VideoHashInfo]) -> Vec<Vec<String>> {
    let mut groups: HashMap<&str, Vec<String>> = HashMap::new();

    for video in group {
        if let Some(full_hash) = video.full_hash.as_deref() {
            groups.entry(full_hash).or_default().push(video.id.clone());
        }
    }

    groups.into_values().filter(|ids| ids.len() >= 2).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(id: &str, quick_hash: &str, file_size: i64, full_hash: Option<&str>) -> VideoHashInfo {
        VideoHashInfo {
            id: id.to_string(),
            quick_hash: quick_hash.to_string(),
            file_size,
            full_hash: full_hash.map(|s| s.to_string()),
        }
    }

    fn group_ids(videos: &[VideoHashInfo]) -> Vec<Vec<String>> {
        group_candidates_by_quick_hash(videos)
            .into_iter()
            .map(|group| {
                let mut ids: Vec<String> = group.into_iter().map(|v| v.id.clone()).collect();
                ids.sort();
                ids
            })
            .collect()
    }

    #[test]
    fn groups_videos_sharing_quick_hash_and_file_size() {
        let videos = vec![
            video("a", "hash1", 100, None),
            video("b", "hash1", 100, None),
            video("c", "hash2", 100, None),
        ];
        let mut groups = group_ids(&videos);
        groups.sort();
        assert_eq!(groups, vec![vec!["a".to_string(), "b".to_string()]]);
    }

    #[test]
    fn excludes_groups_with_only_one_member() {
        let videos = vec![
            video("a", "hash1", 100, None),
            video("b", "hash2", 100, None),
            video("c", "hash3", 999, None),
        ];
        assert!(group_candidates_by_quick_hash(&videos).is_empty());
    }

    #[test]
    fn same_quick_hash_but_different_file_size_does_not_group() {
        let videos = vec![
            video("a", "hash1", 100, None),
            video("b", "hash1", 200, None),
        ];
        assert!(group_candidates_by_quick_hash(&videos).is_empty());
    }

    #[test]
    fn empty_quick_hash_rows_are_never_grouped_even_with_each_other() {
        let videos = vec![
            video("a", "", 100, None),
            video("b", "", 100, None),
            video("c", "", 100, None),
        ];
        assert!(
            group_candidates_by_quick_hash(&videos).is_empty(),
            "empty quick_hash placeholder rows must never be treated as matching"
        );
    }

    #[test]
    fn empty_quick_hash_row_is_excluded_while_real_matches_still_group() {
        let videos = vec![
            video("a", "", 100, None),
            video("b", "hash1", 100, None),
            video("c", "hash1", 100, None),
        ];
        let groups = group_ids(&videos);
        assert_eq!(groups, vec![vec!["b".to_string(), "c".to_string()]]);
    }

    #[test]
    fn confirm_splits_group_by_full_hash() {
        let group = vec![
            video("a", "hash1", 100, Some("full-x")),
            video("b", "hash1", 100, Some("full-x")),
            video("c", "hash1", 100, Some("full-y")),
        ];
        let mut confirmed = confirm_duplicates_by_full_hash(&group);
        for ids in confirmed.iter_mut() {
            ids.sort();
        }
        confirmed.sort();
        // "full-y" has only one member so it is not a confirmed duplicate group.
        assert_eq!(confirmed, vec![vec!["a".to_string(), "b".to_string()]]);
    }

    #[test]
    fn confirm_excludes_rows_with_no_full_hash_yet() {
        let group = vec![
            video("a", "hash1", 100, Some("full-x")),
            video("b", "hash1", 100, None),
            video("c", "hash1", 100, None),
        ];
        let confirmed = confirm_duplicates_by_full_hash(&group);
        assert!(
            confirmed.is_empty(),
            "rows without a computed full_hash must not form or join a confirmed group"
        );
    }

    #[test]
    fn confirm_returns_empty_when_all_full_hashes_differ() {
        let group = vec![
            video("a", "hash1", 100, Some("full-x")),
            video("b", "hash1", 100, Some("full-y")),
        ];
        assert!(confirm_duplicates_by_full_hash(&group).is_empty());
    }

    #[test]
    fn end_to_end_two_stage_flow_confirms_true_duplicates_only() {
        // Three videos share quick_hash+file_size (stage 1 candidate group).
        // Of those, two share full_hash (confirmed duplicates); the third has
        // a different full_hash and must not appear in any confirmed group.
        let videos = vec![
            video("a", "hash1", 100, Some("full-x")),
            video("b", "hash1", 100, Some("full-x")),
            video("c", "hash1", 100, Some("full-y")),
            video("d", "hash-unique", 50, None),
        ];

        let candidate_groups = group_candidates_by_quick_hash(&videos);
        assert_eq!(candidate_groups.len(), 1);
        assert_eq!(candidate_groups[0].len(), 3);

        let owned_group: Vec<VideoHashInfo> =
            candidate_groups[0].iter().map(|v| (*v).clone()).collect();
        let mut confirmed = confirm_duplicates_by_full_hash(&owned_group);
        for ids in confirmed.iter_mut() {
            ids.sort();
        }

        assert_eq!(confirmed, vec![vec!["a".to_string(), "b".to_string()]]);
        assert!(!confirmed.iter().flatten().any(|id| id == "c"));
        assert!(!confirmed.iter().flatten().any(|id| id == "d"));
    }
}
