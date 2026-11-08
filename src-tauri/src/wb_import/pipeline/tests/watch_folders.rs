//! Registering the imported videos' folders as watch folders.

use super::common::{
    harness, import_all, import_registering, no_op_ffmpeg, row, stored_watch_folders,
    unassigned_dir, Harness, OnlineDir,
};
use super::*;

// --- watch-folder registration ------------------------------------------

fn online_row(id: i64, path: String) -> WbMovieRow {
    row(|r| {
        r.movie_id = id;
        r.movie_name = "v.mp4".to_string();
        r.movie_path = path;
    })
}

fn video_id_of(h: &Harness, path: &str) -> String {
    h.db.writer
        .lock()
        .unwrap()
        .query_row("SELECT id FROM videos WHERE file_path = ?1", [path], |r| {
            r.get(0)
        })
        .unwrap()
}

#[test]
fn only_parent_folders_of_newly_registered_rows_are_added() {
    let h = harness();
    let fresh = OnlineDir::new("a.mp4");
    let known = OnlineDir::new("b.mp4");
    let reconf = FakeWatchReconfigurer::default();

    // The "known" video is registered first (without registering
    // folders), so the second run sees it as Skipped.
    import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies: Ok(vec![online_row(1, known.file("b.mp4"))]),
            ..Default::default()
        },
        &no_op_ffmpeg(),
        &FakeWbImportNotifier::default(),
        &FakeCatalogNotifier::default(),
    );

    let summary = import_registering(
        &h,
        vec![
            online_row(1, known.file("b.mp4")),
            online_row(2, fresh.file("a.mp4")),
            online_row(3, r"Z:\gone\offline.mp4".to_string()),
        ],
        &reconf,
    );

    assert_eq!(summary.registered, 2);
    assert_eq!(summary.skipped, 1);
    // The offline row's folder is added too; the skipped row's is not.
    // `Z:` sorts after the temp dir's drive, so the order is stable.
    let expected = vec![fresh.folder(), r"Z:\gone".to_string()];
    assert_eq!(summary.watch_folders_added, expected);
    assert_eq!(stored_watch_folders(&h), expected);
    assert_eq!(*reconf.calls.lock().unwrap(), vec![expected]);
}

#[test]
fn offline_rows_parent_folders_are_added_and_nested_ones_collapse() {
    let h = harness();
    let online = OnlineDir::new("a.mp4");
    let reconf = FakeWatchReconfigurer::default();

    let summary = import_registering(
        &h,
        vec![
            online_row(1, online.file("a.mp4")),
            online_row(2, r"Z:\nas\x\b.mp4".to_string()),
            online_row(3, r"Z:\nas\c.mp4".to_string()),
            // Drive root: never a candidate, and must not swallow Z:\nas.
            online_row(4, r"Z:\top.mp4".to_string()),
        ],
        &reconf,
    );

    assert_eq!(summary.registered, 4);
    assert_eq!(
        summary.watch_folders_added,
        vec![online.folder(), r"Z:\nas".to_string()]
    );
    assert_eq!(stored_watch_folders(&h), summary.watch_folders_added);
    assert_eq!(reconf.calls.lock().unwrap().len(), 1);
}

#[test]
fn root_level_videos_are_reported_even_when_no_folder_is_added() {
    let h = harness();
    let reconf = FakeWatchReconfigurer::default();

    let summary = import_registering(&h, vec![online_row(1, r"Z:\top.mp4".to_string())], &reconf);

    assert_eq!(summary.registered, 1);
    assert!(summary.watch_folders_added.is_empty());
    assert_eq!(
        summary.watch_folders_skipped_roots,
        vec![r"Z:\".to_string()]
    );
    assert!(stored_watch_folders(&h).is_empty());
}

#[test]
fn root_level_videos_are_not_reported_when_opting_out() {
    let h = harness();
    let notifier = FakeWbImportNotifier::default();
    import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies: Ok(vec![online_row(1, r"Z:\top.mp4".to_string())]),
            ..Default::default()
        },
        &no_op_ffmpeg(),
        &notifier,
        &FakeCatalogNotifier::default(),
    );
    let summary = notifier.complete_calls.lock().unwrap()[0].clone();
    assert!(summary.watch_folders_skipped_roots.is_empty());
}

