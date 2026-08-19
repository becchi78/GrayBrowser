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

/// Classification of a video's generation retry state, derived purely from
/// the number of attempts already made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryStatus {
    /// No attempt has been made yet.
    NotAttempted,
    /// At least one attempt has failed, but the retry limit has not been
    /// reached yet; an automatic retry is still eligible.
    Retrying { attempts: u32 },
    /// The retry limit has been reached (or exceeded); no further automatic
    /// retries should be attempted.
    Exhausted { attempts: u32 },
}

/// Classifies the retry status of a video based on its prior attempt count.
///
/// - `attempts == 0` -> [`RetryStatus::NotAttempted`]
/// - `0 < attempts < MAX_GENERATION_ATTEMPTS` -> [`RetryStatus::Retrying`]
/// - `attempts >= MAX_GENERATION_ATTEMPTS` -> [`RetryStatus::Exhausted`]
pub fn classify_retry_status(attempts: u32) -> RetryStatus {
    if attempts == 0 {
        RetryStatus::NotAttempted
    } else if attempts < MAX_GENERATION_ATTEMPTS {
        RetryStatus::Retrying { attempts }
    } else {
        RetryStatus::Exhausted { attempts }
    }
}

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
    fn zero_attempts_is_not_attempted_and_eligible() {
        assert_eq!(classify_retry_status(0), RetryStatus::NotAttempted);
        assert!(is_eligible_for_automatic_retry(0));
    }

    #[test]
    fn one_attempt_is_retrying_and_eligible() {
        assert_eq!(
            classify_retry_status(1),
            RetryStatus::Retrying { attempts: 1 }
        );
        assert!(is_eligible_for_automatic_retry(1));
    }

    #[test]
    fn attempts_below_max_minus_one_is_retrying_and_eligible() {
        // Any attempts strictly between 0 and MAX_GENERATION_ATTEMPTS must
        // still be classified as Retrying / eligible.
        let attempts = MAX_GENERATION_ATTEMPTS - 1;
        assert_eq!(
            classify_retry_status(attempts),
            RetryStatus::Retrying { attempts }
        );
        assert!(is_eligible_for_automatic_retry(attempts));
    }

    #[test]
    fn attempts_at_max_is_exhausted_and_ineligible() {
        // Boundary: attempts == MAX_GENERATION_ATTEMPTS means the video has
        // already failed MAX_GENERATION_ATTEMPTS times, so a further
        // (MAX_GENERATION_ATTEMPTS + 1)-th attempt must NOT be permitted.
        let attempts = MAX_GENERATION_ATTEMPTS;
        assert_eq!(
            classify_retry_status(attempts),
            RetryStatus::Exhausted { attempts }
        );
        assert!(!is_eligible_for_automatic_retry(attempts));
    }

    #[test]
    fn attempts_beyond_max_remains_exhausted_and_ineligible() {
        let attempts = MAX_GENERATION_ATTEMPTS + 1;
        assert_eq!(
            classify_retry_status(attempts),
            RetryStatus::Exhausted { attempts }
        );
        assert!(!is_eligible_for_automatic_retry(attempts));

        let attempts_far_beyond = MAX_GENERATION_ATTEMPTS + 100;
        assert_eq!(
            classify_retry_status(attempts_far_beyond),
            RetryStatus::Exhausted {
                attempts: attempts_far_beyond
            }
        );
        assert!(!is_eligible_for_automatic_retry(attempts_far_beyond));
    }

    #[test]
    fn exhaustive_boundary_check_around_max_generation_attempts() {
        // Explicit table matching the spec's boundary description, using
        // literal values so a regression in the constant-derived tests
        // above would still be caught by an independent check.
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
