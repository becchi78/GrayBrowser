//! The basic import: counts, tags and ratings, repeat imports, a failed
//! read, stage order, the committed fixtures and the summary counts.

use super::common::{harness, import_all, import_and_summarize, no_op_ffmpeg, row};
use super::*;

#[test]
fn imports_online_and_offline_rows_and_reports_expected_counts() {
    let h = harness();

    // Online: a real temp file, movie_path points at it.
    let online_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(online_file.path(), b"hello wb import").unwrap();
    let online_row = row(|r| {
        r.movie_id = 1;
        r.movie_name = "online.mp4".to_string();
        r.movie_path = online_file.path().to_string_lossy().to_string();
        r.score = 3;
    });

    // Offline: a path that does not exist on this machine.
    let offline_row = row(|r| {
        r.movie_id = 2;
        r.movie_name = "offline.mp4".to_string();
        r.movie_path = "Z:\\this\\path\\does\\not\\exist\\offline.mp4".to_string();
        r.score = 12; // clamp-triggering
    });

    // Invalid datetime: must be skipped without aborting the batch.
    let invalid_row = row(|r| {
        r.movie_id = 3;
        r.movie_name = "invalid.mp4".to_string();
        r.movie_path = "Z:\\invalid.mp4".to_string();
        r.regist_date = "not-a-date".to_string();
    });

    let wb_source = FakeWbSourceAdapter {
        movies: Ok(vec![online_row, offline_row, invalid_row]),
        ..Default::default()
    };
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

    let complete_calls = notifier.complete_calls.lock().unwrap();
    assert_eq!(
        complete_calls.len(),
        1,
        "notify_complete must fire exactly once"
    );
    let summary = &complete_calls[0];

    assert_eq!(
        summary.registered, 2,
        "the online and offline rows both import"
    );
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.clamped_scores, 1, "only the score=12 row clamps");
    assert_eq!(catalog_notifier.calls(), 1);

    // The offline row must land as the documented placeholder.
    let conn = h.db.writer.lock().unwrap();
    let (status, quick_hash, mtime, file_size): (String, String, i64, i64) = conn
        .query_row(
            "SELECT status, quick_hash, mtime, file_size FROM videos \
             WHERE file_path = 'Z:\\this\\path\\does\\not\\exist\\offline.mp4'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(status, "offline");
    assert_eq!(quick_hash, "");
    assert_eq!(mtime, 0);
    assert_eq!(file_size, 0);

    // The online row must have a real quick_hash/mtime/file_size.
    let (status, quick_hash, file_size): (String, String, i64) = conn
        .query_row(
            "SELECT status, quick_hash, file_size FROM videos WHERE file_name = 'online.mp4'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(status, "online");
    assert_ne!(quick_hash, "", "an online row must get a real quick_hash");
    assert_eq!(file_size, "hello wb import".len() as i64);

    // The invalid-datetime row must never have reached the DB at all.
    let invalid_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM videos WHERE file_path = 'Z:\\invalid.mp4'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(invalid_count, 0);
}

/// `source_tag_count` must reflect the raw `.wb` source data
/// (`gb_core::wb_import::count_source_tags`) independent of
/// `tags_assigned` -- summing every row's tags regardless of whether
/// that row's video was inserted (and its tags actually written) or
/// skipped (already registered, tags left untouched).
#[test]
fn source_tag_count_reflects_every_rows_raw_tag_data_not_just_assigned_ones() {
    let h = harness();
    let wb_source = FakeWbSourceAdapter {
        movies: Ok(vec![
            row(|r| {
                r.movie_id = 1;
                r.movie_path = "Z:\\tagged.mp4".to_string();
                r.tag = "foo\nbar".to_string();
            }),
            row(|r| {
                r.movie_id = 2;
                r.movie_path = "Z:\\untagged.mp4".to_string();
                r.tag = String::new();
            }),
        ]),
        ..Default::default()
    };
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
    assert_eq!(summary.tags_assigned, 2);
    assert_eq!(summary.source_tag_count, 2);
}

/// The "every row already registered" edge case: a repeat import writes
/// no new `video_tags` (`tags_assigned == 0`), but `source_tag_count`
/// still reports the source data's real tag count -- proving
/// `tags_assigned == 0` here is *not* "the source had no tags", which is
/// exactly the ambiguity `source_tag_count` exists to resolve for the
/// frontend.
#[test]
fn source_tag_count_stays_nonzero_even_when_a_repeat_import_assigns_no_new_tags() {
    let h = harness();
    let wb_source = FakeWbSourceAdapter {
        movies: Ok(vec![row(|r| {
            r.movie_path = "Z:\\repeat_tagged.mp4".to_string();
            r.tag = "foo".to_string();
        })]),
        ..Default::default()
    };
    let ffmpeg = no_op_ffmpeg();

    import_all(
        &h.db,
        &h.paths,
        &wb_source,
        &ffmpeg,
        &FakeWbImportNotifier::default(),
        &FakeCatalogNotifier::default(),
    );

    let notifier_second = FakeWbImportNotifier::default();
    import_all(
        &h.db,
        &h.paths,
        &wb_source,
        &ffmpeg,
        &notifier_second,
        &FakeCatalogNotifier::default(),
    );
    let second_summary = notifier_second.complete_calls.lock().unwrap()[0].clone();
    assert_eq!(
        second_summary.tags_assigned, 0,
        "the repeat run's already-registered row writes no new video_tags"
    );
    assert_eq!(
        second_summary.source_tag_count, 1,
        "source_tag_count reflects the raw .wb data, not what got (re-)written"
    );
}

/// Star-only tags become a rating (and never a tag); mixed/hollow-star
/// tags stay tags. A repeat import (all rows Skipped) counts nothing.
#[test]
fn star_only_tags_become_ratings_and_are_counted_for_inserted_rows_only() {
    let h = harness();
    let wb_source = FakeWbSourceAdapter {
        movies: Ok(vec![
            row(|r| {
                r.movie_id = 1;
                r.movie_path = "Z:\\star3.mp4".to_string();
                r.tag = "★★★\nfoo".to_string();
                r.score = 1;
            }),
            row(|r| {
                r.movie_id = 2;
                r.movie_path = "Z:\\mixed.mp4".to_string();
                r.tag = "☆\n★お気に入り".to_string();
                r.score = 4;
            }),
            row(|r| {
                r.movie_id = 3;
                r.movie_path = "Z:\\star_only.mp4".to_string();
                r.tag = "★★★★★★★".to_string();
                r.score = 0;
            }),
        ]),
        ..Default::default()
    };
    let ffmpeg = no_op_ffmpeg();
    let notifier = FakeWbImportNotifier::default();

    import_all(
        &h.db,
        &h.paths,
        &wb_source,
        &ffmpeg,
        &notifier,
        &FakeCatalogNotifier::default(),
    );

    let summary = notifier.complete_calls.lock().unwrap()[0].clone();
    assert_eq!(summary.registered, 3);
    assert_eq!(summary.ratings_from_star_tags, 2);
    // foo + ☆ + ★お気に入り; the star-only tags are not source tags.
    assert_eq!(summary.source_tag_count, 3);
    assert_eq!(summary.tags_assigned, 3);

    {
        let conn = h.db.writer.lock().unwrap();
        let rating_of = |name: &str| -> i64 {
            conn.query_row(
                "SELECT rating FROM videos WHERE file_path = ?1",
                [name],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(rating_of(r"Z:\star3.mp4"), 3, "star tag beats score=1");
        assert_eq!(
            rating_of(r"Z:\mixed.mp4"),
            4,
            "no star-only tag: score wins"
        );
        assert_eq!(rating_of(r"Z:\star_only.mp4"), 5, "7 stars cap at 5");

        let star_tags: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tags WHERE name LIKE '★%' AND name NOT LIKE '%お%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(star_tags, 0, "star-only tags are not registered as tags");
        let tag_names: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tags WHERE name IN ('foo', '☆', '★お気に入り')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tag_names, 3);
    }

    let notifier_second = FakeWbImportNotifier::default();
    import_all(
        &h.db,
        &h.paths,
        &wb_source,
        &ffmpeg,
        &notifier_second,
        &FakeCatalogNotifier::default(),
    );
    let second = notifier_second.complete_calls.lock().unwrap()[0].clone();
    assert_eq!(second.skipped, 3);
    assert_eq!(second.ratings_from_star_tags, 0);
}

#[test]
fn a_repeat_import_of_the_same_source_is_fully_skipped() {
    let h = harness();
    let wb_source = FakeWbSourceAdapter {
        movies: Ok(vec![row(|r| {
            r.movie_path = "Z:\\repeat.mp4".to_string();
        })]),
        ..Default::default()
    };
    let ffmpeg = no_op_ffmpeg();

    let notifier_first = FakeWbImportNotifier::default();
    let catalog_notifier_first = FakeCatalogNotifier::default();
    import_all(
        &h.db,
        &h.paths,
        &wb_source,
        &ffmpeg,
        &notifier_first,
        &catalog_notifier_first,
    );
    assert_eq!(
        notifier_first.complete_calls.lock().unwrap()[0].registered,
        1
    );
    assert_eq!(notifier_first.complete_calls.lock().unwrap()[0].skipped, 0);

    let notifier_second = FakeWbImportNotifier::default();
    let catalog_notifier_second = FakeCatalogNotifier::default();
    import_all(
        &h.db,
        &h.paths,
        &wb_source,
        &ffmpeg,
        &notifier_second,
        &catalog_notifier_second,
    );
    let second_summary = notifier_second.complete_calls.lock().unwrap()[0].clone();
    assert_eq!(
        second_summary.registered, 0,
        "the second run must register nothing new"
    );
    assert_eq!(
        second_summary.skipped, 1,
        "the second run must skip the already-imported row"
    );

    let conn = h.db.writer.lock().unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        count, 1,
        "no duplicate row must be created by the repeat import"
    );
}

#[test]
fn read_movies_failure_calls_notify_failed_instead_of_notify_complete() {
    let h = harness();
    let wb_source = FakeWbSourceAdapter::default(); // default `movies` is Err
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

    assert!(
        notifier.complete_calls.lock().unwrap().is_empty(),
        "no summary exists to report -- the import never got past read_movies"
    );
    assert!(notifier.progress_calls.lock().unwrap().is_empty());
    let failed_calls = notifier.failed_calls.lock().unwrap();
    assert_eq!(
        failed_calls.len(),
        1,
        "notify_failed must fire exactly once so the frontend learns the import died"
    );
    assert!(
        !failed_calls[0].is_empty(),
        "the failure reason should be a non-empty human-readable message"
    );
    assert_eq!(catalog_notifier.calls(), 0);
    assert_eq!(
        *notifier.stage_calls.lock().unwrap(),
        vec![WbImportStage::Reading],
        "a failed read must not announce the later phases"
    );
}

#[test]
fn stages_are_announced_in_order() {
    let h = harness();
    let wb_source = FakeWbSourceAdapter {
        movies: Ok(vec![row(|r| {
            r.movie_path = "Z:\\stage\\order.mp4".to_string();
        })]),
        ..Default::default()
    };
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

    assert_eq!(
        *notifier.stage_calls.lock().unwrap(),
        vec![
            WbImportStage::Reading,
            WbImportStage::Registering,
            WbImportStage::Linking
        ]
    );
    assert_eq!(notifier.complete_calls.lock().unwrap().len(), 1);
}

/// Stands in for the import log and every notifier at once, so the order
/// of the calls that end an import can be read off one list.
#[derive(Default)]
struct EndRecorder {
    events: Mutex<Vec<&'static str>>,
}

impl EndRecorder {
    fn push(&self, event: &'static str) {
        self.events.lock().unwrap().push(event);
    }
}

impl WbImportLog for EndRecorder {
    fn append(&self, _: &str) {
        self.push("log");
    }
    fn location(&self) -> String {
        String::new()
    }
}

impl WbImportNotifier for EndRecorder {
    fn notify_progress(&self, _: WbImportStage, _: u32, _: Option<u32>) {}
    fn notify_complete(&self, _: &WbImportSummary) {
        self.push("complete");
    }
    fn notify_failed(&self, _: &str) {
        self.push("failed");
    }
}

impl CatalogNotifier for EndRecorder {
    fn notify_changed(&self) {
        self.push("changed");
    }
}

impl WatchReconfigurer for EndRecorder {
    fn reconfigure(&self, _: &[String]) {
        self.push("reconfigure");
    }
}

/// The log is complete before the screen hears of the result, the list is
/// re-fetched after it, and the watchers are restarted last.
#[test]
fn an_import_ends_with_the_log_then_complete_then_changed_then_reconfigure() {
    let h = harness();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("w.mp4");
    std::fs::write(&path, b"online bytes").unwrap();
    let recorder = EndRecorder::default();

    super::import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies: Ok(vec![row(|r| {
                r.movie_name = "w.mp4".to_string();
                r.movie_path = path.to_string_lossy().to_string();
            })]),
            ..Default::default()
        },
        &no_op_ffmpeg(),
        &recorder,
        &recorder,
        Some(&recorder),
        &FakeDirLister::default(),
        &FakeFolderProbe::default(),
        &FakeVolumeLabelResolver::default(),
        &FakeWbImportClock::default(),
        &recorder,
    );

    let events = recorder.events.lock().unwrap();
    assert!(
        events.ends_with(&["log", "complete", "changed", "reconfigure"]),
        "{events:?}"
    );
    for once in ["complete", "changed", "reconfigure"] {
        assert_eq!(
            events.iter().filter(|e| **e == once).count(),
            1,
            "{once}: {events:?}"
        );
    }
}

