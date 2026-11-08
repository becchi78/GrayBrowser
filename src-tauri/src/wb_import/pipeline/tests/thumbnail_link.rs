//! Linking legacy JPG thumbnails by hash: what is linked, recorded as
//! unmatched or left to online videos, and parallel linking matching a
//! serial run.

use super::common::{
    harness, image_size, import_all, jpeg_bytes, legacy_movies, link_with_layout, no_op_ffmpeg,
    recorded_unmatched, row, six_by_one_layout, split_ffmpeg, unassigned_dir, Harness,
};
use super::*;

#[test]
fn links_matching_thumbnails_and_counts_unmatched_and_failed() {
    let h = harness();

    let matched_row = row(|r| {
        r.movie_id = 1;
        r.movie_path = "Z:\\matched.mp4".to_string();
        r.hash = "1e5e0fbf".to_string();
    });
    let failing_row = row(|r| {
        r.movie_id = 2;
        r.movie_path = "Z:\\failing.mp4".to_string();
        r.hash = "aaaaaaaa".to_string();
    });
    let wb_source = FakeWbSourceAdapter {
        movies: Ok(vec![matched_row, failing_row]),
        ..Default::default()
    };

    std::fs::write(
        h.paths.thumbnail_folder.join("MyVideo.mp4.#1e5e0fbf.jpg"),
        b"fake jpg",
    )
    .unwrap();
    std::fs::write(
        h.paths.thumbnail_folder.join("MyVideo.mp4.#aaaaaaaa.jpg"),
        b"fake jpg",
    )
    .unwrap();
    std::fs::write(
        h.paths.thumbnail_folder.join("no_match_at_all.jpg"),
        b"fake jpg",
    )
    .unwrap();

    let ffmpeg = FakeFfmpegAdapter {
        convert_result: Box::new(|src_path| {
            if src_path.to_string_lossy().contains("aaaaaaaa") {
                Err(FfmpegError::NonZeroExit {
                    status: 1,
                    stderr: "simulated conversion failure".into(),
                })
            } else {
                Ok(())
            }
        }),
        ..Default::default()
    };
    let notifier = FakeWbImportNotifier::default();
    let catalog_notifier = FakeCatalogNotifier::default();

    import_all(
        &h.db,
        &h.paths,
        &wb_source,
        &ffmpeg,
        &notifier,
        &catalog_notifier,
    );

    let summary = notifier.complete_calls.lock().unwrap()[0].clone();
    assert_eq!(summary.registered, 2);
    assert_eq!(
        summary.thumbnails_linked, 1,
        "only the matching, succeeding hash links"
    );
    assert_eq!(
        summary.thumbnails_failed, 1,
        "the failing conversion must be counted"
    );
    assert_eq!(
        summary.thumbnails_unmatched, 1,
        "the filename with no matching hash must be counted"
    );

    let conn = h.db.writer.lock().unwrap();
    let matched_id: String = conn
        .query_row(
            "SELECT id FROM videos WHERE file_path = 'Z:\\matched.mp4'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let video_dir = unassigned_dir(&h.paths.thumbnails_dir);
    for i in 0..THUMBNAILS_PER_VIDEO {
        assert!(
            video_dir.join(format!("{matched_id}_{i}.webp")).exists(),
            "slot {i} must exist for the matched video"
        );
    }
    assert!(!video_dir
        .join(format!("{matched_id}.wb_legacy.webp.tmp"))
        .exists());

    let failing_id: String = conn
        .query_row(
            "SELECT id FROM videos WHERE file_path = 'Z:\\failing.mp4'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for i in 0..THUMBNAILS_PER_VIDEO {
        assert!(
            !video_dir.join(format!("{failing_id}_{i}.webp")).exists(),
            "slot {i} must not exist for the failing video"
        );
    }
    assert!(!video_dir
        .join(format!("{failing_id}.wb_legacy.webp.tmp"))
        .exists());

    let ready = |id: &str| -> i64 {
        conn.query_row(
            "SELECT thumbnail_ready FROM videos WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(
        ready(&matched_id),
        1,
        "a linked video must be flagged ready"
    );
    assert_eq!(
        ready(&failing_id),
        0,
        "a conversion failure must leave thumbnail_ready unset"
    );
}

fn import_once(h: &Harness, wb_source: &FakeWbSourceAdapter, ffmpeg: &FakeFfmpegAdapter) {
    import_all(
        &h.db,
        &h.paths,
        wb_source,
        ffmpeg,
        &FakeWbImportNotifier::default(),
        &FakeCatalogNotifier::default(),
    );
}

#[test]
fn records_unmatched_thumbnails_with_kind_and_layout_and_a_reimport_replaces_them() {
    let h = harness();
    let wb_source = FakeWbSourceAdapter {
        movies: Ok(vec![row(|r| {
            r.movie_id = 1;
            r.movie_path = "Z:\\linked.mp4".to_string();
            r.hash = "1e5e0fbf".to_string();
        })]),
        profile_values: Ok(vec!["200x150x6x1".to_string()]),
        ..Default::default()
    };
    for name in [
        "linked.mp4.#1e5e0fbf.jpg",
        "gone.mp4.#.jpg",
        "orphan.mp4.#deadbeef.jpg",
    ] {
        std::fs::write(h.paths.thumbnail_folder.join(name), b"fake jpg").unwrap();
    }
    let ffmpeg = FakeFfmpegAdapter::default();

    import_once(&h, &wb_source, &ffmpeg);

    let folder = h.paths.thumbnail_folder.to_string_lossy().to_string();
    let expected = |name: &str, kind: &str| queries::UnmatchedThumbnailRow {
        folder_path: folder.clone(),
        file_name: name.to_string(),
        kind: kind.to_string(),
        thum_layout: Some("200x150x6x1".to_string()),
    };
    assert_eq!(
        recorded_unmatched(&h),
        vec![
            expected("gone.mp4.#.jpg", "empty_hash"),
            expected("orphan.mp4.#deadbeef.jpg", "no_matching_video"),
        ],
        "the linked file must not be recorded"
    );

    // One of the two is dealt with outside the app; the repeat import must
    // rebuild the record, not add to it.
    std::fs::remove_file(h.paths.thumbnail_folder.join("orphan.mp4.#deadbeef.jpg")).unwrap();
    import_once(&h, &wb_source, &ffmpeg);
    assert_eq!(
        recorded_unmatched(&h),
        vec![expected("gone.mp4.#.jpg", "empty_hash")]
    );
}

#[test]
fn records_no_layout_when_the_wb_states_none() {
    let h = harness();
    let wb_source = FakeWbSourceAdapter {
        movies: Ok(vec![row(|r| {
            r.movie_path = "Z:\\a.mp4".to_string();
        })]),
        // Default: reading the profile fails, so there is no layout.
        ..Default::default()
    };
    std::fs::write(h.paths.thumbnail_folder.join("a.mp4.#.jpg"), b"fake jpg").unwrap();

    import_once(&h, &wb_source, &FakeFfmpegAdapter::default());

    let rows = recorded_unmatched(&h);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].thum_layout, None);
}

