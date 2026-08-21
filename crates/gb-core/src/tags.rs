//! Tag name normalization. Pure string processing, no DB access -- the
//! DB-side uniqueness/assignment logic lives in `src-tauri::db::queries`.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagNameError {
    /// The name is empty, or becomes empty after trimming/width-folding
    /// (e.g. whitespace-only, including full-width spaces).
    Empty,
}

impl fmt::Display for TagNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TagNameError::Empty => write!(f, "tag name must not be empty or whitespace-only"),
        }
    }
}

impl std::error::Error for TagNameError {}

/// Folds full-width (Zenkaku) ASCII-range characters (U+FF01-U+FF5E) and the
/// full-width space (U+3000, IDEOGRAPHIC SPACE) to their half-width
/// equivalents. Japanese IMEs commonly leave full-width mode on, so
/// "Ａｃｔｉｏｎ　movie" (full-width letters + full-width space) folds to
/// "Action movie" -- the same tag typed with full-width vs half-width input
/// should still be treated as the same tag. Characters outside these two
/// ranges (hiragana, katakana, kanji, half-width text) pass through
/// unchanged.
fn fold_width(input: &str) -> String {
    input
        .chars()
        .map(|c| match c {
            '\u{3000}' => ' ',
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            other => other,
        })
        .collect()
}

/// Normalizes a raw, user-typed tag name: width-folds, trims leading/
/// trailing whitespace, and rejects empty/whitespace-only input. Idempotent
/// -- normalizing an already-normalized name returns it unchanged.
///
/// Deliberately does *not* case-fold (e.g. "Action" and "action" remain
/// distinct tags) -- an accepted simplification, not a permanent design
/// decision.
pub fn normalize_tag_name(raw: &str) -> Result<String, TagNameError> {
    let normalized = fold_width(raw).trim().to_string();
    if normalized.is_empty() {
        return Err(TagNameError::Empty);
    }
    Ok(normalized)
}

/// Filters a persisted tag-bar "pinned" list down to the ids that still
/// exist, preserving order. Self-healing for the tag bar's persisted pin
/// list: a tag can be deleted (`queries::delete_tag`) independently of the
/// tag bar, so a previously-pinned id can go stale. Reading
/// `existing_tag_ids` from the DB is `src-tauri::db::queries`'s job -- this
/// stays a pure filter so it can be unit-tested without any DB access.
pub fn prune_missing_tag_ids(
    pinned: Vec<i64>,
    existing_tag_ids: &std::collections::HashSet<i64>,
) -> Vec<i64> {
    pinned
        .into_iter()
        .filter(|id| existing_tag_ids.contains(id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_ascii_whitespace() {
        assert_eq!(normalize_tag_name("  action  "), Ok("action".to_string()));
    }

    #[test]
    fn trims_full_width_space() {
        assert_eq!(
            normalize_tag_name("\u{3000}action\u{3000}"),
            Ok("action".to_string())
        );
    }

    #[test]
    fn rejects_an_empty_string() {
        assert_eq!(normalize_tag_name(""), Err(TagNameError::Empty));
    }

    #[test]
    fn rejects_whitespace_only_ascii() {
        assert_eq!(normalize_tag_name("   "), Err(TagNameError::Empty));
    }

    #[test]
    fn rejects_whitespace_only_full_width() {
        assert_eq!(
            normalize_tag_name("\u{3000}\u{3000}"),
            Err(TagNameError::Empty)
        );
    }

    #[test]
    fn folds_full_width_alphanumerics_to_half_width() {
        // Full-width "A", "c", "t", "I", "O", "N" (U+FF21/FF43/FF54/FF29/FF2F/FF2E).
        assert_eq!(
            normalize_tag_name("\u{FF21}\u{FF43}\u{FF54}\u{FF29}\u{FF2F}\u{FF2E}"),
            Ok("ActION".to_string())
        );
    }

    #[test]
    fn folds_full_width_punctuation() {
        // full-width "!" (U+FF01) -> half-width "!"
        assert_eq!(
            normalize_tag_name("action\u{FF01}"),
            Ok("action!".to_string())
        );
    }

    #[test]
    fn folds_internal_full_width_space_to_a_regular_space() {
        assert_eq!(
            normalize_tag_name("action\u{3000}movie"),
            Ok("action movie".to_string())
        );
    }

    #[test]
    fn preserves_japanese_text_unchanged() {
        assert_eq!(
            normalize_tag_name("アクション"),
            Ok("アクション".to_string())
        );
        assert_eq!(
            normalize_tag_name("コメディ映画"),
            Ok("コメディ映画".to_string())
        );
    }

    #[test]
    fn preserves_already_half_width_text_unchanged() {
        assert_eq!(
            normalize_tag_name("action-movie_2"),
            Ok("action-movie_2".to_string())
        );
    }

    #[test]
    fn is_idempotent() {
        let inputs = [
            "  action  ",
            "\u{FF21}\u{FF43}\u{FF54}",
            "アクション",
            "a b\u{3000}c",
        ];
        for input in inputs {
            let once = normalize_tag_name(input).unwrap();
            let twice = normalize_tag_name(&once).unwrap();
            assert_eq!(once, twice, "normalize should be idempotent for {input:?}");
        }
    }

    #[test]
    fn distinguishes_names_that_differ_only_by_case() {
        // Documents the accepted simplification: no case-folding.
        assert_ne!(
            normalize_tag_name("Action").unwrap(),
            normalize_tag_name("action").unwrap()
        );
    }

    // --- prune_missing_tag_ids ------------------------------------------

    #[test]
    fn prune_missing_tag_ids_keeps_order_of_surviving_ids() {
        let existing = std::collections::HashSet::from([1, 3, 5]);
        assert_eq!(
            prune_missing_tag_ids(vec![5, 1, 3], &existing),
            vec![5, 1, 3]
        );
    }

    #[test]
    fn prune_missing_tag_ids_removes_only_the_stale_ids() {
        let existing = std::collections::HashSet::from([1, 3]);
        assert_eq!(
            prune_missing_tag_ids(vec![1, 2, 3, 4], &existing),
            vec![1, 3]
        );
    }

    #[test]
    fn prune_missing_tag_ids_removes_everything_when_none_exist() {
        let existing = std::collections::HashSet::new();
        assert_eq!(
            prune_missing_tag_ids(vec![1, 2, 3], &existing),
            Vec::<i64>::new()
        );
    }

    #[test]
    fn prune_missing_tag_ids_keeps_everything_when_all_exist() {
        let existing = std::collections::HashSet::from([1, 2, 3]);
        assert_eq!(
            prune_missing_tag_ids(vec![1, 2, 3], &existing),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn prune_missing_tag_ids_on_empty_input_returns_empty() {
        let existing = std::collections::HashSet::from([1, 2]);
        assert_eq!(
            prune_missing_tag_ids(Vec::new(), &existing),
            Vec::<i64>::new()
        );
    }
}