#[test]
fn an_offline_row_with_a_question_mark_left_in_its_folder_adds_nothing() {
    let h = harness();
    let reconf = FakeWatchReconfigurer::default();

    let summary = import_registering(
        &h,
        vec![
            online_row(1, r"Z:\a?b\c.mp4".to_string()),
            online_row(2, r"Z:\ok\d?e.mp4".to_string()),
        ],
        &reconf,
    );

    // Only the file-name `?` leaves a usable parent folder.
    assert_eq!(summary.watch_folders_added, vec![r"Z:\ok".to_string()]);
    assert_eq!(stored_watch_folders(&h), vec![r"Z:\ok".to_string()]);
}

#[test]
fn skipped_rows_folders_are_not_collected_on_reimport() {
    let h = harness();
    let reconf = FakeWatchReconfigurer::default();
    let rows = || vec![online_row(1, r"Z:\nas\a.mp4".to_string())];

    let first = import_registering(&h, rows(), &reconf);
    assert_eq!(first.watch_folders_added, vec![r"Z:\nas".to_string()]);

    // The user removes the watch folder; a re-import only sees a skipped
    // row and so must not bring it back.
    {
        let conn = h.db.writer.lock().unwrap();
        queries::set_watch_folders(&conn, &[]).unwrap();
    }
    let second = import_registering(&h, rows(), &reconf);

    assert_eq!(second.skipped, 1);
    assert!(second.watch_folders_added.is_empty());
    assert!(stored_watch_folders(&h).is_empty());
    assert_eq!(reconf.calls.lock().unwrap().len(), 1);
}

#[test]
fn opting_out_adds_no_offline_folder_and_does_not_reconfigure() {
    let h = harness();
    let notifier = FakeWbImportNotifier::default();
    import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies: Ok(vec![online_row(1, r"Z:\nas\a.mp4".to_string())]),
            ..Default::default()
        },
        &no_op_ffmpeg(),
        &notifier,
        &FakeCatalogNotifier::default(),
    );
    let summary = notifier.complete_calls.lock().unwrap()[0].clone();
    assert_eq!(summary.registered, 1);
    assert!(summary.watch_folders_added.is_empty());
    assert!(stored_watch_folders(&h).is_empty());
}

#[test]
fn folders_already_covered_by_a_watch_folder_are_not_added() {
    let h = harness();
    let dir = OnlineDir::new("a.mp4");
    let parent = dir
        .dir
        .path()
        .parent()
        .unwrap()
        .to_string_lossy()
        .to_string();
    {
        let conn = h.db.writer.lock().unwrap();
        queries::set_watch_folders(&conn, std::slice::from_ref(&parent)).unwrap();
    }
    let reconf = FakeWatchReconfigurer::default();

    let summary = import_registering(&h, vec![online_row(1, dir.file("a.mp4"))], &reconf);

    assert!(summary.watch_folders_added.is_empty());
    assert_eq!(stored_watch_folders(&h), vec![parent]);
    assert!(
        reconf.calls.lock().unwrap().is_empty(),
        "nothing was added, so the watchers must not be restarted"
    );
}

#[test]
fn opting_out_adds_nothing() {
    let h = harness();
    let dir = OnlineDir::new("a.mp4");
    let notifier = FakeWbImportNotifier::default();
    import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies: Ok(vec![online_row(1, dir.file("a.mp4"))]),
            ..Default::default()
        },
        &no_op_ffmpeg(),
        &notifier,
        &FakeCatalogNotifier::default(),
    );
    let summary = notifier.complete_calls.lock().unwrap()[0].clone();
    assert_eq!(summary.registered, 1);
    assert!(summary.watch_folders_added.is_empty());
    assert!(stored_watch_folders(&h).is_empty());
}

#[test]
fn reimporting_does_not_add_the_folder_twice_or_reconfigure_again() {
    let h = harness();
    let dir = OnlineDir::new("a.mp4");
    let reconf = FakeWatchReconfigurer::default();
    let rows = || vec![online_row(1, dir.file("a.mp4"))];

    import_registering(&h, rows(), &reconf);
    let second = import_registering(&h, rows(), &reconf);

    assert!(second.watch_folders_added.is_empty());
    assert_eq!(stored_watch_folders(&h), vec![dir.folder()]);
    assert_eq!(reconf.calls.lock().unwrap().len(), 1);
}

