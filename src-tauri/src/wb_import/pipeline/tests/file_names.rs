//! Rows refused for a machine-dependent file name.

use super::common::{
    count_of, harness, import_all, import_registering, import_with_lister, named_row, no_op_ffmpeg,
    registering_progress, replaced_row, row, stored_watch_folders, Harness, OnlineDir, DASH,
};
use super::*;

// --- machine-dependent file names -----------------------------------------

fn skipped_file_paths(h: &Harness) -> Vec<String> {
    let conn = h.db.writer.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT file_path FROM skipped_files ORDER BY file_path")
        .unwrap();
    let paths = stmt
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<Vec<String>, _>>()
        .unwrap();
    paths
}

#[test]
fn machine_dependent_names_are_recorded_as_skipped_files_not_videos() {
    let h = harness();
    let names = [
        "circled\u{2460}.mp4",
        "roman\u{2163}.mp4",
        "corp\u{3231}.mp4",
    ];
    let dir = OnlineDir::new("plain.mp4");
    for name in names {
        std::fs::write(dir.dir.path().join(name), b"video bytes").unwrap();
    }
    let rows = names
        .iter()
        .enumerate()
        .map(|(i, n)| named_row(i as i64 + 1, &dir.folder(), n))
        .chain([named_row(9, &dir.folder(), "plain.mp4")])
        .collect();

    let summary = import_with_lister(&h, rows, &FakeDirLister::default(), None);

    assert_eq!(summary.skipped_invalid_name, 3);
    assert_eq!(summary.registered, 1, "the plain name registers as before");
    assert_eq!(count_of(&h, "SELECT COUNT(*) FROM videos"), 1);
    assert_eq!(
        count_of(
            &h,
            "SELECT COUNT(*) FROM videos WHERE file_name = 'plain.mp4' AND status = 'online'"
        ),
        1
    );
    let mut expected: Vec<String> = names.iter().map(|n| dir.file(n)).collect();
    expected.sort();
    assert_eq!(skipped_file_paths(&h), expected);
    assert_eq!(
        count_of(
            &h,
            "SELECT COUNT(*) FROM skipped_files WHERE reason = 'machine_dependent_char'"
        ),
        3
    );
}

#[test]
fn offline_rows_are_judged_by_name_alone() {
    let h = harness();
    let row = named_row(1, "Z:\\gone", "off\u{2460}.mp4");

    let summary = import_with_lister(&h, vec![row], &FakeDirLister::default(), None);

    assert_eq!(summary.skipped_invalid_name, 1);
    assert_eq!(summary.registered, 0);
    assert_eq!(count_of(&h, "SELECT COUNT(*) FROM videos"), 0);
    assert_eq!(
        skipped_file_paths(&h),
        vec!["Z:\\gone\\off\u{2460}.mp4".to_string()]
    );
    let payload: Option<String> =
        h.db.writer
            .lock()
            .unwrap()
            .query_row("SELECT wb_payload FROM skipped_files", [], |r| r.get(0))
            .unwrap();
    assert!(
        WbSkippedPayload::from_json(&payload.expect("a .wb import keeps the row's data")).is_some()
    );
}

#[test]
fn the_name_is_judged_after_restoring_question_marks() {
    let h = harness();
    let real = "a\u{2460}.mp4".to_string();
    let dir = OnlineDir::new(&real);
    let lister = FakeDirLister::default().with_dir(&dir.folder(), &[&real]);

    let summary = import_with_lister(&h, vec![replaced_row(1, &dir.folder(), "a")], &lister, None);

    assert_eq!(summary.skipped_invalid_name, 1);
    assert_eq!(count_of(&h, "SELECT COUNT(*) FROM videos"), 0);
    assert_eq!(skipped_file_paths(&h), vec![dir.file(&real)]);
}

#[test]
fn a_question_mark_name_that_restores_to_a_clean_name_still_registers() {
    let h = harness();
    let real = format!("a{DASH}.mp4");
    let dir = OnlineDir::new(&real);
    let lister = FakeDirLister::default().with_dir(&dir.folder(), &[&real]);

    let summary = import_with_lister(&h, vec![replaced_row(1, &dir.folder(), "a")], &lister, None);

    assert_eq!(summary.skipped_invalid_name, 0);
    assert_eq!(summary.registered, 1);
    assert!(skipped_file_paths(&h).is_empty());
}

