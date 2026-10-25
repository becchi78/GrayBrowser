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
    /// `full_hash` is absent because the file's folder cannot be reached
    /// right now, so whether the content matches is unknowable at the moment.
    /// Distinct from a plain `None`, which may be an I/O failure on a
    /// reachable file that says nothing about reachability.
    pub unverifiable: bool,
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

/// Given one candidate group and the sub-groups `confirm_duplicates_by_full_hash`
/// confirmed for it, returns the ids that should be shown as "candidates whose
/// content could not be verified", or `None` when there is nothing to show.
///
/// Shown only when at least one member is `unverifiable` and at least two
/// members remain outside every confirmed sub-group. Those remaining members
/// are the `unverifiable` ones plus members whose hash is known but which have
/// no matching partner (they may match an unverifiable member once it can be
/// read). Confirmed members are never returned, so no member appears in two
/// groups, and a plain `None` hash (I/O failure) is never treated as a
/// candidate. A single leftover `unverifiable` member is not shown: with no
/// second member to pair against it is no longer a duplicate candidate by
/// itself, and it is settled when the folder comes back.
pub fn unverified_candidates(
    group: &[VideoHashInfo],
    confirmed: &[Vec<String>],
) -> Option<Vec<String>> {
    if !group.iter().any(|v| v.unverifiable) {
        return None;
    }
    let ids: Vec<String> = group
        .iter()
        .filter(|v| v.unverifiable || v.full_hash.is_some())
        .filter(|v| !confirmed.iter().any(|ids| ids.contains(&v.id)))
        .map(|v| v.id.clone())
        .collect();
    (ids.len() >= 2).then_some(ids)
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
            unverifiable: false,
        }
    }

    fn unverifiable(id: &str) -> VideoHashInfo {
        VideoHashInfo {
            unverifiable: true,
            ..video(id, "hash1", 100, None)
        }
    }

    fn candidates(group: &[VideoHashInfo]) -> Option<Vec<String>> {
        let confirmed = confirm_duplicates_by_full_hash(group);
        unverified_candidates(group, &confirmed).map(|mut ids| {
            ids.sort();
            ids
        })
    }

    fn ids(list: &[&str]) -> Option<Vec<String>> {
        Some(list.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn two_unverifiable_members_are_candidates() {
        assert_eq!(
            candidates(&[unverifiable("a"), unverifiable("b")]),
            ids(&["a", "b"])
        );
    }

    #[test]
    fn known_hash_member_pairs_with_an_unverifiable_one() {
        let group = [video("a", "hash1", 100, Some("full-x")), unverifiable("b")];
        assert_eq!(candidates(&group), ids(&["a", "b"]));
    }

    #[test]
    fn three_or_more_unverifiable_members_are_all_candidates() {
        let group = [unverifiable("a"), unverifiable("b"), unverifiable("c")];
        assert_eq!(candidates(&group), ids(&["a", "b", "c"]));
    }

    #[test]
    fn confirmed_members_are_excluded_and_a_lone_unverifiable_third_is_hidden() {
        let group = [
            video("a", "hash1", 100, Some("full-x")),
            video("b", "hash1", 100, Some("full-x")),
            unverifiable("c"),
        ];
        assert_eq!(candidates(&group), None);
    }

    #[test]
    fn confirmed_group_plus_two_unverifiable_members_shows_only_the_unverifiable_pair() {
        let group = [
            video("a", "hash1", 100, Some("full-x")),
            video("b", "hash1", 100, Some("full-x")),
            unverifiable("c"),
            unverifiable("d"),
        ];
        assert_eq!(candidates(&group), ids(&["c", "d"]));
    }

    #[test]
    fn confirmed_group_plus_unmatched_known_and_unverifiable_members_are_candidates() {
        let group = [
            video("a", "hash1", 100, Some("full-x")),
            video("b", "hash1", 100, Some("full-x")),
            video("c", "hash1", 100, Some("full-y")),
            unverifiable("d"),
        ];
        assert_eq!(candidates(&group), ids(&["c", "d"]));
    }

    #[test]
    fn a_single_unverifiable_member_alone_is_hidden() {
        let group = [unverifiable("a")];
        assert_eq!(candidates(&group), None);
    }

    #[test]
    fn io_failure_none_is_never_a_candidate() {
        // Neither member is unverifiable: both are plain I/O failures.
        let group = [
            video("a", "hash1", 100, None),
            video("b", "hash1", 100, None),
        ];
        assert_eq!(candidates(&group), None);
        // An I/O-failure member does not pair with an unverifiable one.
        let group = [unverifiable("a"), video("b", "hash1", 100, None)];
        assert_eq!(candidates(&group), None);
        // ...and is left out when other members qualify.
        let group = [
            unverifiable("a"),
            unverifiable("b"),
            video("c", "hash1", 100, None),
        ];
        assert_eq!(candidates(&group), ids(&["a", "b"]));
    }

    #[test]
    fn fully_known_groups_have_no_unverified_candidates() {
        let group = [
            video("a", "hash1", 100, Some("full-x")),
            video("b", "hash1", 100, Some("full-y")),
        ];
        assert_eq!(candidates(&group), None);
    }

    #[test]
    fn empty_quick_hash_placeholders_never_become_candidates() {
        // Candidate groups come from group_candidates_by_quick_hash, which
        // drops empty-quick_hash rows, so they cannot reach this function.
        let videos = vec![
            VideoHashInfo {
                unverifiable: true,
                ..video("a", "", 100, None)
            },
            VideoHashInfo {
                unverifiable: true,
                ..video("b", "", 100, None)
            },
        ];
        assert!(group_candidates_by_quick_hash(&videos).is_empty());
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