#[test]
fn an_import_that_cannot_list_the_folder_keeps_the_previous_record() {
    let h = harness();
    let wb_source = FakeWbSourceAdapter {
        movies: Ok(vec![row(|r| {
            r.movie_path = "Z:\\a.mp4".to_string();
        })]),
        ..Default::default()
    };
    std::fs::write(h.paths.thumbnail_folder.join("a.mp4.#.jpg"), b"fake jpg").unwrap();
    let ffmpeg = FakeFfmpegAdapter::default();
    import_once(&h, &wb_source, &ffmpeg);
    assert_eq!(recorded_unmatched(&h).len(), 1);

    let missing = WbImportPaths {
        wb_path: h.paths.wb_path.clone(),
        thumbnail_folder: h.paths.thumbnail_folder.join("does-not-exist"),
        thumbnails_dir: h.paths.thumbnails_dir.clone(),
    };
    import_all(
        &h.db,
        &missing,
        &wb_source,
        &ffmpeg,
        &FakeWbImportNotifier::default(),
        &FakeCatalogNotifier::default(),
    );
    assert_eq!(
        recorded_unmatched(&h).len(),
        1,
        "an unlistable folder is not evidence that nothing is unmatched"
    );
}

/// Only an offline row whose legacy JPG was linked gets `thumbnail_ready`;
/// online rows (thumbnail comes from the real video) and offline rows with
/// no JPG stay 0. A re-import must not disturb the already-set flag.
#[test]
fn only_linked_offline_rows_become_thumbnail_ready_and_reimport_keeps_it() {
    let h = harness();

    let online_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(online_file.path(), b"hello ready flag").unwrap();
    let wb_source = FakeWbSourceAdapter {
        movies: Ok(vec![
            row(|r| {
                r.movie_id = 1;
                r.movie_path = online_file.path().to_string_lossy().to_string();
                r.hash = "1e5e0fbf".to_string();
            }),
            row(|r| {
                r.movie_id = 2;
                r.movie_path = "Z:\\linked.mp4".to_string();
                r.hash = "2f6f1cc0".to_string();
            }),
            row(|r| {
                r.movie_id = 3;
                r.movie_path = "Z:\\no_jpg.mp4".to_string();
                r.hash = "3a7a2dd1".to_string();
            }),
        ]),
        ..Default::default()
    };
    for name in ["On.mp4.#1e5e0fbf.jpg", "Off.mp4.#2f6f1cc0.jpg"] {
        std::fs::write(h.paths.thumbnail_folder.join(name), b"fake jpg").unwrap();
    }
    let ffmpeg = no_op_ffmpeg();

    let ready_of = |path_like: &str| -> i64 {
        let conn = h.db.writer.lock().unwrap();
        conn.query_row(
            "SELECT thumbnail_ready FROM videos WHERE file_path LIKE ?1",
            [path_like],
            |r| r.get(0),
        )
        .unwrap()
    };

    for pass in 0..2 {
        import_all(
            &h.db,
            &h.paths,
            &wb_source,
            &ffmpeg,
            &FakeWbImportNotifier::default(),
            &FakeCatalogNotifier::default(),
        );
        assert_eq!(ready_of("Z:\\linked.mp4"), 1, "pass {pass}");
        assert_eq!(ready_of("Z:\\no_jpg.mp4"), 0, "pass {pass}");
        assert_eq!(
            ready_of(&online_file.path().to_string_lossy()),
            0,
            "pass {pass}: online rows are left to the thumbnail worker"
        );
    }
}