/// Registering folders is a convenience: a failed save must neither abort
/// the import nor be reported as added.
#[test]
fn a_failed_watch_folder_save_does_not_stop_the_import() {
    let h = harness();
    let dir = OnlineDir::new("a.mp4");
    {
        let conn = h.db.writer.lock().unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_watch_folders BEFORE INSERT ON settings
                 WHEN NEW.key = 'watch_folders'
                 BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
        )
        .unwrap();
    }
    let reconf = FakeWatchReconfigurer::default();

    let summary = import_registering(&h, vec![online_row(1, dir.file("a.mp4"))], &reconf);

    assert_eq!(summary.registered, 1);
    assert!(summary.watch_folders_added.is_empty());
    assert!(stored_watch_folders(&h).is_empty());
    assert!(reconf.calls.lock().unwrap().is_empty());
}

/// The legacy thumbnail of an offline video is linked after the folders
/// are registered, so it must land in the new folder's directory rather
/// than `_unassigned/` (where it would then be unreadable).
#[test]
fn legacy_thumbnails_are_linked_into_the_newly_added_folder() {
    let h = harness();
    let dir = OnlineDir::new("real.mp4");
    let offline_path = dir.file("missing.mp4");
    std::fs::write(
        h.paths.thumbnail_folder.join("Off.mp4.#2f6f1cc0.jpg"),
        b"fake jpg",
    )
    .unwrap();
    let offline = row(|r| {
        r.movie_id = 2;
        r.movie_path = offline_path.clone();
        r.hash = "2f6f1cc0".to_string();
    });

    let summary = import_registering(
        &h,
        vec![online_row(1, dir.file("real.mp4")), offline],
        &FakeWatchReconfigurer::default(),
    );
    assert_eq!(summary.thumbnails_linked, 1);

    let id = video_id_of(&h, &offline_path);
    let dest = crate::thumbnail::paths::video_thumbnail_dir(
        &h.paths.thumbnails_dir,
        &[dir.folder()],
        &offline_path,
    );
    for i in 0..THUMBNAILS_PER_VIDEO {
        assert!(dest.join(format!("{id}_{i}.webp")).exists(), "slot {i}");
    }
    assert!(!unassigned_dir(&h.paths.thumbnails_dir)
        .join(format!("{id}_0.webp"))
        .exists());
}

/// A video registered before the folder was added (here an offline one
/// whose legacy thumbnail was linked by an earlier import) has its files
/// in `_unassigned/`; adding the folder must carry them along.
#[test]
fn thumbnails_of_previously_registered_videos_follow_the_new_folder() {
    let h = harness();
    let dir = OnlineDir::new("real.mp4");
    let offline_path = dir.file("missing.mp4");
    std::fs::write(
        h.paths.thumbnail_folder.join("Off.mp4.#2f6f1cc0.jpg"),
        b"fake jpg",
    )
    .unwrap();
    let offline = || {
        row(|r| {
            r.movie_id = 2;
            r.movie_path = offline_path.clone();
            r.hash = "2f6f1cc0".to_string();
        })
    };

    import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies: Ok(vec![offline()]),
            ..Default::default()
        },
        &no_op_ffmpeg(),
        &FakeWbImportNotifier::default(),
        &FakeCatalogNotifier::default(),
    );
    let id = video_id_of(&h, &offline_path);
    let unassigned = unassigned_dir(&h.paths.thumbnails_dir);
    assert!(unassigned.join(format!("{id}_0.webp")).exists());

    let summary = import_registering(
        &h,
        vec![online_row(1, dir.file("real.mp4")), offline()],
        &FakeWatchReconfigurer::default(),
    );
    assert_eq!(summary.skipped, 1);

    let dest = crate::thumbnail::paths::video_thumbnail_dir(
        &h.paths.thumbnails_dir,
        &[dir.folder()],
        &offline_path,
    );
    for i in 0..THUMBNAILS_PER_VIDEO {
        assert!(dest.join(format!("{id}_{i}.webp")).exists(), "slot {i}");
        assert!(!unassigned.join(format!("{id}_{i}.webp")).exists());
    }
}
