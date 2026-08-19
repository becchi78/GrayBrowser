//! Deterministic stratified sampling of `.wb` `movie` rows for
//! `sample_small.wb`. Pure logic only -- no file I/O.

use std::collections::BTreeSet;

use crate::ports::wb_source::WbMovieRow;
use crate::wb_anonymize::{is_drive_letter_segment, path_segments};

const PER_STRATUM_LIMIT: usize = 5;

/// Selects a deterministic, structurally representative subset of `rows`
/// for use as a small test fixture: every row with an empty `hash` (real
/// data's known gap), a spread of `score` values including the 0/1-5/6-20
/// bands and an exact-20 row if one exists, both empty and multi-line `tag`
/// rows, rows with an empty `kana`/`roma`, and one row per distinct drive
/// letter seen in `movie_path`. Once those strata are satisfied, fills up to
/// `target_count` with additional rows in `movie_id` order.
///
/// `target_count` is a soft floor for the fill step, not a hard cap --
/// mandatory edge cases (the empty-hash rows) are always included even if
/// that pushes the result slightly past `target_count`. Selection is
/// deterministic: the same `rows` always produces the same output, in
/// ascending `movie_id` order.
pub fn select_sample_rows(rows: &[WbMovieRow], target_count: usize) -> Vec<WbMovieRow> {
    let mut sorted: Vec<&WbMovieRow> = rows.iter().collect();
    sorted.sort_by_key(|r| r.movie_id);

    let mut selected_ids: BTreeSet<i64> = BTreeSet::new();
    let mut selected: Vec<WbMovieRow> = Vec::new();

    // Mandatory edge case: every row with an empty hash (real data has 1).
    take_matching(&sorted, &mut selected_ids, &mut selected, usize::MAX, |r| {
        r.hash.is_empty()
    });

    // score strata
    take_matching(&sorted, &mut selected_ids, &mut selected, 1, |r| {
        r.score == 20
    });
    take_matching(
        &sorted,
        &mut selected_ids,
        &mut selected,
        PER_STRATUM_LIMIT,
        |r| r.score == 0,
    );
    take_matching(
        &sorted,
        &mut selected_ids,
        &mut selected,
        PER_STRATUM_LIMIT,
        |r| (1..=5).contains(&r.score),
    );
    take_matching(
        &sorted,
        &mut selected_ids,
        &mut selected,
        PER_STRATUM_LIMIT,
        |r| r.score > 5,
    );

    // tag strata
    take_matching(
        &sorted,
        &mut selected_ids,
        &mut selected,
        PER_STRATUM_LIMIT,
        |r| r.tag.contains('\n'),
    );
    take_matching(
        &sorted,
        &mut selected_ids,
        &mut selected,
        PER_STRATUM_LIMIT,
        |r| !r.tag.is_empty() && !r.tag.contains('\n'),
    );
    take_matching(
        &sorted,
        &mut selected_ids,
        &mut selected,
        PER_STRATUM_LIMIT,
        |r| r.tag.is_empty(),
    );

    // kana/roma emptiness (only matters if real data actually has any empty ones)
    take_matching(
        &sorted,
        &mut selected_ids,
        &mut selected,
        PER_STRATUM_LIMIT,
        |r| r.kana.is_empty() || r.roma.is_empty(),
    );

    // Drive-letter diversity: one row per distinct drive letter seen.
    let mut seen_drives: BTreeSet<String> = BTreeSet::new();
    for row in &sorted {
        if selected_ids.contains(&row.movie_id) {
            continue;
        }
        if let Some(first_segment) = path_segments(&row.movie_path).first() {
            if is_drive_letter_segment(first_segment)
                && seen_drives.insert(first_segment.to_uppercase())
            {
                selected_ids.insert(row.movie_id);
                selected.push((*row).clone());
            }
        }
    }

    // Fill up to target_count with remaining rows in movie_id order.
    if selected.len() < target_count {
        let remaining = target_count - selected.len();
        take_matching(&sorted, &mut selected_ids, &mut selected, remaining, |_| {
            true
        });
    }

    selected.sort_by_key(|r| r.movie_id);
    selected
}