/// Follow-up to the test above: that test only exercises
/// `ffmpeg.convert_image_to_webp` itself failing (the early-return
/// branch before the copy loop). This test instead exercises the copy
/// loop's own fail-fast + best-effort-cleanup branch: conversion
/// succeeds, but `fs::copy` fails partway through duplicating the
/// converted image into the `THUMBNAILS_PER_VIDEO` slots.
///
/// Calls `link_thumbnails` directly rather than going through
/// `import_all`: this function never touches the DB, so a hand-picked
/// `video_id` can be used to pre-create a directory at slot 2's
/// destination path ahead of time -- `fs::copy` always fails when its
/// destination is an existing directory, which is what forces the copy
/// loop to fail specifically at slot 2, after slots 0 and 1 have already
/// been written for real.
#[test]
fn copy_failure_partway_through_the_slot_loop_cleans_up_and_stops() {
    let h = harness();
    let video_id = "fixed-video-id-for-copy-failure-test";
    let video_file_path = "Z:\\video-for-copy-failure-test.mp4";
    let video_dir = unassigned_dir(&h.paths.thumbnails_dir);
    std::fs::create_dir_all(&video_dir).unwrap();

    std::fs::write(
        h.paths.thumbnail_folder.join("MyVideo.mp4.#deadbeef.jpg"),
        b"fake jpg",
    )
    .unwrap();

    // Pre-create a directory at slot 2's destination so `fs::copy` fails
    // there specifically. Slots 0 and 1 must succeed first (their
    // destinations are untouched), proving the loop really got partway
    // through the six slots before hitting the failure.
    std::fs::create_dir(video_dir.join(format!("{video_id}_2.webp"))).unwrap();

    // Fail-fast side channel for slot 3 (the slot immediately after the
    // failure, so the very first one a broken/missing `break` would
    // reach): the best-effort cleanup pass below unconditionally
    // `fs::remove_file`s every slot 0..THUMBNAILS_PER_VIDEO regardless of
    // which slot actually failed, so merely asserting "slot 3's path
    // doesn't exist afterward" is true whether or not slot 3 was ever
    // copied to -- cleanup would delete it either way, making that
    // assertion alone unable to catch a missing `break`. To observe the
    // copy attempt itself (not just its cleaned-up-or-not-cleaned-up
    // aftermath), hard-link slot 3's destination to a marker file that
    // lives outside the six slot paths cleanup ever touches:
    // `fs::copy`'s Windows implementation overwrites an existing
    // destination in place (verified empirically: it does NOT
    // delete-and-recreate, so a hard link to that file sees the new
    // content too), while `fs::remove_file` on the slot path afterward
    // only detaches that one directory entry -- the marker keeps
    // whatever content the copy left behind, cleaned up or not.
    let marker_path = video_dir.join("fail_fast_side_channel_marker.bin");
    std::fs::write(&marker_path, b"untouched-original-marker-content").unwrap();
    std::fs::hard_link(&marker_path, video_dir.join(format!("{video_id}_3.webp"))).unwrap();

    let ffmpeg = no_op_ffmpeg();

    let outcome = link_thumbnails(
        &h.paths.thumbnail_folder,
        &h.paths.thumbnails_dir,
        &[],
        &[(
            video_id.to_string(),
            "deadbeef".to_string(),
            video_file_path.to_string(),
        )],
        &[],
        &HashSet::new(),
        &HashSet::new(),
        &ffmpeg,
        None,
        |_, _| {},
    );

    assert_eq!(outcome.skipped_online, 0);
    assert_eq!(outcome.linked, 0, "a copy failure must not count as linked");
    assert!(
        outcome.linked_video_ids.is_empty(),
        "a copy failure must not be reported as linked, or its thumbnail_ready would be set"
    );
    assert_eq!(
        outcome.failed, 1,
        "a copy failure partway through the slot loop must still count as failed"
    );
    assert_eq!(outcome.unmatched, 0);

    // Slots 0 and 1 were written successfully, then must have been
    // cleaned back up by the best-effort cleanup pass.
    for i in 0..2 {
        assert!(
            !video_dir.join(format!("{video_id}_{i}.webp")).exists(),
            "slot {i} must have been cleaned up after the later copy failure at slot 2"
        );
    }

    // Slot 2's destination is still present, but only because it is the
    // directory the test pre-created as the failure trigger: cleanup
    // uses `fs::remove_file`, which cannot remove a directory, so
    // best-effort cleanup leaves it untouched by design (not a bug --
    // the test's own `tempfile::TempDir` removes it on drop).
    assert!(video_dir.join(format!("{video_id}_2.webp")).is_dir());

    // Slots 3..THUMBNAILS_PER_VIDEO's own destination paths must be gone
    // too (removed by the same best-effort cleanup pass as slots 0/1 --
    // this alone doesn't prove they were *never attempted*, see above).
    for i in 3..THUMBNAILS_PER_VIDEO {
        assert!(
            !video_dir.join(format!("{video_id}_{i}.webp")).exists(),
            "slot {i}'s path must not remain after cleanup"
        );
    }

    // The actual fail-fast proof: the marker file hard-linked to slot 3
    // must still read back its original content. If the missing `break`
    // regression were reintroduced, the loop would reach slot 3 after
    // slot 2's failure, `fs::copy` would overwrite the marker's content
    // in place, and this would read back the copy's ("fake webp bytes")
    // content instead -- even though slot 3's own path gets cleaned up
    // either way (checked above) and can't tell the two cases apart by
    // itself.
    assert_eq!(
        std::fs::read(&marker_path).unwrap(),
        b"untouched-original-marker-content",
        "slot 3 must never have been attempted after the slot-2 failure -- the loop must \
         stop immediately, not merely clean up whatever it went on to write"
    );

    // The shared scratch tmp file must not survive the failure either.
    assert!(!video_dir
        .join(format!("{video_id}.wb_legacy.webp.tmp"))
        .exists());
}

