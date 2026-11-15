//! Runs the full leak/coverage check against the
//! developer's real `.wb` sample. Read-only against the real file. Skips
//! itself (does not fail) when the local fixture is absent, since it is
//! never committed and CI has no access to it.
//!
//! Prints only counts (never real or dummy values) so this is safe to run
//! with output visible (`cargo test -- --nocapture`).

use std::path::PathBuf;

use gb_core::ports::wb_source::WbSourceAdapter;
use gb_core::wb_anonymize::Anonymizer;
use wb_anonymize_tool::{leak_check, reader};

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

    let wb = reader::ReadOnlyWb::open(&path).expect("should open the real .wb read-only");
    let movies = wb.read_movies().expect("read_movies should succeed");
    let real_cells = wb
        .read_all_text_cells()
        .expect("read_all_text_cells should succeed");
    drop(wb);

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

    eprintln!("leak_check against real data: {report}");

    assert_eq!(
        report.negative_violations, 0,
        "no forbidden real-data token should survive anonymization"
    );
    assert_eq!(
        report.positive_violations, 0,
        "every anonymized value should match its dummy generation rule"
    );
}
