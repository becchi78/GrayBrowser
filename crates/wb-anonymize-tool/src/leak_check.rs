//! Coverage/leak check for the anonymizer. Works
//! entirely in memory against `ReadOnlyWb::read_all_text_cells` output -- no second
//! `.wb` file is written just for this check. Never logs or returns any
//! real or dummy *values* -- only counts, so callers (the CLI's stderr
//! summary, and the integration test) can report on this safely.
//!
//! The negative check is **per-cell**: each real cell's own value is turned
//! into tokens, and those tokens are checked only against *that same cell's*
//! anonymized output -- not against every other cell's output too. This is
//! deliberate, not a shortcut: `gb_core::wb_anonymize::anonymize_cell` is a
//! pure function of `(role, value)` alone, so there is no mechanism by which
//! cell A's real content could end up inside cell B's dummy output -- the
//! only way for a real token to survive would be inside its *own* cell's
//! transform. Checking all-cells-vs-all-cells was tried first and produced
//! ~1.8% false-positive "violations" purely from short (>=3 char) real
//! tokens that happen to consist only of `0-9a-f` characters coincidentally
//! matching substrings of the hex-suffixed dummy tokens elsewhere in a
//! 20000+-token vocabulary -- noise, not a real leak, given the
//! per-cell-pure-function anonymization model.

use std::collections::HashSet;
use std::fmt;

use gb_core::wb_anonymize::{self, Anonymizer};

use crate::reader::WbTextCell;

/// Real data's `tag` values are documented as at least 5
/// bytes; this stays comfortably below that so genuine short tags aren't
/// excluded, while still dropping single-character noise like drive letters
/// that would otherwise flag as false-positive "leaks" of a purely
/// structural token. Conservative on purpose, accepting the false-negative
/// tradeoff this implies.
const MIN_TOKEN_CHARS: usize = 3;

pub struct TokenStats {
    /// Total candidate tokens considered across all cells (sum per-cell, not
    /// deduplicated globally -- this is a diagnostic count, not a set size).
    pub candidate_count: usize,
    pub excluded_short: usize,
    pub kept_min_len: Option<usize>,
    pub kept_max_len: Option<usize>,
}

pub struct LeakCheckReport {
    pub token_stats: TokenStats,
    pub cells_checked: usize,
    pub negative_violations: usize,
    pub positive_violations: usize,
}

impl LeakCheckReport {
    pub fn passed(&self) -> bool {
        self.negative_violations == 0 && self.positive_violations == 0
    }
}

/// Counts only -- never a real or dummy value -- so it is safe to print.
impl fmt::Display for LeakCheckReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cells_checked={} token_candidate_count={} token_excluded_short={} \
             token_kept_min_len={:?} token_kept_max_len={:?} negative_violations={} positive_violations={}",
            self.cells_checked,
            self.token_stats.candidate_count,
            self.token_stats.excluded_short,
            self.token_stats.kept_min_len,
            self.token_stats.kept_max_len,
            self.negative_violations,
            self.positive_violations,
        )
    }
}

fn candidate_tokens(value: &str) -> impl Iterator<Item = String> + '_ {
    std::iter::once(value.to_string())
        .chain(value.split(['\\', '/']).map(str::to_string))
        .chain(value.split('\n').map(str::to_string))
}

/// Tokens derived from a single real value, filtered to `MIN_TOKEN_CHARS`+.
fn forbidden_tokens_for(value: &str) -> HashSet<String> {
    candidate_tokens(value)
        .filter(|t| !t.is_empty() && t.chars().count() >= MIN_TOKEN_CHARS)
        .collect()
}

/// Runs the full coverage check: for every real cell, anonymizes it with
/// `anonymizer`, then checks both directions -- negative: none of that
/// cell's own real tokens survive as a substring of its own dummy output;
/// positive: the dummy output matches `is_valid_dummy_value`'s shape for
/// that column.
pub fn run(real_cells: &[WbTextCell], anonymizer: &mut Anonymizer) -> LeakCheckReport {
    run_with(real_cells, |cell| {
        wb_anonymize::anonymize_cell(anonymizer, &cell.table_name, &cell.column_name, &cell.value)
    })
}