/// An *online* row's thumbnail must come from the real video file (the
/// existing background thumbnail pipeline, kicked off by
/// `run_wb_import` after `import_all` returns -- not exercised by this
/// test, which calls `import_all` directly), never from the legacy
/// JPG, even when a hash-matching legacy JPG exists. It must therefore
/// never reach `ffmpeg.convert_image_to_webp` at all, and its legacy
/// JPG must be counted as `thumbnails_skipped_online` -- not as
/// `thumbnails_unmatched` (which means "needs manual attention") and not
/// as `thumbnails_linked`.
#[test]
fn online_rows_are_excluded_from_legacy_thumbnail_matching() {
    let h = harness();

    let online_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(online_file.path(), b"hello online thumbnail exclusion").unwrap();
    let online_row = row(|r| {
        r.movie_id = 1;
        r.movie_path = online_file.path().to_string_lossy().to_string();
        r.hash = "1e5e0fbf".to_string();
    });

    let wb_source = FakeWbSourceAdapter {
        movies: Ok(vec![online_row]),
        ..Default::default()
    };

    std::fs::write(
        h.paths.thumbnail_folder.join("MyVideo.mp4.#1e5e0fbf.jpg"),
        b"fake jpg",
    )
    .unwrap();

    let ffmpeg = no_op_ffmpeg();
    let notifier = FakeWbImportNotifier::default();
    let catalog_notifier = FakeCatalogNotifier::default();

    import_all(
        &h.db,
        &h.paths,
        &wb_source,
        &ffmpeg,
        &notifier,
        &catalog_notifier,
    );

    let summary = notifier.complete_calls.lock().unwrap()[0].clone();
    assert_eq!(summary.registered, 1);
    assert_eq!(summary.thumbnails_linked, 0);
    assert_eq!(summary.thumbnails_failed, 0);
    assert_eq!(
        summary.thumbnails_unmatched, 0,
        "an online row's legacy JPG is intentionally skipped, not unmatched"
    );
    assert_eq!(summary.thumbnails_skipped_online, 1);

    assert!(
        ffmpeg.calls.lock().unwrap().is_empty(),
        "ffmpeg must never be invoked for an online row's legacy thumbnail"
    );
}