/// Integration coverage against the committed, anonymized real-shaped
/// fixture (`tests/fixtures/wb/sample_small.wb`, 50 rows -- see
/// `tests/fixtures/wb/README.md`). Reads it once via the production
/// `RealWbSourceAdapter`, then feeds those same rows through
/// `import_all` twice via a `FakeWbSourceAdapter` (no real `.wb` I/O in
/// the pipeline itself) to check counts at a realistic scale/shape and
/// idempotency on a repeat run.
#[test]
fn sample_fixture_imports_with_expected_counts_and_is_idempotent_on_rerun() {
    use crate::adapters::wb_source::RealWbSourceAdapter;
    use gb_core::ports::wb_source::WbSourceAdapter as _;

    let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/wb/sample_small.wb");
    let real_adapter = RealWbSourceAdapter::open(&fixture_path)
        .expect("should open the committed sample fixture read-only");
    let rows = real_adapter
        .read_movies()
        .expect("read_movies should succeed against the fixture");
    assert_eq!(
        rows.len(),
        50,
        "sample_small.wb is documented as exactly 50 rows"
    );

    let expected_clamped = count_clamped_scores(&rows) as u32;
    let expected_valid_rows = rows
        .iter()
        .filter(|r| gb_core::wb_import::wb_row_to_import_candidate(r).is_ok())
        .count() as u32;

    let h = harness();
    let wb_source = FakeWbSourceAdapter {
        movies: Ok(rows),
        ..Default::default()
    };
    let ffmpeg = no_op_ffmpeg();

    let notifier_first = FakeWbImportNotifier::default();
    let catalog_notifier_first = FakeCatalogNotifier::default();
    import_all(
        &h.db,
        &h.paths,
        &wb_source,
        &ffmpeg,
        &notifier_first,
        &catalog_notifier_first,
    );
    let first_summary = notifier_first.complete_calls.lock().unwrap()[0].clone();
    assert_eq!(first_summary.clamped_scores, expected_clamped);
    assert_eq!(
        first_summary.registered + first_summary.skipped,
        expected_valid_rows,
        "every row that produced a valid ImportCandidate must be either \
         registered or skipped"
    );
    assert_eq!(
        first_summary.skipped, 0,
        "a fresh DB has nothing to skip yet"
    );

    let notifier_second = FakeWbImportNotifier::default();
    let catalog_notifier_second = FakeCatalogNotifier::default();
    import_all(
        &h.db,
        &h.paths,
        &wb_source,
        &ffmpeg,
        &notifier_second,
        &catalog_notifier_second,
    );
    let second_summary = notifier_second.complete_calls.lock().unwrap()[0].clone();
    assert_eq!(
        second_summary.registered, 0,
        "re-importing the same fixture must register nothing new"
    );
    assert_eq!(second_summary.skipped, expected_valid_rows);
}