#[test]
fn a_refused_rows_folder_does_not_become_a_watch_folder() {
    let h = harness();
    let dir = OnlineDir::new("bad\u{2460}.mp4");
    let reconf = FakeWatchReconfigurer::default();

    let summary = import_registering(
        &h,
        vec![named_row(1, &dir.folder(), "bad\u{2460}.mp4")],
        &reconf,
    );

    assert_eq!(summary.skipped_invalid_name, 1);
    assert!(summary.watch_folders_added.is_empty());
    assert!(stored_watch_folders(&h).is_empty());
    assert!(reconf.calls.lock().unwrap().is_empty());
}

#[test]
fn reimporting_does_not_duplicate_skipped_files() {
    let h = harness();
    let rows = || vec![named_row(1, "Z:\\gone", "off\u{2460}.mp4")];

    import_with_lister(&h, rows(), &FakeDirLister::default(), None);
    let second = import_with_lister(&h, rows(), &FakeDirLister::default(), None);

    assert_eq!(second.skipped_invalid_name, 1);
    assert_eq!(count_of(&h, "SELECT COUNT(*) FROM skipped_files"), 1);
    assert_eq!(count_of(&h, "SELECT COUNT(*) FROM videos"), 0);
}

#[test]
fn a_refused_rows_legacy_jpg_is_not_counted_for_manual_attention() {
    let h = harness();
    let dir = OnlineDir::new("bad\u{2460}.mp4");
    let bad = row(|r| {
        r.movie_id = 1;
        r.movie_name = "bad\u{2460}.mp4".to_string();
        r.movie_path = dir.file("bad\u{2460}.mp4");
        r.hash = "1e5e0fbf".to_string();
    });
    let offline_bad = row(|r| {
        r.movie_id = 2;
        r.movie_name = "off\u{2460}.mp4".to_string();
        r.movie_path = "Z:\\gone\\off\u{2460}.mp4".to_string();
        r.hash = "2f6f1cc0".to_string();
    });
    for name in [
        "bad\u{2460}.mp4.#1e5e0fbf.jpg",
        "off\u{2460}.mp4.#2f6f1cc0.jpg",
    ] {
        std::fs::write(h.paths.thumbnail_folder.join(name), b"fake jpg").unwrap();
    }
    let ffmpeg = no_op_ffmpeg();
    let notifier = FakeWbImportNotifier::default();

    import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies: Ok(vec![bad, offline_bad]),
            ..Default::default()
        },
        &ffmpeg,
        &notifier,
        &FakeCatalogNotifier::default(),
    );

    let summary = notifier.complete_calls.lock().unwrap()[0].clone();
    assert_eq!(summary.skipped_invalid_name, 2);
    assert_eq!(summary.thumbnails_unmatched, 0);
    assert_eq!(summary.thumbnails_skipped_online, 0);
    assert_eq!(summary.thumbnails_linked, 0);
    assert!(
        ffmpeg.calls.lock().unwrap().is_empty(),
        "no thumbnail work for a refused row"
    );
}

#[test]
fn an_unrelated_jpg_is_still_unmatched_next_to_refused_rows() {
    let h = harness();
    let bad = row(|r| {
        r.movie_id = 1;
        r.movie_name = "off\u{2460}.mp4".to_string();
        r.movie_path = "Z:\\gone\\off\u{2460}.mp4".to_string();
        r.hash = "2f6f1cc0".to_string();
    });
    for name in ["off\u{2460}.mp4.#2f6f1cc0.jpg", "Other.mp4.#deadbeef.jpg"] {
        std::fs::write(h.paths.thumbnail_folder.join(name), b"fake jpg").unwrap();
    }
    let notifier = FakeWbImportNotifier::default();

    import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies: Ok(vec![bad]),
            ..Default::default()
        },
        &no_op_ffmpeg(),
        &notifier,
        &FakeCatalogNotifier::default(),
    );

    let summary = notifier.complete_calls.lock().unwrap()[0].clone();
    assert_eq!(summary.thumbnails_unmatched, 1);
}

#[test]
fn progress_still_reaches_the_last_row_when_it_is_refused() {
    let h = harness();
    let wb_source = FakeWbSourceAdapter {
        movies: Ok(vec![
            named_row(1, "Z:\\gone", "ok.mp4"),
            named_row(2, "Z:\\gone", "bad\u{2460}.mp4"),
        ]),
        ..Default::default()
    };
    let notifier = FakeWbImportNotifier::default();

    import_all(
        &h.db,
        &h.paths,
        &wb_source,
        &no_op_ffmpeg(),
        &notifier,
        &FakeCatalogNotifier::default(),
    );

    assert_eq!(
        registering_progress(&notifier),
        vec![(2, Some(2))],
        "the refused last row reports the final count itself"
    );
}
