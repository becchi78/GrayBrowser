//! Progress counting for background thumbnail generation.
//!
//! Several generation queues (startup, post-scan, `.wb` import) run
//! independently and can each list the same video as pending. Summing a
//! count per queue would count such a video twice and inflate the
//! denominator, so progress is derived from one shared set of pending video
//! ids instead: a video present in several queues is still one item.

use std::collections::HashSet;

/// How a tracked video's processing ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishOutcome {
    /// Thumbnails were generated just now.
    Generated,
    /// Another queue had already produced them, so nothing was done.
    AlreadyGenerated,
    /// Generation gave up. Counts as processed but not as done, so the
    /// batch can still complete and the failure is surfaced elsewhere.
    Failed,
    /// Another holder (a different queue or a manual retry) owns the video's
    /// claim, so this queue skipped it. The holder may not be tracked here at
    /// all, so waiting for its `finish` could leave the video pending
    /// forever; the skipper drops it instead. It is not counted as done
    /// because this queue did not produce it.
    SkippedBusy,
    /// The video's folder cannot be reached right now, so it was not
    /// attempted. Dropped from the pending set like `SkippedBusy` -- waiting
    /// on it would hold the batch open for as long as the outage lasts -- and
    /// not counted as done. A later run re-lists it once the folder is back.
    SkippedUnreachable,
}

impl FinishOutcome {
    fn is_success(self) -> bool {
        matches!(self, Self::Generated | Self::AlreadyGenerated)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProgressSnapshot {
    pub done: u32,
    pub total: u32,
}

#[derive(Debug, Default)]
pub struct ThumbnailProgress {
    /// Videos added but not yet finished.
    pending: HashSet<String>,
    done: u32,
    total: u32,
}

impl ThumbnailProgress {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts tracking the videos that are not tracked yet and grows the
    /// denominator by exactly that many.
    pub fn add<I, S>(&mut self, ids: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for id in ids {
            if self.pending.insert(id.into()) {
                self.total += 1;
            }
        }
    }

    /// Marks a tracked video as processed. Ids that are not tracked (already
    /// finished, or never added) are ignored so nothing is counted twice.
    /// When nothing remains tracked, the counters return to zero so the next
    /// batch is counted from scratch.
    pub fn finish(&mut self, id: &str, outcome: FinishOutcome) {
        if !self.pending.remove(id) {
            return;
        }
        if outcome.is_success() {
            self.done += 1;
        }
        if self.pending.is_empty() {
            self.done = 0;
            self.total = 0;
        }
    }

    pub fn snapshot(&self) -> ProgressSnapshot {
        ProgressSnapshot {
            done: self.done,
            total: self.total,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use FinishOutcome::*;

    fn snap(p: &ThumbnailProgress) -> (u32, u32) {
        let s = p.snapshot();
        (s.done, s.total)
    }

    #[test]
    fn starts_empty() {
        assert_eq!(snap(&ThumbnailProgress::new()), (0, 0));
    }

    #[test]
    fn duplicate_ids_count_once() {
        let mut p = ThumbnailProgress::new();
        p.add(["a", "b"]);
        p.add(["b", "c", "a"]);
        p.add(["c", "c"]);
        assert_eq!(snap(&p), (0, 3));
    }

    #[test]
    fn finishing_the_same_video_twice_counts_once() {
        let mut p = ThumbnailProgress::new();
        p.add(["a", "b"]);
        p.finish("a", Generated);
        p.finish("a", Generated);
        assert_eq!(snap(&p), (1, 2));
    }

    #[test]
    fn untracked_ids_are_ignored() {
        let mut p = ThumbnailProgress::new();
        p.finish("ghost", Generated);
        assert_eq!(snap(&p), (0, 0));
        p.add(["a", "b"]);
        p.finish("ghost", Generated);
        assert_eq!(snap(&p), (0, 2));
    }

    #[test]
    fn failure_is_processed_but_not_done() {
        let mut p = ThumbnailProgress::new();
        p.add(["a", "b"]);
        p.finish("a", Failed);
        assert_eq!(snap(&p), (0, 2));
        // A failed video is no longer pending, so finishing it again is a no-op.
        p.finish("a", Generated);
        assert_eq!(snap(&p), (0, 2));
    }

    #[test]
    fn resets_to_zero_when_everything_is_processed() {
        let mut p = ThumbnailProgress::new();
        p.add(["a", "b", "c"]);
        p.finish("a", Generated);
        p.finish("b", Failed);
        assert_eq!(snap(&p), (1, 3));
        p.finish("c", AlreadyGenerated);
        assert_eq!(snap(&p), (0, 0));
    }

    #[test]
    fn all_failed_still_resets() {
        let mut p = ThumbnailProgress::new();
        p.add(["a"]);
        p.finish("a", Failed);
        assert_eq!(snap(&p), (0, 0));
    }

    #[test]
    fn adding_after_a_reset_starts_a_new_batch() {
        let mut p = ThumbnailProgress::new();
        p.add(["a", "b"]);
        p.finish("a", Generated);
        p.finish("b", Generated);
        p.add(["c"]);
        assert_eq!(snap(&p), (0, 1));
        p.add(["a"]);
        assert_eq!(snap(&p), (0, 2));
    }

    #[test]
    fn adding_mid_batch_extends_the_denominator() {
        let mut p = ThumbnailProgress::new();
        p.add(["a", "b"]);
        p.finish("a", Generated);
        p.add(["b", "c"]);
        assert_eq!(snap(&p), (1, 3));
    }

    #[test]
    fn skipped_busy_is_processed_but_not_done() {
        let mut p = ThumbnailProgress::new();
        p.add(["a", "b"]);
        p.finish("a", SkippedBusy);
        assert_eq!(snap(&p), (0, 2));
        // The claim holder finishing later is ignored: already untracked.
        p.finish("a", Generated);
        assert_eq!(snap(&p), (0, 2));
    }

    #[test]
    fn skipped_busy_on_the_last_video_resets() {
        let mut p = ThumbnailProgress::new();
        p.add(["a", "b"]);
        p.finish("a", Generated);
        p.finish("b", SkippedBusy);
        assert_eq!(snap(&p), (0, 0));
    }

    #[test]
    fn skipped_busy_for_an_untracked_id_is_ignored() {
        let mut p = ThumbnailProgress::new();
        p.finish("ghost", SkippedBusy);
        assert_eq!(snap(&p), (0, 0));
        p.add(["a"]);
        p.finish("ghost", SkippedBusy);
        assert_eq!(snap(&p), (0, 1));
    }

    #[test]
    fn skipped_unreachable_lets_the_batch_complete_without_counting_as_done() {
        let mut p = ThumbnailProgress::new();
        p.add(["a", "b", "c"]);
        p.finish("a", Generated);
        p.finish("b", SkippedUnreachable);
        assert_eq!(snap(&p), (1, 3));
        p.finish("c", SkippedUnreachable);
        assert_eq!(snap(&p), (0, 0));
    }

    #[test]
    fn already_generated_counts_as_success() {
        let mut p = ThumbnailProgress::new();
        p.add(["a", "b"]);
        p.finish("a", AlreadyGenerated);
        assert_eq!(snap(&p), (1, 2));
    }
}