/// Re-importing the same `.wb` skips every already-registered row, but an
/// online row's legacy JPG must still be counted as skipped-online rather
/// than falling into `thumbnails_unmatched`.
#[test]
fn reimport_keeps_online_legacy_thumbnail_out_of_unmatched() {
    let h = harness();

    let online_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(online_file.path(), b"hello reimport online").unwrap();
    let online_row = row(|r| {
        r.movie_id = 1;
        r.movie_path = online_file.path().to_string_lossy().to_string();
        r.hash = "1e5e0fbf".to_string();
    });
    let wb_source = FakeWbSourceAdapter {
        movies: Ok(vec![online_row]),
        ..Default::default()
    };
    std::fs::write(
        h.paths.thumbnail_folder.join("MyVideo.mp4.#1e5e0fbf.jpg"),
        b"fake jpg",
    )
    .unwrap();

    let ffmpeg = no_op_ffmpeg();
    for pass in 0..2 {
        let notifier = FakeWbImportNotifier::default();
        let catalog_notifier = FakeCatalogNotifier::default();
        import_all(
            &h.db,
            &h.paths,
            &wb_source,
            &ffmpeg,
            &notifier,
            &catalog_notifier,
        );
        let summary = notifier.complete_calls.lock().unwrap()[0].clone();
        assert_eq!(summary.registered, 1 - pass, "pass {pass}");
        assert_eq!(summary.skipped, pass, "pass {pass}");
        assert_eq!(summary.thumbnails_unmatched, 0, "pass {pass}");
        assert_eq!(summary.thumbnails_skipped_online, 1, "pass {pass}");
    }
    assert!(ffmpeg.calls.lock().unwrap().is_empty());
}

fn offline_row(id: i64, name: &str, hash: &str) -> WbMovieRow {
    row(|r| {
        r.movie_id = id;
        r.movie_name = name.to_string();
        r.movie_path = format!("Z:\\no\\such\\{name}");
        r.hash = hash.to_string();
    })
}

fn run_import(h: &Harness, rows: Vec<WbMovieRow>, ffmpeg: &FakeFfmpegAdapter) -> WbImportSummary {
    let wb_source = FakeWbSourceAdapter {
        movies: Ok(rows),
        ..Default::default()
    };
    let notifier = FakeWbImportNotifier::default();
    let catalog_notifier = FakeCatalogNotifier::default();
    import_all(
        &h.db,
        &h.paths,
        &wb_source,
        ffmpeg,
        &notifier,
        &catalog_notifier,
    );
    let summary = notifier.complete_calls.lock().unwrap()[0].clone();
    summary
}

/// A re-import skips every already-registered offline row; their legacy
/// JPGs were linked by the first import and must not be reported as
/// needing manual attention, while the first import's result is unchanged.
#[test]
fn reimport_keeps_registered_offline_thumbnail_out_of_unmatched() {
    let h = harness();
    std::fs::write(
        h.paths.thumbnail_folder.join("Off.mp4.#2f6f1cc0.jpg"),
        b"fake jpg",
    )
    .unwrap();
    let ffmpeg = no_op_ffmpeg();
    let rows = || vec![offline_row(1, "off.mp4", "2f6f1cc0")];

    let first = run_import(&h, rows(), &ffmpeg);
    assert_eq!(first.registered, 1);
    assert_eq!(first.thumbnails_linked, 1);
    assert_eq!(first.thumbnails_unmatched, 0);

    let second = run_import(&h, rows(), &ffmpeg);
    assert_eq!(second.registered, 0);
    assert_eq!(second.skipped, 1);
    assert_eq!(second.thumbnails_linked, 0);
    assert_eq!(second.thumbnails_failed, 0);
    assert_eq!(second.thumbnails_unmatched, 0);
}

/// A JPG that belongs to no row is still reported on a re-import.
#[test]
fn reimport_still_counts_truly_unmatched_thumbnail() {
    let h = harness();
    for name in ["Off.mp4.#2f6f1cc0.jpg", "Other.mp4.#deadbeef.jpg"] {
        std::fs::write(h.paths.thumbnail_folder.join(name), b"fake jpg").unwrap();
    }
    let ffmpeg = no_op_ffmpeg();
    let rows = || vec![offline_row(1, "off.mp4", "2f6f1cc0")];

    assert_eq!(run_import(&h, rows(), &ffmpeg).thumbnails_unmatched, 1);
    let second = run_import(&h, rows(), &ffmpeg);
    assert_eq!(second.skipped, 1);
    assert_eq!(second.thumbnails_unmatched, 1);
}

/// A newly registered row that shares its hash with an already-registered
/// offline row still takes the JPG.
#[test]
fn reimport_new_row_sharing_hash_with_registered_offline_row_still_links() {
    let h = harness();
    std::fs::write(
        h.paths.thumbnail_folder.join("Off.mp4.#2f6f1cc0.jpg"),
        b"fake jpg",
    )
    .unwrap();
    let ffmpeg = no_op_ffmpeg();

    run_import(&h, vec![offline_row(1, "a.mp4", "2f6f1cc0")], &ffmpeg);
    let second = run_import(
        &h,
        vec![
            offline_row(1, "a.mp4", "2f6f1cc0"),
            offline_row(2, "b.mp4", "2f6f1cc0"),
        ],
        &ffmpeg,
    );
    assert_eq!(second.registered, 1);
    assert_eq!(second.skipped, 1);
    assert_eq!(second.thumbnails_linked, 1);
    assert_eq!(second.thumbnails_unmatched, 0);
}

