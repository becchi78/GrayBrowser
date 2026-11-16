//! `/` in a stored path is folded to `\` before anything is registered.

use super::common::{harness, import_with_lister, replaced_row, row, OnlineDir, DASH};
use super::*;

fn stored_paths(h: &common::Harness) -> Vec<String> {
    let conn = h.db.writer.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT file_path FROM videos ORDER BY file_path")
        .unwrap();
    let rows = stmt.query_map([], |r| r.get(0)).unwrap();
    rows.map(Result::unwrap).collect()
}

fn slash_row(id: i64, path: &str) -> WbMovieRow {
    row(|r| {
        r.movie_id = id;
        r.movie_name = "movie.mp4".to_string();
        r.movie_path = path.to_string();
    })
}

#[test]
fn a_forward_slash_path_is_stored_with_backslashes() {
    let h = harness();
    let summary = import_with_lister(
        &h,
        vec![slash_row(1, "T:/videos/sub/movie.mp4")],
        &FakeDirLister::default(),
        None,
    );

    assert_eq!(summary.registered, 1);
    assert_eq!(stored_paths(&h), vec![r"T:\videos\sub\movie.mp4"]);
}

#[test]
fn a_restored_question_mark_path_is_stored_with_backslashes() {
    let h = harness();
    let real = format!("a{DASH}.mp4");
    let dir = OnlineDir::new(&real);
    let lister = FakeDirLister::default().with_dir(&dir.folder(), &[&real]);
    let mut r = replaced_row(1, &dir.folder(), "a");
    r.movie_path = r.movie_path.replace(char::from(92), "/");

    let summary = import_with_lister(&h, vec![r], &lister, None);

    assert_eq!(summary.registered, 1);
    assert_eq!(stored_paths(&h), vec![dir.file(&real)]);
}

#[test]
fn slash_and_backslash_forms_of_one_path_are_the_same_video() {
    let h = harness();
    let lister = FakeDirLister::default();
    let first = import_with_lister(
        &h,
        vec![slash_row(1, r"T:\videos\movie.mp4")],
        &lister,
        None,
    );
    assert_eq!(first.registered, 1);

    let second = import_with_lister(&h, vec![slash_row(1, "T:/videos/movie.mp4")], &lister, None);

    assert_eq!(second.registered, 0);
    assert_eq!(second.skipped, 1);
    assert_eq!(stored_paths(&h), vec![r"T:\videos\movie.mp4"]);
}

#[test]
fn a_backslash_path_reimport_is_still_skipped() {
    let h = harness();
    let lister = FakeDirLister::default();
    import_with_lister(
        &h,
        vec![slash_row(1, r"T:\videos\movie.mp4")],
        &lister,
        None,
    );

    let second = import_with_lister(
        &h,
        vec![slash_row(1, r"T:\videos\movie.mp4")],
        &lister,
        None,
    );

    assert_eq!(second.registered, 0);
    assert_eq!(second.skipped, 1);
}