/// Runs the real pipeline logic against the developer's actual `.wb`
/// library (`tests/fixtures/wb/local/default_20110504.wb`, gitignored
/// -- never committed, absent in CI/other machines) to get real-scale
/// counts. Mirrors `tests/wb_source_local_fixture.rs`'s safety
/// conventions exactly:
/// - Skips itself (prints a message, does not fail) when the local
///   fixture is absent -- this must never fail a normal `cargo test`.
/// - Opens the `.wb` strictly read-only via `RealWbSourceAdapter`;
///   nothing in this test (or `import_all`) ever writes back to it.
/// - Only counts are asserted or printed to stderr -- never real paths,
///   tags, filenames, or any other cell content.
///
/// The legacy thumbnail folder is deliberately an empty temp directory
/// (the developer doesn't have the real legacy thumbnail folder at
/// hand), so this exercises the "3072 real online/offline rows, but no
/// legacy thumbnails available at all" shape. Every DB write lands in a
/// throwaway temp DB (`harness()`'s `init_temp_db`), never the real
/// `app.db` -- this test cannot touch a real user's database.
#[test]
fn real_local_fixture_import_reports_realistic_counts() {
    use crate::adapters::wb_source::RealWbSourceAdapter;
    use gb_core::ports::wb_source::WbSourceAdapter as _;

    let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/wb/local/default_20110504.wb");
    if !fixture_path.exists() {
        eprintln!(
            "skipping: local .wb fixture not present at {} (never committed, developer-only)",
            fixture_path.display()
        );
        return;
    }

    let real_adapter =
        RealWbSourceAdapter::open(&fixture_path).expect("should open the real .wb read-only");
    let rows = real_adapter
        .read_movies()
        .expect("read_movies should succeed against the real .wb");
    assert_eq!(rows.len(), 3072, "movie row count");

    let expected_clamped = count_clamped_scores(&rows) as u32;
    assert_eq!(
        expected_clamped, 89,
        "known confirmed value, same as wb_source_local_fixture.rs"
    );
    let expected_valid_rows = rows
        .iter()
        .filter(|r| gb_core::wb_import::wb_row_to_import_candidate(r).is_ok())
        .count() as u32;

    // Fresh temp DB + an empty temp "legacy thumbnail folder" + an empty
    // temp thumbnails_dir -- none of this touches the real app.db or a
    // real legacy thumbnail folder.
    let h = harness();
    let wb_source = FakeWbSourceAdapter {
        movies: Ok(rows),
        ..Default::default()
    };
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

    // Counts only -- never content.
    eprintln!(
        "wb import (real local .wb, empty legacy thumbnail folder) summary: \
         registered={} skipped={} clamped_scores={} tags_assigned={} \
         thumbnails_linked={} thumbnails_failed={} thumbnails_unmatched={}",
        summary.registered,
        summary.skipped,
        summary.clamped_scores,
        summary.tags_assigned,
        summary.thumbnails_linked,
        summary.thumbnails_failed,
        summary.thumbnails_unmatched
    );

    assert_eq!(
        summary.registered + summary.skipped,
        expected_valid_rows,
        "every row that produced a valid ImportCandidate must be either \
         registered or skipped -- none silently dropped"
    );
    assert_eq!(
        summary.skipped, 0,
        "a fresh temp DB has nothing to skip yet"
    );
    assert_eq!(summary.clamped_scores, expected_clamped);
    assert_eq!(
        summary.thumbnails_linked, 0,
        "the legacy thumbnail folder is an empty temp dir -- nothing to link"
    );
    assert_eq!(
        summary.thumbnails_failed, 0,
        "no legacy thumbnail files exist to fail converting"
    );
    assert_eq!(
        summary.thumbnails_unmatched, 0,
        "no legacy thumbnail files exist, so none can be unmatched either"
    );
}