/// Online, offline and unrelated legacy JPGs in one folder must land in
/// three separate buckets.
#[test]
fn mixed_online_offline_and_unrelated_thumbnails_are_counted_separately() {
    let h = harness();

    let online_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(online_file.path(), b"hello mixed thumbnails").unwrap();
    let online_row = row(|r| {
        r.movie_id = 1;
        r.movie_path = online_file.path().to_string_lossy().to_string();
        r.hash = "1e5e0fbf".to_string();
    });
    let offline_row = row(|r| {
        r.movie_id = 2;
        r.movie_name = "offline.mp4".to_string();
        r.movie_path = "Z:\\no\\such\\offline.mp4".to_string();
        r.hash = "2f6f1cc0".to_string();
    });

    let wb_source = FakeWbSourceAdapter {
        movies: Ok(vec![online_row, offline_row]),
        ..Default::default()
    };

    for name in [
        "On.mp4.#1e5e0fbf.jpg",
        "Off.mp4.#2f6f1cc0.jpg",
        "Other.mp4.#deadbeef.jpg",
    ] {
        std::fs::write(h.paths.thumbnail_folder.join(name), b"fake jpg").unwrap();
    }

    let ffmpeg = no_op_ffmpeg();
    let notifier = FakeWbImportNotifier::default();
    let catalog_notifier = FakeCatalogNotifier::default();

    import_all(
        &h.db,
        &h.paths,
        &wb_source,
        &ffmpeg,
        &notifier,
        &catalog_notifier,
    );

    let summary = notifier.complete_calls.lock().unwrap()[0].clone();
    assert_eq!(summary.registered, 2);
    assert_eq!(summary.thumbnails_linked, 1);
    assert_eq!(summary.thumbnails_failed, 0);
    assert_eq!(summary.thumbnails_skipped_online, 1);
    assert_eq!(summary.thumbnails_unmatched, 1);
    assert_eq!(
        ffmpeg.calls.lock().unwrap().len(),
        1,
        "only the offline row's JPG is converted"
    );
}

#[test]
fn parallel_linking_keeps_the_order_counts_and_progress_of_a_serial_run() {
    let h = harness();
    let count = PROGRESS_NOTIFY_INTERVAL as usize * 3 + 7;
    let movies = legacy_movies(&h, count, |_| b"fake jpg".to_vec());
    // Every third JPG fails to convert.
    let ffmpeg = FakeFfmpegAdapter {
        convert_result: Box::new(|src| {
            let name = src.file_name().unwrap().to_string_lossy().into_owned();
            let index: usize = name[1..5].parse().unwrap();
            if index.is_multiple_of(3) {
                Err(FfmpegError::NonZeroExit {
                    status: 1,
                    stderr: "simulated".into(),
                })
            } else {
                Ok(())
            }
        }),
        ..Default::default()
    };
    let progress = Mutex::new(Vec::new());

    let outcome = link_thumbnails(
        &h.paths.thumbnail_folder,
        &h.paths.thumbnails_dir,
        &[],
        &movies,
        &[],
        &HashSet::new(),
        &HashSet::new(),
        &ffmpeg,
        None,
        |processed, total| progress.lock().unwrap().push((processed, total)),
    );

    // What a one-at-a-time run would have produced, in matching order.
    let hash_to_index: HashMap<&str, usize> = movies
        .iter()
        .enumerate()
        .map(|(i, (_, hash, _))| (hash.as_str(), i))
        .collect();
    let movie_pairs: Vec<(String, String, String)> = movies
        .iter()
        .map(|(id, hash, path)| {
            (
                id.clone(),
                hash.clone(),
                file_name_from_path(path).to_string(),
            )
        })
        .collect();
    let filenames = thumbnail_scan::list_filenames(&h.paths.thumbnail_folder).unwrap();
    let plan = match_thumbnail_files(&movie_pairs, &[], &filenames);
    let mut expected_linked = Vec::new();
    let mut expected_failed = Vec::new();
    for (video_id, filename) in &plan.matched {
        let hash = extract_thumbnail_hash(filename).unwrap();
        if hash_to_index[hash.as_str()].is_multiple_of(3) {
            expected_failed.push(video_id.clone());
        } else {
            expected_linked.push(video_id.clone());
        }
    }
    assert_eq!(outcome.linked_video_ids, expected_linked);
    let failed_ids: Vec<String> = outcome
        .failed_video_ids
        .iter()
        .map(|(id, _)| id.clone())
        .collect();
    assert_eq!(failed_ids, expected_failed);
    assert_eq!(outcome.linked as usize, expected_linked.len());
    assert_eq!(outcome.failed as usize, expected_failed.len());
    assert!(outcome
        .failed_video_ids
        .iter()
        .all(|(_, reason)| reason.starts_with("WebPへの変換に失敗")));

    let total = count as u32;
    let progress = progress.lock().unwrap().clone();
    let expected_progress: Vec<(u32, u32)> = [0, 50, 100, 150, total].map(|n| (n, total)).to_vec();
    assert_eq!(progress, expected_progress);

    // A failed video leaves no slot file, a linked one has all of them.
    let dir = unassigned_dir(&h.paths.thumbnails_dir);
    for (i, (video_id, _, _)) in movies.iter().enumerate() {
        for slot in 0..THUMBNAILS_PER_VIDEO {
            let exists = crate::thumbnail::paths::slot_path(&dir, video_id, slot).exists();
            assert_eq!(exists, !i.is_multiple_of(3), "video {i} slot {slot}");
        }
        assert!(!dir.join(format!("{video_id}.wb_legacy.webp.tmp")).exists());
    }
}

