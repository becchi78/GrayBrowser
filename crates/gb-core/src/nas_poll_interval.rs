//! Range rules for the user-facing NAS polling interval. The UI works in whole
//! minutes while storage keeps seconds, so conversion and clamping live here.

pub const MIN_MINUTES: i64 = 1;
pub const MAX_MINUTES: i64 = 1440;
pub const DEFAULT_MINUTES: i64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IntervalError {
    #[error("interval must be between {MIN_MINUTES} and {MAX_MINUTES} minutes: {0}")]
    OutOfRange(i64),
}

/// Converts minutes to seconds, rejecting values outside the allowed range.
pub fn minutes_to_secs(minutes: i64) -> Result<i64, IntervalError> {
    if (MIN_MINUTES..=MAX_MINUTES).contains(&minutes) {
        Ok(minutes * 60)
    } else {
        Err(IntervalError::OutOfRange(minutes))
    }
}

/// Converts stored seconds to minutes for display: rounded up so a sub-minute
/// value never shows as 0, and clamped into the allowed range so a hand-edited
/// extreme value still yields something the dialog can save back.
pub fn secs_to_display_minutes(secs: i64) -> u32 {
    let minutes = secs.saturating_add(59).div_euclid(60);
    minutes.clamp(MIN_MINUTES, MAX_MINUTES) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minutes_to_secs_accepts_the_range_boundaries() {
        assert_eq!(minutes_to_secs(1), Ok(60));
        assert_eq!(minutes_to_secs(10), Ok(600));
        assert_eq!(minutes_to_secs(1440), Ok(86_400));
    }

    #[test]
    fn minutes_to_secs_rejects_out_of_range() {
        for bad in [0, 1441, -1, i64::MIN, i64::MAX] {
            assert_eq!(minutes_to_secs(bad), Err(IntervalError::OutOfRange(bad)));
        }
    }

    #[test]
    fn secs_to_display_minutes_rounds_up() {
        assert_eq!(secs_to_display_minutes(90), 2);
        assert_eq!(secs_to_display_minutes(60), 1);
        assert_eq!(secs_to_display_minutes(61), 2);
        assert_eq!(secs_to_display_minutes(600), 10);
    }

    #[test]
    fn secs_to_display_minutes_clamps_into_range() {
        assert_eq!(secs_to_display_minutes(0), 1);
        assert_eq!(secs_to_display_minutes(-500), 1);
        assert_eq!(secs_to_display_minutes(i64::MIN), 1);
        assert_eq!(secs_to_display_minutes(86_400), 1440);
        assert_eq!(secs_to_display_minutes(1_000_000), 1440);
        assert_eq!(secs_to_display_minutes(i64::MAX), 1440);
    }

    #[test]
    fn default_is_within_range() {
        assert!((MIN_MINUTES..=MAX_MINUTES).contains(&DEFAULT_MINUTES));
    }
}