/// Every row lands in exactly one bucket, so the buckets add up to the
/// number of rows in the `.wb`.
fn assert_counts_add_up(summary: &WbImportSummary) {
    assert_eq!(
        summary.total,
        summary.registered + summary.skipped + summary.skipped_invalid_name + summary.failed_rows,
        "total must equal registered + skipped + skipped_invalid_name + failed_rows: \
         {summary:?}"
    );
    assert_eq!(
        summary.registered,
        summary.registered_online + summary.registered_offline,
        "registered must equal registered_online + registered_offline: {summary:?}"
    );
}

#[test]
fn counts_split_registered_rows_into_online_and_offline() {
    let h = harness();
    let online_dir = tempfile::tempdir().unwrap();
    let mut rows = Vec::new();
    for i in 0..2 {
        let path = online_dir.path().join(format!("on{i}.mp4"));
        std::fs::write(&path, b"online bytes").unwrap();
        rows.push(row(|r| {
            r.movie_id = i + 1;
            r.movie_name = format!("on{i}.mp4");
            r.movie_path = path.to_string_lossy().to_string();
        }));
    }
    for i in 0..3 {
        rows.push(row(|r| {
            r.movie_id = 10 + i;
            r.movie_name = format!("off{i}.mp4");
            r.movie_path = format!("Z:\\gone\\off{i}.mp4");
        }));
    }

    let summary = import_and_summarize(&h, rows);

    assert_eq!(summary.total, 5);
    assert_eq!(summary.registered, 5);
    assert_eq!(summary.registered_online, 2);
    assert_eq!(summary.registered_offline, 3);
    assert_eq!(summary.failed_rows, 0);
    assert_counts_add_up(&summary);
}