#[test]
fn a_failing_frame_cut_leaves_no_slot_files_and_stops_no_other_video() {
    let h = harness();
    let movies = legacy_movies(&h, 12, |_| jpeg_bytes(1200, 150));
    // Splitting every video fails.
    let ffmpeg = split_ffmpeg(image_size(1200, 150), Some(600));

    let outcome = link_with_layout(&h, &movies, &ffmpeg, Some(&six_by_one_layout()));

    assert_eq!((outcome.linked, outcome.failed), (0, 12));
    let dir = unassigned_dir(&h.paths.thumbnails_dir);
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
}

/// Two legacy JPGs with the same hash both match one video; with many
/// other videos around so several workers run, the outcome must be what a
/// serial run gives: the later file decides the slots.
fn duplicate_hash_run(first_fails: bool, check: impl FnOnce(&LinkOutcome, &Path)) {
    let h = harness();
    let movies = legacy_movies(&h, 20, |_| b"fake jpg".to_vec());
    let dup_hash = movies[5].1.clone();
    // Only the two `dup_` files match video-5.
    std::fs::remove_file(
        h.paths
            .thumbnail_folder
            .join(format!("m0005.mp4.#{dup_hash}.jpg")),
    )
    .unwrap();
    // `a`/`b` sort in this order and both carry the hash of video-5.
    std::fs::write(
        h.paths
            .thumbnail_folder
            .join(format!("dup_a.mp4.#{dup_hash}.jpg")),
        b"fake jpg",
    )
    .unwrap();
    std::fs::write(
        h.paths
            .thumbnail_folder
            .join(format!("dup_b.mp4.#{dup_hash}.jpg")),
        b"fake jpg",
    )
    .unwrap();
    let ffmpeg = FakeFfmpegAdapter {
        convert_result: Box::new(move |src| {
            let name = src.file_name().unwrap().to_string_lossy().into_owned();
            let fails = if name.starts_with("dup_a") {
                first_fails
            } else {
                name.starts_with("dup_b") && !first_fails
            };
            if fails {
                Err(FfmpegError::NonZeroExit {
                    status: 1,
                    stderr: "simulated".into(),
                })
            } else {
                Ok(())
            }
        }),
        ..Default::default()
    };
    let outcome = link_with_layout(&h, &movies, &ffmpeg, None);
    let dir = unassigned_dir(&h.paths.thumbnails_dir);
    check(&outcome, &dir);
}

#[test]
fn a_later_file_of_the_same_video_wins_as_in_a_serial_run() {
    let slots_exist = |dir: &Path| {
        (0..THUMBNAILS_PER_VIDEO)
            .map(|slot| crate::thumbnail::paths::slot_path(dir, "video-5", slot).exists())
            .collect::<Vec<_>>()
    };

    // a fails, b succeeds: the video ends up linked, with its slots.
    duplicate_hash_run(true, |outcome, dir| {
        assert_eq!((outcome.linked, outcome.failed), (20, 0));
        assert_eq!(slots_exist(dir), [true; THUMBNAILS_PER_VIDEO]);
    });

    // a succeeds, b fails to convert: the video is counted once, by its
    // last item (b): failed. A failed video keeps no slot files, so a's
    // slots are removed and disk, classification and the ready flag agree.
    duplicate_hash_run(false, |outcome, dir| {
        assert_eq!((outcome.linked, outcome.failed), (19, 1));
        assert_eq!(outcome.failed_video_ids.len(), 1);
        assert_eq!(outcome.failed_video_ids[0].0, "video-5");
        assert!(!outcome.linked_video_ids.contains(&"video-5".to_string()));
        assert_eq!(slots_exist(dir), [false; THUMBNAILS_PER_VIDEO]);
        assert!(crate::thumbnail::paths::slot_path(dir, "video-6", 0).exists());
    });
}

