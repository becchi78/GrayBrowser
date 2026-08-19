//! Incremental search query parsing. Actual substring matching
//! happens in SQL (`LIKE`) in `src-tauri::db::queries`; this module only
//! decides what terms a raw search-box string breaks into, and how to
//! escape a term for safe use inside a `LIKE` pattern.

/// Splits on Unicode whitespace (`str::split_whitespace` already covers the
/// full-width space U+3000, since it's Unicode `White_Space`), trims, and
/// drops empty terms. Every returned term must match (AND semantics) against
/// a candidate row: "action comedy" requires both substrings present, not
/// one literal two-word phrase.
pub fn parse_search_terms(raw: &str) -> Vec<String> {
    raw.split_whitespace().map(str::to_string).collect()
}

/// Escapes SQLite `LIKE` metacharacters (`%`, `_`, and the escape character
/// itself, `\`) so a user-typed `%` or `_` is matched literally rather than
/// interpreted as a wildcard. Callers must pair this with `ESCAPE '\'` in
/// the SQL and wrap the result in `%...%` themselves (this function only
/// escapes; it doesn't add the wildcard wrapping).
pub fn escape_like_pattern(term: &str) -> String {
    let mut escaped = String::with_capacity(term.len());
    for c in term.chars() {
        if matches!(c, '\\' | '%' | '_') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_yields_no_terms() {
        assert_eq!(parse_search_terms(""), Vec::<String>::new());
    }

    #[test]
    fn whitespace_only_yields_no_terms() {
        assert_eq!(parse_search_terms("   \u{3000}  "), Vec::<String>::new());
    }

    #[test]
    fn a_single_term_is_returned_trimmed() {
        assert_eq!(parse_search_terms("  action  "), vec!["action".to_string()]);
    }

    #[test]
    fn multiple_terms_split_on_regular_spaces() {
        assert_eq!(
            parse_search_terms("action comedy"),
            vec!["action".to_string(), "comedy".to_string()]
        );
    }

    #[test]
    fn multiple_terms_split_on_full_width_space() {
        assert_eq!(
            parse_search_terms("action\u{3000}comedy"),
            vec!["action".to_string(), "comedy".to_string()]
        );
    }

    #[test]
    fn mixed_half_and_full_width_spaces_and_repeats_collapse() {
        assert_eq!(
            parse_search_terms(" action  \u{3000} comedy \u{3000}\u{3000}drama"),
            vec![
                "action".to_string(),
                "comedy".to_string(),
                "drama".to_string()
            ]
        );
    }

    #[test]
    fn escape_leaves_plain_text_unchanged() {
        assert_eq!(escape_like_pattern("action"), "action");
    }

    #[test]
    fn escape_escapes_percent() {
        assert_eq!(escape_like_pattern("100%"), "100\\%");
    }

    #[test]
    fn escape_escapes_underscore() {
        assert_eq!(escape_like_pattern("a_b"), "a\\_b");
    }

    #[test]
    fn escape_escapes_a_literal_backslash() {
        assert_eq!(escape_like_pattern(r"a\b"), r"a\\b");
    }

    #[test]
    fn escape_handles_all_three_metacharacters_together() {
        assert_eq!(escape_like_pattern(r"100%_\done"), r"100\%\_\\done");
    }

    #[test]
    fn escape_preserves_japanese_text_unchanged() {
        assert_eq!(escape_like_pattern("アクション"), "アクション");
    }
}