#[test]
fn a_row_with_an_unparseable_date_counts_as_failed() {
    let h = harness();
    let summary = import_and_summarize(
        &h,
        vec![
            row(|r| {
                r.movie_id = 1;
                r.movie_path = "Z:\\ok.mp4".to_string();
            }),
            row(|r| {
                r.movie_id = 2;
                r.movie_path = "Z:\\bad_date.mp4".to_string();
                r.regist_date = "not-a-date".to_string();
            }),
        ],
    );

    assert_eq!(summary.total, 2);
    assert_eq!(summary.registered, 1);
    assert_eq!(summary.failed_rows, 1);
    assert_counts_add_up(&summary);
}

/// A trigger that aborts the insert for one path stands in for any
/// database write failure; the other row must still import.
#[test]
fn a_row_whose_database_write_fails_counts_as_failed() {
    let h = harness();
    {
        let conn = h.db.writer.lock().unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_one_insert BEFORE INSERT ON videos \
             WHEN NEW.file_path = 'Z:\\db_fail.mp4' \
             BEGIN SELECT RAISE(ABORT, 'forced failure'); END;",
        )
        .unwrap();
    }

    let summary = import_and_summarize(
        &h,
        vec![
            row(|r| {
                r.movie_id = 1;
                r.movie_path = "Z:\\ok.mp4".to_string();
            }),
            row(|r| {
                r.movie_id = 2;
                r.movie_path = "Z:\\db_fail.mp4".to_string();
            }),
        ],
    );

    assert_eq!(summary.total, 2);
    assert_eq!(summary.registered, 1);
    assert_eq!(summary.failed_rows, 1);
    assert_counts_add_up(&summary);
}

