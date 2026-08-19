//! Thumbnail seek-position policy: extract 6 frames from evenly spaced
//! points across the video's duration (the video split into 7 equal
//! segments, using the 6 interior boundary points), falling back to a
//! fixed 1s point for all 6 slots when the duration is unknown.
//!
//! Unlike the single-thumbnail policy this replaces, no short-video special
//! case is needed here: each of the 6 fractions (1/7 .. 6/7) is always
//! strictly less than `duration` whenever `duration > 0`, so every computed
//! position is a valid seek target within the video.

/// Fallback seek point used for every one of the 6 slots when duration is
/// unknown, zero, non-finite, or negative.
const UNKNOWN_DURATION_FALLBACK_SECS: f64 = 1.0;

/// The 6 interior boundary points of the video split into 7 equal segments
/// (1/7, 2/7, .., 6/7), used to pick 6 evenly spaced thumbnail seek
/// positions.
const FRACTIONS: [f64; 6] = [
    1.0 / 7.0,
    2.0 / 7.0,
    3.0 / 7.0,
    4.0 / 7.0,
    5.0 / 7.0,
    6.0 / 7.0,
];

/// Returns the 6 initial seek offsets (in seconds) to extract thumbnail
/// frames from, given the video duration if known.
///
/// This only picks the *initial* attempt for each slot. If the actual
/// `ffmpeg` seek at a given offset fails at runtime, retrying that slot at 0s
/// is the thumbnail worker's responsibility (`fallback_seek_seconds`), not
/// this pure function's -- it has no way to know that in advance when
/// `duration_secs` is `None` or unreliable.
pub fn thumbnail_seek_positions(duration_secs: Option<f64>) -> [f64; 6] {
    match duration_secs.filter(|d| d.is_finite() && *d > 0.0) {
        Some(duration) => FRACTIONS.map(|f| duration * f),
        None => [UNKNOWN_DURATION_FALLBACK_SECS; 6],
    }
}

/// Given that extraction failed at `failed_seek_secs`, returns the retry
/// position (0s), or `None` if we were already at 0s (no further fallback --
/// prevents an infinite retry loop). This is the pure "what should we retry
/// at" decision; actually re-invoking ffmpeg is the thumbnail worker's job,
/// not this function's.
pub fn fallback_seek_seconds(failed_seek_secs: f64) -> Option<f64> {
    if failed_seek_secs > 0.0 {
        Some(0.0)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_from_a_positive_seek_retries_at_zero() {
        assert_eq!(fallback_seek_seconds(12.0), Some(0.0));
        assert_eq!(fallback_seek_seconds(1.0), Some(0.0));
    }

    #[test]
    fn fallback_from_zero_has_no_further_retry() {
        assert_eq!(fallback_seek_seconds(0.0), None);
    }

    #[test]
    fn normal_length_video_uses_seven_equal_segments() {
        let positions = thumbnail_seek_positions(Some(140.0));
        assert_eq!(positions, [20.0, 40.0, 60.0, 80.0, 100.0, 120.0]);
    }

    #[test]
    fn positions_are_evenly_spaced() {
        let positions = thumbnail_seek_positions(Some(700.0));
        for pair in positions.windows(2) {
            assert!((pair[1] - pair[0] - 100.0).abs() < 1e-9);
        }
    }

    #[test]
    fn every_position_is_strictly_within_the_duration() {
        // The whole point of using 1/7..6/7 rather than 10%/20%/.. spaced
        // from an edge is that even a 1-second video has valid (if
        // sub-millisecond) seek targets, with no need for a short-video
        // special case.
        for duration in [1.0, 0.1, 0.001, 3600.0] {
            let positions = thumbnail_seek_positions(Some(duration));
            for p in positions {
                assert!(
                    p > 0.0,
                    "position {p} should be > 0 for duration {duration}"
                );
                assert!(p < duration, "position {p} should be < duration {duration}");
            }
        }
    }

    #[test]
    fn zero_duration_is_treated_like_unknown() {
        assert_eq!(thumbnail_seek_positions(Some(0.0)), [1.0; 6]);
    }

    #[test]
    fn nan_duration_is_treated_like_unknown() {
        assert_eq!(thumbnail_seek_positions(Some(f64::NAN)), [1.0; 6]);
    }

    #[test]
    fn negative_duration_is_treated_like_unknown() {
        assert_eq!(thumbnail_seek_positions(Some(-5.0)), [1.0; 6]);
    }

    #[test]
    fn unknown_duration_uses_fixed_one_second_for_every_slot() {
        assert_eq!(thumbnail_seek_positions(None), [1.0; 6]);
    }
}
