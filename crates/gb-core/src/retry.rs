//! Retry-eligibility classification shared by generation pipelines
//! (thumbnail generation, metadata probing).
//!
//! This module is intentionally narrow: it only answers "given how many
//! times a generation attempt has already failed for this video, should an
//! automatic retry still be attempted?". Whether a given attempt succeeded
//! (e.g. thumbnail file exists, `probed_at` is set) is determined by each
//! worker independently and is out of scope here.

/// Maximum number of automatic generation attempts before a video is
/// considered permanently exhausted and excluded from automatic retries.
pub const MAX_GENERATION_ATTEMPTS: u32 = 3;

/// Returns whether a video with the given number of prior failed attempts
/// is still eligible for an automatic retry.
///
/// Equivalent to `attempts < MAX_GENERATION_ATTEMPTS`.
pub fn is_eligible_for_automatic_retry(attempts: u32) -> bool {
    attempts < MAX_GENERATION_ATTEMPTS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligibility_boundaries_around_max_generation_attempts() {
        // Literal boundary values (0 / 1 / MAX-1 / MAX / MAX+1 / far beyond):
        // attempts == MAX means the video has already failed MAX times, so a
        // further attempt must NOT be permitted.
        assert!(is_eligible_for_automatic_retry(0));
        assert!(is_eligible_for_automatic_retry(1));
        assert!(is_eligible_for_automatic_retry(MAX_GENERATION_ATTEMPTS - 1));
        assert!(!is_eligible_for_automatic_retry(MAX_GENERATION_ATTEMPTS));
        assert!(!is_eligible_for_automatic_retry(
            MAX_GENERATION_ATTEMPTS + 1
        ));
        assert!(!is_eligible_for_automatic_retry(
            MAX_GENERATION_ATTEMPTS + 100
        ));
        assert!(!is_eligible_for_automatic_retry(u32::MAX));
    }

    #[test]
    fn exhaustive_boundary_check_around_max_generation_attempts() {
        for attempts in 0..MAX_GENERATION_ATTEMPTS {
            assert!(
                is_eligible_for_automatic_retry(attempts),
                "attempts={attempts} should still be eligible"
            );
        }
        for attempts in MAX_GENERATION_ATTEMPTS..(MAX_GENERATION_ATTEMPTS + 5) {
            assert!(
                !is_eligible_for_automatic_retry(attempts),
                "attempts={attempts} should be exhausted"
            );
        }
    }
}