#[test]
fn counts_add_up_with_every_kind_of_row_and_on_a_repeat_import() {
    let h = harness();
    let online_dir = tempfile::tempdir().unwrap();
    let online_path = online_dir.path().join("online.mp4");
    std::fs::write(&online_path, b"online bytes").unwrap();
    let rows = vec![
        row(|r| {
            r.movie_id = 1;
            r.movie_name = "online.mp4".to_string();
            r.movie_path = online_path.to_string_lossy().to_string();
        }),
        row(|r| {
            r.movie_id = 2;
            r.movie_name = "offline.mp4".to_string();
            r.movie_path = "Z:\\gone\\offline.mp4".to_string();
        }),
        row(|r| {
            r.movie_id = 3;
            r.movie_name = "bad\u{2460}.mp4".to_string();
            r.movie_path = "Z:\\gone\\bad\u{2460}.mp4".to_string();
        }),
        row(|r| {
            r.movie_id = 4;
            r.movie_path = "Z:\\bad_date.mp4".to_string();
            r.last_date = "not-a-date".to_string();
        }),
    ];

    let first = import_and_summarize(&h, rows.clone());
    assert_eq!(first.total, 4);
    assert_eq!(first.registered_online, 1);
    assert_eq!(first.registered_offline, 1);
    assert_eq!(first.skipped_invalid_name, 1);
    assert_eq!(first.failed_rows, 1);
    assert_eq!(first.skipped, 0);
    assert_counts_add_up(&first);

    let second = import_and_summarize(&h, rows);
    assert_eq!(second.total, 4);
    assert_eq!(second.registered, 0);
    assert_eq!(second.registered_online, 0);
    assert_eq!(second.registered_offline, 0);
    assert_eq!(second.skipped, 2, "the two registered rows are skipped");
    assert_eq!(second.skipped_invalid_name, 1);
    assert_eq!(second.failed_rows, 1);
    assert_counts_add_up(&second);
}