/// Two videos sharing one hash, each with a legacy JPG named after it.
fn shared_hash_run(fail_name: Option<&'static str>) -> (LinkOutcome, Vec<(String, String)>) {
    let h = harness();
    let hash = "abcdef01";
    for name in ["alpha", "beta"] {
        std::fs::write(
            h.paths.thumbnail_folder.join(format!("{name}.#{hash}.jpg")),
            b"fake jpg",
        )
        .unwrap();
    }
    let movies = vec![
        (
            "video-a".to_string(),
            hash.to_string(),
            r"Z:\gone\alpha.mp4".to_string(),
        ),
        (
            "video-b".to_string(),
            hash.to_string(),
            "Z:/gone/BETA.mp4".to_string(),
        ),
    ];
    let ffmpeg = FakeFfmpegAdapter {
        convert_result: Box::new(move |src| {
            let name = src.file_name().unwrap().to_string_lossy().into_owned();
            if fail_name.is_some_and(|f| name.starts_with(f)) {
                Err(FfmpegError::NonZeroExit {
                    status: 1,
                    stderr: "simulated".into(),
                })
            } else {
                Ok(())
            }
        }),
        ..Default::default()
    };
    let outcome = link_with_layout(&h, &movies, &ffmpeg, None);
    // (video the conversion was for, legacy JPG it read), sorted.
    let mut converted: Vec<(String, String)> = ffmpeg
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter_map(|c| match c {
            FakeCall::ConvertImageToWebp {
                src_path,
                output_tmp_path,
                ..
            } => {
                let tmp = output_tmp_path.file_name()?.to_string_lossy().into_owned();
                let video = tmp.strip_suffix(".wb_legacy.webp.tmp")?.to_string();
                let src = src_path.file_name()?.to_string_lossy().into_owned();
                Some((video, src))
            }
            _ => None,
        })
        .collect();
    converted.sort();
    (outcome, converted)
}

#[test]
fn videos_sharing_a_hash_are_counted_once_each_and_get_their_own_file() {
    let (outcome, converted) = shared_hash_run(None);

    assert_eq!(
        (outcome.linked, outcome.failed, outcome.unmatched),
        (2, 0, 0)
    );
    assert_eq!(outcome.linked_video_ids, ["video-a", "video-b"]);
    // Each video is converted from its own file, once; no cross pairing.
    assert_eq!(
        converted,
        [
            ("video-a".to_string(), "alpha.#abcdef01.jpg".to_string()),
            ("video-b".to_string(), "beta.#abcdef01.jpg".to_string()),
        ]
    );
}

#[test]
fn a_shared_hash_video_is_classified_by_its_own_file() {
    let (outcome, converted) = shared_hash_run(Some("beta"));

    assert_eq!(
        converted,
        [
            ("video-a".to_string(), "alpha.#abcdef01.jpg".to_string()),
            ("video-b".to_string(), "beta.#abcdef01.jpg".to_string()),
        ]
    );
    assert_eq!((outcome.linked, outcome.failed), (1, 1));
    assert_eq!(outcome.linked_video_ids, ["video-a"]);
    assert_eq!(outcome.failed_video_ids.len(), 1);
    assert_eq!(outcome.failed_video_ids[0].0, "video-b");
}

#[test]
fn link_progress_sends_nothing_when_the_folder_cannot_be_listed() {
    let h = harness();
    let calls = Mutex::new(Vec::new());

    let outcome = link_thumbnails(
        &h.paths.thumbnail_folder.join("does-not-exist"),
        &h.paths.thumbnails_dir,
        &[],
        &[],
        &[],
        &HashSet::new(),
        &HashSet::new(),
        &no_op_ffmpeg(),
        None,
        |processed, total| calls.lock().unwrap().push((processed, total)),
    );

    assert!(outcome.folder_error.is_some());
    assert!(calls.lock().unwrap().is_empty());
}

/// Two rows can share a thumbnail hash; if only one of them is refused the
/// JPG still belongs to the registered one and must be linked.
#[test]
fn a_jpg_shared_with_a_registered_row_is_still_linked() {
    let h = harness();
    let bad = row(|r| {
        r.movie_id = 1;
        r.movie_name = "dup\u{2460}.mp4".to_string();
        r.movie_path = "Z:\\gone\\dup\u{2460}.mp4".to_string();
        r.hash = "2f6f1cc0".to_string();
    });
    let good = row(|r| {
        r.movie_id = 2;
        r.movie_name = "dup.mp4".to_string();
        r.movie_path = "Z:\\gone\\dup.mp4".to_string();
        r.hash = "2f6f1cc0".to_string();
    });
    std::fs::write(
        h.paths.thumbnail_folder.join("dup.mp4.#2f6f1cc0.jpg"),
        b"fake jpg",
    )
    .unwrap();
    let notifier = FakeWbImportNotifier::default();

    import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies: Ok(vec![bad, good]),
            ..Default::default()
        },
        &no_op_ffmpeg(),
        &notifier,
        &FakeCatalogNotifier::default(),
    );

    let summary = notifier.complete_calls.lock().unwrap()[0].clone();
    assert_eq!(summary.skipped_invalid_name, 1);
    assert_eq!(summary.registered, 1);
    assert_eq!(summary.thumbnails_linked, 1);
    assert_eq!(summary.thumbnails_unmatched, 0);
}
