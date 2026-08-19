//! Rating value-range validation. 0 = unrated, 1-5 = star rating.
//! Pure validation only -- the DB write path lives in
//! `src-tauri::db::queries::set_rating`.

pub const MIN_RATING: u8 = 0;
pub const MAX_RATING: u8 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RatingOutOfRange {
    pub value: u8,
}

/// Validates a rating value is within `0..=5`. `0` means "unrated" (also
/// how a rating is cleared, per the UI's "clear rating" action) -- it is a
/// valid value, not an error.
pub fn validate_rating(value: u8) -> Result<u8, RatingOutOfRange> {
    if value <= MAX_RATING {
        Ok(value)
    } else {
        Err(RatingOutOfRange { value })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_is_a_valid_unrated_value() {
        assert_eq!(validate_rating(0), Ok(0));
    }

    #[test]
    fn one_through_five_are_valid() {
        for v in 1..=5u8 {
            assert_eq!(validate_rating(v), Ok(v));
        }
    }

    #[test]
    fn six_is_out_of_range() {
        assert_eq!(validate_rating(6), Err(RatingOutOfRange { value: 6 }));
    }

    #[test]
    fn the_max_u8_value_is_out_of_range() {
        assert_eq!(validate_rating(255), Err(RatingOutOfRange { value: 255 }));
    }
}
