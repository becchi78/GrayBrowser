//! Restoring `?` in stored paths from the parent folder's contents.

use super::common::{
    harness, import_with_lister, replaced_row, stored_watch_folders, Harness, OnlineDir, DASH,
};
use super::*;

// --- restoring `?` in stored paths ---------------------------------------

fn video_columns(h: &Harness) -> (String, String, String, String) {
    h.db.writer
        .lock()
        .unwrap()
        .query_row(
            "SELECT file_path, file_name, status, quick_hash FROM videos",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap()
}

#[test]
fn a_question_mark_path_is_restored_and_registered_online_under_the_real_name() {
    let h = harness();
    let real = format!("a{DASH}.mp4");
    let dir = OnlineDir::new(&real);
    let lister = FakeDirLister::default().with_dir(&dir.folder(), &[&real]);

    let summary = import_with_lister(&h, vec![replaced_row(1, &dir.folder(), "a")], &lister, None);

    assert_eq!(summary.registered, 1);
    let (file_path, file_name, status, quick_hash) = video_columns(&h);
    assert_eq!(file_path, dir.file(&real));
    assert_eq!(file_name, real, "the name must not keep the `?`");
    assert_eq!(status, "online");
    assert_ne!(quick_hash, "");
}

#[test]
fn several_candidates_leave_the_row_offline_with_the_stored_path() {
    let h = harness();
    let dir = OnlineDir::new("unrelated.mp4");
    let lister =
        FakeDirLister::default().with_dir(&dir.folder(), &[&format!("a{DASH}.mp4"), "a-.mp4"]);
    let row = replaced_row(1, &dir.folder(), "a");
    let stored = row.movie_path.clone();

    let summary = import_with_lister(&h, vec![row], &lister, None);

    assert_eq!(summary.registered, 1);
    let (file_path, file_name, status, quick_hash) = video_columns(&h);
    assert_eq!(file_path, stored);
    assert_eq!(file_name, "a?.mp4");
    assert_eq!(status, "offline");
    assert_eq!(quick_hash, "");
}

#[test]
fn an_unlistable_folder_leaves_the_row_offline() {
    let h = harness();
    let dir = OnlineDir::new(&format!("a{DASH}.mp4"));
    let row = replaced_row(1, &dir.folder(), "a");
    let stored = row.movie_path.clone();

    import_with_lister(&h, vec![row], &FakeDirLister::default(), None);

    let (file_path, _, status, _) = video_columns(&h);
    assert_eq!(file_path, stored);
    assert_eq!(status, "offline");
}

#[test]
fn a_folder_is_listed_once_per_import_however_many_rows_need_it() {
    let h = harness();
    let names: Vec<String> = ["a", "b", "c"]
        .iter()
        .map(|stem| format!("{stem}{DASH}.mp4"))
        .collect();
    let dir = OnlineDir::new(&names[0]);
    for name in &names[1..] {
        std::fs::write(dir.dir.path().join(name), b"video bytes").unwrap();
    }
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let lister = FakeDirLister::default().with_dir(&dir.folder(), &name_refs);

    let summary = import_with_lister(
        &h,
        vec![
            replaced_row(1, &dir.folder(), "a"),
            replaced_row(2, &dir.folder(), "b"),
            replaced_row(3, &dir.folder(), "c"),
        ],
        &lister,
        None,
    );

    assert_eq!(summary.registered, 3);
    assert_eq!(lister.call_count_for(&dir.folder()), 1);
}

#[test]
fn a_folder_that_cannot_be_listed_is_not_retried_for_each_row() {
    let h = harness();
    let lister = FakeDirLister::default();
    let folder = "Z:\\gone";

    import_with_lister(
        &h,
        vec![
            replaced_row(1, folder, "a"),
            replaced_row(2, folder, "b"),
            replaced_row(3, folder, "c"),
        ],
        &lister,
        None,
    );

    assert_eq!(lister.call_count_for(folder), 1);
}

#[test]
fn the_folder_of_a_restored_online_video_becomes_a_watch_folder() {
    let h = harness();
    let real = format!("a{DASH}.mp4");
    let dir = OnlineDir::new(&real);
    let lister = FakeDirLister::default().with_dir(&dir.folder(), &[&real]);
    let reconf = FakeWatchReconfigurer::default();

    let summary = import_with_lister(
        &h,
        vec![replaced_row(1, &dir.folder(), "a")],
        &lister,
        Some(&reconf),
    );

    assert_eq!(summary.watch_folders_added, vec![dir.folder()]);
    assert_eq!(stored_watch_folders(&h), vec![dir.folder()]);
}

#[test]
fn reimporting_skips_the_restored_path() {
    let h = harness();
    let real = format!("a{DASH}.mp4");
    let dir = OnlineDir::new(&real);
    let lister = FakeDirLister::default().with_dir(&dir.folder(), &[&real]);
    let rows = || vec![replaced_row(1, &dir.folder(), "a")];

    import_with_lister(&h, rows(), &lister, None);
    let second = import_with_lister(&h, rows(), &lister, None);

    assert_eq!(second.registered, 0);
    assert_eq!(second.skipped, 1);
    let count: i64 =
        h.db.writer
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap();
    assert_eq!(count, 1);
}
