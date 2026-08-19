//! Integration test against the committed, anonymized `sample_small.wb`
//! fixture (`tests/fixtures/wb/sample_small.wb`, generated from real data by
//! `wb-anonymize-tool` -- see `tests/fixtures/wb/README.md`). Unlike
//! `wb_source_local_fixture.rs`, this fixture is committed and contains no
//! real personal data, so this test runs in CI.

use std::path::PathBuf;

use gb_core::ports::wb_source::WbSourceAdapter;
use gb_core::wb_import::count_clamped_scores;
use graybrowser_lib::adapters::wb_source::RealWbSourceAdapter;

fn sample_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/wb/sample_small.wb")
}

#[test]
fn sample_small_wb_parses_via_the_production_adapter() {
    let path = sample_fixture_path();
    let adapter = RealWbSourceAdapter::open(&path)
        .expect("should open the committed sample fixture read-only");
    let movies = adapter.read_movies().expect("read_movies should succeed");

    assert!(
        !movies.is_empty(),
        "sample fixture should have at least one movie row"
    );

    // Structural properties the sampler (gb_core::wb_sampling) guarantees:
    // at least one empty-hash row, at least one score > 5 (clamp-triggering)
    // row, both empty and multi-line tag rows, and unique non-empty hashes.
    assert!(
        movies.iter().any(|m| m.hash.is_empty()),
        "sample should include the known empty-hash edge case"
    );
    assert!(
        count_clamped_scores(&movies) > 0,
        "sample should include at least one score>5 row"
    );
    assert!(
        movies.iter().any(|m| m.tag.is_empty()),
        "sample should include an empty-tag row"
    );
    assert!(
        movies.iter().any(|m| m.tag.contains('\n')),
        "sample should include a multi-line tag row"
    );

    let non_empty_hashes: Vec<&str> = movies
        .iter()
        .filter(|m| !m.hash.is_empty())
        .map(|m| m.hash.as_str())
        .collect();
    let distinct: std::collections::HashSet<&str> = non_empty_hashes.iter().copied().collect();
    assert_eq!(
        distinct.len(),
        non_empty_hashes.len(),
        "all non-empty hashes in the sample should be unique"
    );
    assert!(
        non_empty_hashes.iter().all(|h| h.len() == 8
            && h.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())),
        "every non-empty hash should be 8 lowercase hex chars"
    );
}
