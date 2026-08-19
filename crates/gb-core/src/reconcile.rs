//! Pure reconciliation decision logic: classifying a discovered file
//! against what's already known, deciding
//! whether a path-follow match should be applied, and deciding which known
//! online videos should transition offline after a scan/poll pass.
//!
//! No I/O, no DB, no filesystem access -- callers in `src-tauri` gather the
//! inputs (via `rusqlite` queries, `WalkDir`, `notify` events, etc.) and pass
//! plain data in here, per this crate's OS-independence rule.

use std::collections::HashSet;

pub struct DiscoveredFile {
    pub file_size: u64,
    pub mtime: i64,
}

pub struct KnownFileMeta {
    pub file_size: u64,
    pub mtime: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileClassification {
    Unchanged,
    NeedsRehash,
    NewCandidate,
}

/// Stage-1 cheap filter shared by the NAS 2-stage diff and general rescan
/// short-circuiting. `known.mtime` is `Option` because rows written before
/// mtime tracking existed have none --
/// treated the same as "changed" (must rehash), never silently as
/// "unchanged", so a missing mtime can never cause a real content change to
/// be skipped.
pub fn classify_discovered_file(
    discovered: &DiscoveredFile,
    known: Option<&KnownFileMeta>,
) -> FileClassification {
    match known {
        None => FileClassification::NewCandidate,
        Some(k) => match k.mtime {
            Some(known_mtime)
                if known_mtime == discovered.mtime && k.file_size == discovered.file_size =>
            {
                FileClassification::Unchanged
            }
            _ => FileClassification::NeedsRehash,
        },
    }
}

pub struct OfflineCandidate {
    pub video_id: String,
    pub file_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathFollowDecision {
    Reactivate {
        video_id: String,
    },
    BlockedByCollision {
        video_id: String,
        colliding_video_id: String,
    },
    NoMatch,
}

/// Matches a newly discovered file against `status='offline'` rows by
/// quick_hash+file_size (the query producing
/// `candidates` is the caller's job). `collision` is `Some(other_video_id)`
/// iff the discovered file's path is already claimed by a *different*,
/// currently-online row (also the caller's job to check). When both a
/// candidate match and a collision exist, the collision wins -- this
/// function must never recommend rewriting into an already-occupied path.
///
/// `candidates` should be pre-sorted by the caller (e.g. `created_at ASC`);
/// multiple offline rows sharing the same quick_hash+file_size resolve to
/// the first element deterministically. Disambiguating true duplicate
/// offline rows is confirmed-duplicate detection's job, not this
/// function's.
pub fn decide_path_follow(
    candidates: &[OfflineCandidate],
    collision: Option<String>,
) -> PathFollowDecision {
    match candidates.first() {
        None => PathFollowDecision::NoMatch,
        Some(candidate) => match collision {
            Some(colliding_video_id) => PathFollowDecision::BlockedByCollision {
                video_id: candidate.video_id.clone(),
                colliding_video_id,
            },
            None => PathFollowDecision::Reactivate {
                video_id: candidate.video_id.clone(),
            },
        },
    }
}

pub struct KnownOnlineVideo {
    pub video_id: String,
    pub file_path: String,
}

pub struct EnumerationResult {
    pub root_reachable: bool,
    pub inaccessible_dirs: Vec<String>,
    pub discovered_paths: Vec<String>,
}

/// A missing-video candidate is only suppressed (all-or-nothing, for the
/// whole cycle) when the enumeration itself looks broken:
/// - `NothingDiscovered`: the walk returned zero files *and* zero
///   inaccessible-directory errors, despite the root being reachable and at
///   least one known video existing under it. Nothing was actually seen,
///   good or bad -- more consistent with a broken listing than with every
///   tracked file having vanished simultaneously.
/// - `RatioExceeded`: `MISSING_RATIO_THRESHOLD` or more of the known online
///   videos in this folder look missing at once (only evaluated once the
///   folder has at least `MIN_KNOWN_COUNT_FOR_RATIO_GUARD` known videos, so
///   small folders aren't held up by ordinary single-file deletions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressReason {
    NothingDiscovered,
    RatioExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuppressedGuard {
    pub reason: SuppressReason,
    /// How many videos would have been marked offline had the guard not
    /// intervened -- for the caller's WARN log.
    pub candidate_count: usize,
    pub known_online_count: usize,
    pub discovered_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingVideoDecision {
    /// Video ids to transition to `offline` this cycle. Always empty when
    /// `suppressed` is `Some`.
    pub missing_ids: Vec<String>,
    pub suppressed: Option<SuppressedGuard>,
}

/// Below this many known-online videos in a folder, the ratio guard never
/// applies (only the `NothingDiscovered` floor can suppress) -- percentages
/// over a handful of files swing too wildly to carry statistical meaning,
/// and ordinary single/few-file deletions in a small folder shouldn't be
/// held up.
const MIN_KNOWN_COUNT_FOR_RATIO_GUARD: usize = 5;

/// Fraction (inclusive) of a folder's known-online videos that must look
/// missing at once to trip the ratio guard. Chosen so that a normal
/// large-scale *legitimate* cleanup (rarely more than a few tens of percent
/// of a folder in one ~10-minute default poll interval) proceeds normally,
/// while a listing that comes back mostly empty (partial enumeration
/// collapse, not full collapse -- the `NothingDiscovered` floor already
/// catches that) gets held instead of mass-flipping the folder offline.
const MISSING_RATIO_THRESHOLD: f64 = 0.8;

/// The full-enumeration missing-video guard, shared by both the NAS poller
/// (`src-tauri::watch::nas_poll`) and the manual/local folder scan
/// (`src-tauri::scan`) -- any caller that derives
/// `known_online`/`EnumerationResult` from a full `WalkDir`-style listing
/// carries the same "enumeration might have partially failed" risk, whether
/// the listing is over a network share or a local disk. A known online
/// video is only reported as missing if the scan's root *was* reachable at
/// all, AND its path wasn't merely missed because its parent directory
/// itself failed to enumerate (an inconclusive per-directory error,
/// distinct from "genuinely gone"), AND the enumeration doesn't otherwise
/// look broken (see `SuppressReason`). If the root itself was unreachable,
/// or the enumeration looks broken, nothing is reported missing -- a
/// transient blip must never mass-flip an entire folder's videos offline in
/// one scan/poll cycle. When in doubt, this function
/// always resolves toward *not* transitioning anything offline: a wrongly
/// withheld transition self-corrects (at worst) once enumeration recovers,
/// while a wrongly applied one mass-offlines a catalog in one cycle --
/// the two outcomes are not equally bad, so the bias is deliberate.
pub fn decide_missing_video_ids(
    known_online: &[KnownOnlineVideo],
    diff: &EnumerationResult,
) -> MissingVideoDecision {
    let none_missing = MissingVideoDecision {
        missing_ids: Vec::new(),
        suppressed: None,
    };

    if !diff.root_reachable {
        return none_missing;
    }

    let discovered: HashSet<&str> = diff.discovered_paths.iter().map(String::as_str).collect();
    let candidates: Vec<&KnownOnlineVideo> = known_online
        .iter()
        .filter(|v| !discovered.contains(v.file_path.as_str()))
        .filter(|v| {
            !diff
                .inaccessible_dirs
                .iter()
                .any(|dir| is_under(&v.file_path, dir))
        })
        .collect();

    if candidates.is_empty() {
        return none_missing;
    }

    let known_count = known_online.len();
    let discovered_count = diff.discovered_paths.len();
    let suppress = |reason| {
        Some(SuppressedGuard {
            reason,
            candidate_count: candidates.len(),
            known_online_count: known_count,
            discovered_count,
        })
    };

    if discovered_count == 0 && diff.inaccessible_dirs.is_empty() {
        return MissingVideoDecision {
            missing_ids: Vec::new(),
            suppressed: suppress(SuppressReason::NothingDiscovered),
        };
    }

    let missing_ratio = candidates.len() as f64 / known_count as f64;
    if known_count >= MIN_KNOWN_COUNT_FOR_RATIO_GUARD && missing_ratio >= MISSING_RATIO_THRESHOLD {
        return MissingVideoDecision {
            missing_ids: Vec::new(),
            suppressed: suppress(SuppressReason::RatioExceeded),
        };
    }

    MissingVideoDecision {
        missing_ids: candidates.into_iter().map(|v| v.video_id.clone()).collect(),
        suppressed: None,
    }
}

/// Case-insensitive prefix match (NTFS/Windows paths are case-insensitive).
fn is_under(file_path: &str, dir: &str) -> bool {
    file_path.to_lowercase().starts_with(&dir.to_lowercase())
}

/// The local realtime watcher's per-event removal decision. Unlike
/// `decide_missing_video_ids`, this has no broken-enumeration guard to
/// apply: a `notify` `Removed` event is a single OS-confirmed fact about
/// one path, not derived from a `WalkDir`-style listing that could have
/// partially failed, so there is no "enumeration looked broken" risk here
/// to guard against. Returns `None` (no write) if the row is already
/// `offline`, so a caller can call this unconditionally without needing to
/// track whether it already handled this row.
pub fn decide_removal_outcome(current_status: &str) -> Option<&'static str> {
    if current_status == "online" {
        Some("offline")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- classify_discovered_file ---

    #[test]
    fn classify_new_when_nothing_known() {
        let discovered = DiscoveredFile {
            file_size: 100,
            mtime: 1000,
        };
        assert_eq!(
            classify_discovered_file(&discovered, None),
            FileClassification::NewCandidate
        );
    }

    #[test]
    fn classify_unchanged_when_size_and_mtime_match() {
        let discovered = DiscoveredFile {
            file_size: 100,
            mtime: 1000,
        };
        let known = KnownFileMeta {
            file_size: 100,
            mtime: Some(1000),
        };
        assert_eq!(
            classify_discovered_file(&discovered, Some(&known)),
            FileClassification::Unchanged
        );
    }

    #[test]
    fn classify_needs_rehash_when_mtime_differs() {
        let discovered = DiscoveredFile {
            file_size: 100,
            mtime: 2000,
        };
        let known = KnownFileMeta {
            file_size: 100,
            mtime: Some(1000),
        };
        assert_eq!(
            classify_discovered_file(&discovered, Some(&known)),
            FileClassification::NeedsRehash
        );
    }

    #[test]
    fn classify_needs_rehash_when_size_differs_but_mtime_matches() {
        let discovered = DiscoveredFile {
            file_size: 200,
            mtime: 1000,
        };
        let known = KnownFileMeta {
            file_size: 100,
            mtime: Some(1000),
        };
        assert_eq!(
            classify_discovered_file(&discovered, Some(&known)),
            FileClassification::NeedsRehash
        );
    }

    #[test]
    fn classify_needs_rehash_when_known_mtime_is_missing() {
        // Rows written before mtime tracking existed must never be treated
        // as "unchanged" -- that would silently skip a real check.
        let discovered = DiscoveredFile {
            file_size: 100,
            mtime: 1000,
        };
        let known = KnownFileMeta {
            file_size: 100,
            mtime: None,
        };
        assert_eq!(
            classify_discovered_file(&discovered, Some(&known)),
            FileClassification::NeedsRehash
        );
    }

    // --- decide_path_follow ---

    #[test]
    fn path_follow_no_match_when_no_candidates() {
        assert_eq!(decide_path_follow(&[], None), PathFollowDecision::NoMatch);
    }

    #[test]
    fn path_follow_reactivates_single_candidate_without_collision() {
        let candidates = [OfflineCandidate {
            video_id: "v1".to_string(),
            file_path: "D:\\old\\a.mp4".to_string(),
        }];
        assert_eq!(
            decide_path_follow(&candidates, None),
            PathFollowDecision::Reactivate {
                video_id: "v1".to_string()
            }
        );
    }

    #[test]
    fn path_follow_blocked_when_target_path_collides_with_online_row() {
        let candidates = [OfflineCandidate {
            video_id: "v1".to_string(),
            file_path: "D:\\old\\a.mp4".to_string(),
        }];
        assert_eq!(
            decide_path_follow(&candidates, Some("v2".to_string())),
            PathFollowDecision::BlockedByCollision {
                video_id: "v1".to_string(),
                colliding_video_id: "v2".to_string(),
            }
        );
    }

    #[test]
    fn path_follow_picks_first_candidate_deterministically_when_multiple_match() {
        let candidates = [
            OfflineCandidate {
                video_id: "earliest".to_string(),
                file_path: "D:\\old\\a.mp4".to_string(),
            },
            OfflineCandidate {
                video_id: "later".to_string(),
                file_path: "D:\\old\\b.mp4".to_string(),
            },
        ];
        assert_eq!(
            decide_path_follow(&candidates, None),
            PathFollowDecision::Reactivate {
                video_id: "earliest".to_string()
            }
        );
    }

    // --- decide_missing_video_ids ---

    fn known_videos(count: usize) -> Vec<KnownOnlineVideo> {
        (0..count)
            .map(|i| KnownOnlineVideo {
                video_id: format!("v{i}"),
                file_path: format!("\\\\nas\\share\\video{i}.mp4"),
            })
            .collect()
    }

    #[test]
    fn missing_reports_nothing_when_root_unreachable() {
        let known = [KnownOnlineVideo {
            video_id: "v1".to_string(),
            file_path: "\\\\nas\\share\\a.mp4".to_string(),
        }];
        let diff = EnumerationResult {
            root_reachable: false,
            inaccessible_dirs: vec![],
            discovered_paths: vec![],
        };
        let decision = decide_missing_video_ids(&known, &diff);
        assert!(decision.missing_ids.is_empty());
        assert!(decision.suppressed.is_none());
    }

    #[test]
    fn missing_reports_video_absent_from_a_fully_reachable_listing() {
        let known = [KnownOnlineVideo {
            video_id: "v1".to_string(),
            file_path: "\\\\nas\\share\\a.mp4".to_string(),
        }];
        let diff = EnumerationResult {
            root_reachable: true,
            inaccessible_dirs: vec![],
            discovered_paths: vec!["\\\\nas\\share\\b.mp4".to_string()],
        };
        let decision = decide_missing_video_ids(&known, &diff);
        assert_eq!(decision.missing_ids, vec!["v1"]);
        assert!(decision.suppressed.is_none());
    }

    #[test]
    fn missing_does_not_report_video_still_present() {
        let known = [KnownOnlineVideo {
            video_id: "v1".to_string(),
            file_path: "\\\\nas\\share\\a.mp4".to_string(),
        }];
        let diff = EnumerationResult {
            root_reachable: true,
            inaccessible_dirs: vec![],
            discovered_paths: vec!["\\\\nas\\share\\a.mp4".to_string()],
        };
        let decision = decide_missing_video_ids(&known, &diff);
        assert!(decision.missing_ids.is_empty());
        assert!(decision.suppressed.is_none());
    }

    #[test]
    fn missing_stays_inconclusive_for_a_video_under_an_inaccessible_subdir() {
        let known = [KnownOnlineVideo {
            video_id: "v1".to_string(),
            file_path: "\\\\nas\\share\\locked\\a.mp4".to_string(),
        }];
        let diff = EnumerationResult {
            root_reachable: true,
            inaccessible_dirs: vec!["\\\\nas\\share\\locked\\".to_string()],
            discovered_paths: vec![],
        };
        let decision = decide_missing_video_ids(&known, &diff);
        assert!(decision.missing_ids.is_empty());
        assert!(
            decision.suppressed.is_none(),
            "excluded via inaccessible_dirs, not the broken-enumeration guard"
        );
    }

    // --- decide_missing_video_ids: broken-enumeration guard ---

    #[test]
    fn suppresses_when_nothing_at_all_was_discovered() {
        let known = known_videos(3);
        let diff = EnumerationResult {
            root_reachable: true,
            inaccessible_dirs: vec![],
            discovered_paths: vec![],
        };
        let decision = decide_missing_video_ids(&known, &diff);
        assert!(decision.missing_ids.is_empty());
        assert_eq!(
            decision.suppressed,
            Some(SuppressedGuard {
                reason: SuppressReason::NothingDiscovered,
                candidate_count: 3,
                known_online_count: 3,
                discovered_count: 0,
            })
        );
    }

    /// known=100, discovered=19 -> 81 missing = 81% >= 80% threshold ->
    /// suppressed.
    #[test]
    fn suppresses_just_past_the_ratio_boundary() {
        let known = known_videos(100);
        let discovered_paths: Vec<String> = (0..19)
            .map(|i| format!("\\\\nas\\share\\video{i}.mp4"))
            .collect();
        let diff = EnumerationResult {
            root_reachable: true,
            inaccessible_dirs: vec![],
            discovered_paths,
        };
        let decision = decide_missing_video_ids(&known, &diff);
        assert!(decision.missing_ids.is_empty());
        assert_eq!(
            decision.suppressed,
            Some(SuppressedGuard {
                reason: SuppressReason::RatioExceeded,
                candidate_count: 81,
                known_online_count: 100,
                discovered_count: 19,
            })
        );
    }

    /// known=100, discovered=20 -> 80 missing = exactly 80% -- the `>=`
    /// boundary is inclusive, so this is still suppressed.
    #[test]
    fn suppresses_exactly_at_the_ratio_boundary_inclusive() {
        let known = known_videos(100);
        let discovered_paths: Vec<String> = (0..20)
            .map(|i| format!("\\\\nas\\share\\video{i}.mp4"))
            .collect();
        let diff = EnumerationResult {
            root_reachable: true,
            inaccessible_dirs: vec![],
            discovered_paths,
        };
        let decision = decide_missing_video_ids(&known, &diff);
        assert!(decision.missing_ids.is_empty());
        assert_eq!(
            decision.suppressed.map(|g| g.reason),
            Some(SuppressReason::RatioExceeded)
        );
    }

    /// known=100, discovered=21 -> 79 missing = 79% < 80% threshold ->
    /// proceeds normally, all 79 go missing.
    #[test]
    fn proceeds_normally_just_below_the_ratio_boundary() {
        let known = known_videos(100);
        let discovered_paths: Vec<String> = (0..21)
            .map(|i| format!("\\\\nas\\share\\video{i}.mp4"))
            .collect();
        let diff = EnumerationResult {
            root_reachable: true,
            inaccessible_dirs: vec![],
            discovered_paths,
        };
        let decision = decide_missing_video_ids(&known, &diff);
        assert_eq!(decision.missing_ids.len(), 79);
        assert!(decision.suppressed.is_none());
    }

    /// A moderate, plausibly-legitimate loss (30% missing, 70% discovered)
    /// must never be suppressed.
    #[test]
    fn does_not_suppress_a_moderate_legitimate_looking_loss() {
        let known = known_videos(100);
        let discovered_paths: Vec<String> = (0..70)
            .map(|i| format!("\\\\nas\\share\\video{i}.mp4"))
            .collect();
        let diff = EnumerationResult {
            root_reachable: true,
            inaccessible_dirs: vec![],
            discovered_paths,
        };
        let decision = decide_missing_video_ids(&known, &diff);
        assert_eq!(
            decision.missing_ids.len(),
            30,
            "30 of 100 (video70..video99) should be missing"
        );
        assert!(decision.suppressed.is_none());
    }

    /// known=3 (below MIN_KNOWN_COUNT_FOR_RATIO_GUARD=5), discovered=1 ->
    /// 2 missing = 67% would exceed the ratio threshold, but the folder is
    /// too small for the ratio guard to apply at all -- proceeds normally.
    #[test]
    fn ratio_guard_does_not_apply_below_the_minimum_known_count() {
        let known = known_videos(3);
        let diff = EnumerationResult {
            root_reachable: true,
            inaccessible_dirs: vec![],
            discovered_paths: vec!["\\\\nas\\share\\video0.mp4".to_string()],
        };
        let decision = decide_missing_video_ids(&known, &diff);
        assert_eq!(decision.missing_ids.len(), 2);
        assert!(decision.suppressed.is_none());
    }

    /// The inaccessible_dirs exclusion and the ratio guard compose
    /// correctly: excluded candidates don't count toward the ratio at all,
    /// and the remainder proceeds normally when under threshold.
    #[test]
    fn ratio_guard_composes_with_inaccessible_dirs_exclusion() {
        let mut known = known_videos(10);
        // Move video0 under a directory that's reported inaccessible this
        // cycle -- it must be excluded from consideration entirely, not
        // counted as "missing" for the ratio calculation.
        known[0].file_path = "\\\\nas\\share\\locked\\video0.mp4".to_string();
        let discovered_paths: Vec<String> = (1..9)
            .map(|i| format!("\\\\nas\\share\\video{i}.mp4"))
            .collect(); // video9 genuinely missing, video0 excluded
        let diff = EnumerationResult {
            root_reachable: true,
            inaccessible_dirs: vec!["\\\\nas\\share\\locked\\".to_string()],
            discovered_paths,
        };
        let decision = decide_missing_video_ids(&known, &diff);
        assert_eq!(decision.missing_ids, vec!["v9"]);
        assert!(decision.suppressed.is_none());
    }

    // --- decide_removal_outcome ---

    #[test]
    fn removal_of_an_online_video_transitions_to_offline() {
        assert_eq!(decide_removal_outcome("online"), Some("offline"));
    }

    #[test]
    fn removal_of_an_already_offline_video_is_a_no_op() {
        assert_eq!(decide_removal_outcome("offline"), None);
    }
}