fn take_matching(
    sorted: &[&WbMovieRow],
    selected_ids: &mut BTreeSet<i64>,
    selected: &mut Vec<WbMovieRow>,
    limit: usize,
    predicate: impl Fn(&WbMovieRow) -> bool,
) {
    let mut taken = 0;
    for row in sorted {
        if taken >= limit {
            break;
        }
        if selected_ids.contains(&row.movie_id) {
            continue;
        }
        if predicate(row) {
            selected_ids.insert(row.movie_id);
            selected.push((*row).clone());
            taken += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, overrides: impl FnOnce(&mut WbMovieRow)) -> WbMovieRow {
        let mut row = WbMovieRow {
            movie_id: id,
            movie_name: format!("movie{id}.mp4"),
            movie_path: format!("T:\\videos\\movie{id}.mp4"),
            tag: String::new(),
            score: 0,
            hash: format!("{id:08x}"),
            kana: "かな".to_string(),
            roma: "kana".to_string(),
            file_date: "2011-05-04 12:00:00".to_string(),
            regist_date: "2011-05-04 12:00:00".to_string(),
            last_date: "2011-05-04 12:00:00".to_string(),
        };
        overrides(&mut row);
        row
    }

    fn sample_dataset() -> Vec<WbMovieRow> {
        vec![
            row(1, |r| r.hash = String::new()), // known-gap edge case
            row(2, |r| r.score = 0),
            row(3, |r| r.score = 3),
            row(4, |r| r.score = 20),
            row(5, |r| r.score = 12),
            row(6, |r| r.tag = "foo\nbar".to_string()),
            row(7, |r| r.tag = "single".to_string()),
            row(8, |_| {}), // empty tag
            row(9, |r| {
                r.kana = String::new();
            }),
            row(10, |r| r.movie_path = "U:\\videos\\movie10.mp4".to_string()),
            row(11, |r| r.movie_path = "V:\\videos\\movie11.mp4".to_string()),
            row(12, |r| r.movie_path = "W:\\videos\\movie12.mp4".to_string()),
        ]
    }

    #[test]
    fn includes_every_empty_hash_row() {
        let selected = select_sample_rows(&sample_dataset(), 5);
        assert!(selected
            .iter()
            .any(|r| r.movie_id == 1 && r.hash.is_empty()));
    }

    #[test]
    fn includes_the_exact_score_twenty_row_when_present() {
        let selected = select_sample_rows(&sample_dataset(), 5);
        assert!(selected.iter().any(|r| r.score == 20));
    }

    #[test]
    fn includes_both_empty_and_multiline_tag_rows() {
        let selected = select_sample_rows(&sample_dataset(), 5);
        assert!(selected.iter().any(|r| r.tag.contains('\n')));
        assert!(selected.iter().any(|r| r.tag.is_empty()));
    }

    #[test]
    fn includes_a_row_per_distinct_drive_letter() {
        let selected = select_sample_rows(&sample_dataset(), 5);
        let drives: BTreeSet<String> = selected
            .iter()
            .filter_map(|r| {
                path_segments(&r.movie_path)
                    .first()
                    .map(|s| s.to_uppercase())
            })
            .collect();
        assert!(drives.contains("T:"));
        assert!(drives.contains("U:"));
        assert!(drives.contains("V:"));
        assert!(drives.contains("W:"));
    }

    #[test]
    fn is_deterministic_across_repeated_calls() {
        let dataset = sample_dataset();
        let first = select_sample_rows(&dataset, 5);
        let second = select_sample_rows(&dataset, 5);
        assert_eq!(first, second);
    }

    #[test]
    fn result_is_sorted_by_movie_id() {
        let selected = select_sample_rows(&sample_dataset(), 5);
        let ids: Vec<i64> = selected.iter().map(|r| r.movie_id).collect();
        let mut sorted_ids = ids.clone();
        sorted_ids.sort();
        assert_eq!(ids, sorted_ids);
    }

    #[test]
    fn fill_step_respects_target_count_as_a_floor_not_a_ceiling_violation() {
        // With a tiny dataset, target_count larger than available rows should
        // just return everything, not panic.
        let dataset = sample_dataset();
        let selected = select_sample_rows(&dataset, 1000);
        assert_eq!(selected.len(), dataset.len());
    }
}
