//! Runs the full leak/coverage check against the
//! developer's real `.wb` sample. Read-only against the real file; any
//! output it writes goes to a tempdir, never to a committed path. Skips
//! itself (does not fail) when the local fixture is absent, since it is
//! never committed and CI has no access to it.
//!
//! Prints only counts (never real or dummy values) so this is safe to run
//! with output visible (`cargo test -- --nocapture`).

use std::path::PathBuf;

use gb_core::wb_anonymize::Anonymizer;
use wb_anonymize_tool::{leak_check, reader, writer};

fn local_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/wb/local/default_20110504.wb")
}

#[test]
fn anonymization_leaves_no_real_token_and_matches_dummy_shape() {
    let path = local_fixture_path();
    if !path.exists() {
        eprintln!(
            "skipping: local .wb fixture not present at {} (never committed, developer-only)",
            path.display()
        );
        return;
    }

    let conn = reader::open_readonly(&path).expect("should open the real .wb read-only");
    let movies = reader::read_movies(&conn).expect("read_movies should succeed");
    let real_cells =
        reader::read_all_text_cells(&conn).expect("read_all_text_cells should succeed");
    drop(conn);

    let mut anonymizer = Anonymizer::new();
    // Anonymize the movie rows too so the shared Anonymizer's hash-collision
    // state matches what the CLI tool itself would produce (see main.rs's
    // comment on why row-level and cell-level anonymization share one
    // Anonymizer instance).
    let _anonymized_movies: Vec<_> = {
        let mut sorted = movies.clone();
        sorted.sort_by_key(|r| r.movie_id);
        sorted
            .iter()
            .map(|r| anonymizer.anonymize_movie_row(r))
            .collect()
    };

    let report = leak_check::run(&real_cells, &mut anonymizer);

    eprintln!(
        "leak_check against real data: cells_checked={} token_candidate_count={} token_excluded_short={} \
         token_kept_min_len={:?} token_kept_max_len={:?} negative_violations={} positive_violations={}",
        report.cells_checked,
        report.token_stats.candidate_count,
        report.token_stats.excluded_short,
        report.token_stats.kept_min_len,
        report.token_stats.kept_max_len,
        report.negative_violations,
        report.positive_violations,
    );

    assert_eq!(
        report.negative_violations, 0,
        "no forbidden real-data token should survive anonymization"
    );
    assert_eq!(
        report.positive_violations, 0,
        "every anonymized value should match its dummy generation rule"
    );
}

#[test]
fn writer_round_trips_through_a_temp_file_without_touching_real_data() {
    let path = local_fixture_path();
    if !path.exists() {
        eprintln!("skipping: local .wb fixture not present (developer-only)");
        return;
    }

    let conn = reader::open_readonly(&path).expect("should open the real .wb read-only");
    let mut movies = reader::read_movies(&conn).expect("read_movies should succeed");
    movies.sort_by_key(|r| r.movie_id);
    drop(conn);

    let mut anonymizer = Anonymizer::new();
    let anonymized: Vec<_> = movies
        .iter()
        .take(10)
        .map(|r| anonymizer.anonymize_movie_row(r))
        .collect();

    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    let out_path = tmp.path().join("sample_small_test.wb");
    let out_conn = writer::create_output(&out_path)
        .expect("create_output should succeed on a fresh temp path");
    writer::write_movies(&out_conn, &anonymized).expect("write_movies should succeed");
    drop(out_conn);

    let verify_conn =
        reader::open_readonly(&out_path).expect("should reopen the written file read-only");
    let round_tripped =
        reader::read_movies(&verify_conn).expect("read_movies should succeed on the written file");
    assert_eq!(round_tripped.len(), anonymized.len());
}
