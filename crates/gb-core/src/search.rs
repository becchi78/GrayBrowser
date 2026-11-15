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
    fn parse_search_terms_splits_on_half_and_full_width_whitespace() {
        let cases: [(&str, &[&str]); 7] = [
            ("", &[]),
            ("   \u{3000}  ", &[]),
            // Trimmed.
            ("  action  ", &["action"]),
            ("action comedy", &["action", "comedy"]),
            ("action\u{3000}comedy", &["action", "comedy"]),
            // Mixed widths and repeats collapse.
            (
                " action  \u{3000} comedy \u{3000}\u{3000}drama",
                &["action", "comedy", "drama"],
            ),
            // Tabs and newlines are whitespace too.
            ("a\tb\nc", &["a", "b", "c"]),
        ];
        for (raw, expected) in cases {
            assert_eq!(parse_search_terms(raw), expected, "{raw:?}");
        }
    }

    #[test]
    fn escape_like_pattern_escapes_only_the_like_metacharacters() {
        let cases = [
            ("action", "action"),
            ("100%", r"100\%"),
            ("a_b", r"a\_b"),
            // The escape character itself.
            (r"a\b", r"a\\b"),
            (r"100%_\done", r"100\%\_\\done"),
            ("アクション", "アクション"),
            ("", ""),
        ];
        for (term, expected) in cases {
            assert_eq!(escape_like_pattern(term), expected, "{term:?}");
        }
    }
}