/// `run` with the anonymization step injected, so the checks themselves can
/// be exercised against a deliberately broken anonymizer.
fn run_with(
    real_cells: &[WbTextCell],
    mut anonymize: impl FnMut(&WbTextCell) -> String,
) -> LeakCheckReport {
    let mut total_candidates = 0usize;
    let mut total_kept = 0usize;
    let mut kept_min_len: Option<usize> = None;
    let mut kept_max_len: Option<usize> = None;
    let mut negative_violations = 0usize;
    let mut positive_violations = 0usize;

    for cell in real_cells {
        let dummy = anonymize(cell);

        let all_candidates: HashSet<String> = candidate_tokens(&cell.value)
            .filter(|t| !t.is_empty())
            .collect();
        total_candidates += all_candidates.len();
        let forbidden = forbidden_tokens_for(&cell.value);
        total_kept += forbidden.len();
        for tok in &forbidden {
            let len = tok.chars().count();
            kept_min_len = Some(kept_min_len.map_or(len, |m| m.min(len)));
            kept_max_len = Some(kept_max_len.map_or(len, |m| m.max(len)));
        }

        if forbidden.iter().any(|tok| dummy.contains(tok.as_str())) {
            negative_violations += 1;
        }
        if !wb_anonymize::is_valid_dummy_value(&cell.table_name, &cell.column_name, &dummy) {
            positive_violations += 1;
        }
    }

    LeakCheckReport {
        token_stats: TokenStats {
            candidate_count: total_candidates,
            excluded_short: total_candidates - total_kept,
            kept_min_len,
            kept_max_len,
        },
        cells_checked: real_cells.len(),
        negative_violations,
        positive_violations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(table: &str, column: &str, row_id: i64, value: &str) -> WbTextCell {
        WbTextCell {
            table_name: table.to_string(),
            column_name: column.to_string(),
            row_id,
            value: value.to_string(),
        }
    }

    #[test]
    fn passes_for_normal_looking_cells() {
        let cells = vec![
            cell("movie", "movie_path", 1, "T:\\videos\\clip.mp4"),
            cell("movie", "tag", 1, "foo\nbar"),
            cell("movie", "hash", 1, "1e5e0fbf"),
            cell("findfact", "find_text", 1, "some search term"),
        ];
        let mut anonymizer = Anonymizer::new();
        let report = run(&cells, &mut anonymizer);
        assert!(
            report.passed(),
            "expected pass, got negative={} positive={}",
            report.negative_violations,
            report.positive_violations
        );
        assert_eq!(report.cells_checked, 4);
    }

    #[test]
    fn detects_a_negative_violation_when_anonymization_is_a_no_op() {
        // An anonymizer that echoes its input back: a hash-shaped value
        // passes the positive/shape check, so only the negative check can
        // catch it.
        let cells = vec![cell("movie", "hash", 1, "1e5e0fbf")];
        let report = run_with(&cells, |c| c.value.clone());
        assert_eq!(report.negative_violations, 1);
        assert_eq!(report.positive_violations, 0);
        assert!(!report.passed());
    }

    #[test]
    fn detects_a_positive_violation_when_output_does_not_match_the_dummy_shape() {
        // The output shares no token with the input (no negative
        // violation) but is not shaped like any dummy value.
        let cells = vec![cell("findfact", "find_text", 1, "some search term")];
        let report = run_with(&cells, |_| "unrelated".to_string());
        assert_eq!(report.negative_violations, 0);
        assert_eq!(report.positive_violations, 1);
        assert!(!report.passed());
    }

    #[test]
    fn token_stats_exclude_short_tokens_and_report_only_counts() {
        // Candidates: the whole value, "T:", "ab", "videos". "T:" and "ab"
        // are shorter than MIN_TOKEN_CHARS and are excluded.
        let cells = vec![cell("watch", "dir", 1, "T:\\ab\\videos")];
        let mut anonymizer = Anonymizer::new();
        let report = run(&cells, &mut anonymizer);
        assert_eq!(report.token_stats.candidate_count, 4);
        assert_eq!(report.token_stats.excluded_short, 2);
        assert_eq!(report.token_stats.kept_min_len, Some("videos".len()));
        assert_eq!(
            report.token_stats.kept_max_len,
            Some("T:\\ab\\videos".len())
        );
    }

    #[test]
    fn display_reports_counts_only() {
        let cells = vec![cell("findfact", "find_text", 1, "secret term")];
        let mut anonymizer = Anonymizer::new();
        let text = run(&cells, &mut anonymizer).to_string();
        assert!(text.contains("cells_checked=1"), "{text}");
        assert!(text.contains("negative_violations=0"), "{text}");
        assert!(!text.contains("secret"), "{text}");
    }

    #[test]
    fn does_not_flag_unrelated_cells_sharing_a_short_hex_like_substring() {
        // Two different real values whose dummy outputs might incidentally
        // share short substrings with each other's tokens must not count as
        // violations -- only a cell's *own* tokens count against its *own*
        // output.
        let cells = vec![
            cell("findfact", "find_text", 1, "cafe"),
            cell("findfact", "find_text", 2, "decade"),
        ];
        let mut anonymizer = Anonymizer::new();
        let report = run(&cells, &mut anonymizer);
        assert!(report.passed());
    }
}
