//! Integration test against the developer's real `.wb` sample
//! (`tests/fixtures/wb/local/default_20110504.wb`, gitignored).
//! Opens it strictly read-only via `RealWbSourceAdapter`
//! and asserts only counts/patterns already confirmed by manual analysis --
//! never real paths, tags, or other cell content (those must never appear in
//! test output or CI logs).
//!
//! Skips itself (prints a message, does not fail) when the local fixture is
//! absent, since it is never committed and CI has no access to it.

use std::path::PathBuf;

use gb_core::ports::wb_source::WbSourceAdapter;
use gb_core::wb_import::count_clamped_scores;
use graybrowser_lib::adapters::wb_source::RealWbSourceAdapter;

fn local_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/wb/local/default_20110504.wb")
}

#[test]
fn real_wb_fixture_matches_manually_verified_structure() {
    let path = local_fixture_path();
    if !path.exists() {
        eprintln!(
            "skipping: local .wb fixture not present at {} (never committed, developer-only)",
            path.display()
        );
        return;
    }

    let adapter = RealWbSourceAdapter::open(&path).expect("should open the real .wb read-only");
    let movies = adapter.read_movies().expect("read_movies should succeed");

    assert_eq!(movies.len(), 3072, "movie row count");

    let non_empty_hash = movies.iter().filter(|m| !m.hash.is_empty()).count();
    let distinct_hash = movies
        .iter()
        .filter(|m| !m.hash.is_empty())
        .map(|m| m.hash.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len();
    assert_eq!(non_empty_hash, 3071, "non-empty hash row count");
    assert_eq!(distinct_hash, 3071, "hash values should be unique");
    assert!(
        movies
            .iter()
            .filter(|m| !m.hash.is_empty())
            .all(|m| m.hash.len() == 8
                && m.hash
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())),
        "every non-empty hash should be 8 lowercase hex chars"
    );

    assert_eq!(count_clamped_scores(&movies), 89, "rows with score > 5");

    let non_empty_tags = movies.iter().filter(|m| !m.tag.is_empty()).count();
    assert_eq!(non_empty_tags, 816, "non-empty tag row count");
    assert!(
        movies
            .iter()
            .filter(|m| !m.tag.is_empty())
            .all(|m| m.tag.contains('\n')),
        "every non-empty tag should contain a newline separator"
    );
}
