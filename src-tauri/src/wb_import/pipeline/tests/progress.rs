//! Progress reported while reading, registering and linking.

use super::common::{
    harness, import_all, named_row, no_op_ffmpeg, registering_progress, row, stage_progress,
};
use super::*;

/// Rows whose date cannot be parsed fail before they reach
/// `notify_row_progress`.
fn bad_date_row(id: i64) -> WbMovieRow {
    let mut r = named_row(id, "Z:\\gone", &format!("bad-date-{id}.mp4"));
    r.regist_date = "not a date".to_string();
    r
}

fn import_rows(rows: Vec<WbMovieRow>) -> FakeWbImportNotifier {
    let h = harness();
    let notifier = FakeWbImportNotifier::default();
    import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies: Ok(rows),
            ..Default::default()
        },
        &no_op_ffmpeg(),
        &notifier,
        &FakeCatalogNotifier::default(),
    );
    notifier
}

#[test]
fn reading_reports_the_row_count_once_it_is_known() {
    let notifier = import_rows(vec![
        named_row(1, "Z:\\gone", "a.mp4"),
        named_row(2, "Z:\\gone", "b.mp4"),
        named_row(3, "Z:\\gone", "c.mp4"),
    ]);

    assert_eq!(
        stage_progress(&notifier, WbImportStage::Reading),
        vec![(3, Some(3))]
    );
}

#[test]
fn a_failed_read_reports_no_progress_at_all() {
    let h = harness();
    let notifier = FakeWbImportNotifier::default();
    import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter::default(),
        &no_op_ffmpeg(),
        &notifier,
        &FakeCatalogNotifier::default(),
    );

    assert!(notifier.progress_calls.lock().unwrap().is_empty());
}

#[test]
fn registering_reports_the_final_count_once_when_the_last_row_reports_it() {
    let notifier = import_rows(vec![
        named_row(1, "Z:\\gone", "a.mp4"),
        named_row(2, "Z:\\gone", "b.mp4"),
    ]);

    assert_eq!(registering_progress(&notifier), vec![(2, Some(2))]);
}

#[test]
fn registering_reaches_the_total_even_when_the_last_row_has_a_bad_date() {
    let notifier = import_rows(vec![
        named_row(1, "Z:\\gone", "a.mp4"),
        bad_date_row(2),
        bad_date_row(3),
    ]);

    assert_eq!(registering_progress(&notifier), vec![(3, Some(3))]);
}

#[test]
fn registering_does_not_repeat_a_count_already_sent_at_the_interval() {
    let rows: Vec<WbMovieRow> = (1..=PROGRESS_NOTIFY_INTERVAL as i64)
        .map(|i| named_row(i, "Z:\\gone", &format!("v{i}.mp4")))
        .collect();
    let n = rows.len() as u32;

    let notifier = import_rows(rows);

    assert_eq!(registering_progress(&notifier), vec![(n, Some(n))]);
}

#[test]
fn registering_throttles_to_the_interval_then_sends_the_final_count() {
    // Interval + 1 rows, the last one unreportable: 50 is sent by the
    // interval, 51 only by the final notification.
    let mut rows: Vec<WbMovieRow> = (1..=PROGRESS_NOTIFY_INTERVAL as i64)
        .map(|i| named_row(i, "Z:\\gone", &format!("v{i}.mp4")))
        .collect();
    rows.push(bad_date_row(1000));
    let total = rows.len() as u32;

    let notifier = import_rows(rows);

    assert_eq!(
        registering_progress(&notifier),
        vec![
            (PROGRESS_NOTIFY_INTERVAL, Some(total)),
            (total, Some(total))
        ]
    );
}

#[test]
fn an_empty_wb_reports_zero_for_reading_and_registering() {
    let notifier = import_rows(Vec::new());

    assert_eq!(
        stage_progress(&notifier, WbImportStage::Reading),
        vec![(0, Some(0))]
    );
    assert_eq!(registering_progress(&notifier), vec![(0, Some(0))]);
    assert_eq!(
        stage_progress(&notifier, WbImportStage::Linking),
        vec![(0, Some(0))]
    );
}

#[test]
fn stages_and_progress_arrive_in_order() {
    let notifier = import_rows(vec![named_row(1, "Z:\\gone", "a.mp4")]);

    assert_eq!(
        *notifier.event_order.lock().unwrap(),
        vec![
            WbImportEvent::Stage(WbImportStage::Reading),
            WbImportEvent::Progress(WbImportStage::Reading, 1, Some(1)),
            WbImportEvent::Stage(WbImportStage::Registering),
            WbImportEvent::Progress(WbImportStage::Registering, 1, Some(1)),
            WbImportEvent::Stage(WbImportStage::Linking),
            WbImportEvent::Progress(WbImportStage::Linking, 0, Some(0)),
        ]
    );
}

#[test]
fn linking_reports_zero_of_the_matched_count_then_the_final_count() {
    let h = harness();
    std::fs::write(
        h.paths.thumbnail_folder.join("a.mp4.#aaaa1111.jpg"),
        b"fake jpg",
    )
    .unwrap();
    std::fs::write(
        h.paths.thumbnail_folder.join("b.mp4.#bbbb2222.jpg"),
        b"fake jpg",
    )
    .unwrap();
    // Matches no registered video, so it must not count toward the total.
    std::fs::write(
        h.paths.thumbnail_folder.join("c.mp4.#cccc3333.jpg"),
        b"fake jpg",
    )
    .unwrap();
    let rows = vec![
        row(|r| {
            r.movie_id = 1;
            r.movie_name = "a.mp4".to_string();
            r.movie_path = "Z:\\gone\\a.mp4".to_string();
            r.hash = "aaaa1111".to_string();
        }),
        row(|r| {
            r.movie_id = 2;
            r.movie_name = "b.mp4".to_string();
            r.movie_path = "Z:\\gone\\b.mp4".to_string();
            r.hash = "bbbb2222".to_string();
        }),
    ];
    let notifier = FakeWbImportNotifier::default();

    import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies: Ok(rows),
            ..Default::default()
        },
        &no_op_ffmpeg(),
        &notifier,
        &FakeCatalogNotifier::default(),
    );

    assert_eq!(
        stage_progress(&notifier, WbImportStage::Linking),
        vec![(0, Some(2)), (2, Some(2))]
    );
    let summary = notifier.complete_calls.lock().unwrap()[0].clone();
    assert_eq!(summary.thumbnails_linked, 2);
    assert_eq!(summary.thumbnails_unmatched, 1);
}

#[test]
fn link_progress_is_throttled_and_ends_at_the_total() {
    let h = harness();
    let count = PROGRESS_NOTIFY_INTERVAL * 2 + 1;
    let movies: Vec<(String, String, String)> = (0..count)
        .map(|i| {
            let hash = format!("{i:08x}");
            std::fs::write(
                h.paths
                    .thumbnail_folder
                    .join(format!("m{i}.mp4.#{hash}.jpg")),
                b"fake jpg",
            )
            .unwrap();
            (format!("video-{i}"), hash, format!("Z:\\gone\\m{i}.mp4"))
        })
        .collect();
    let calls = Mutex::new(Vec::new());

    link_thumbnails(
        &h.paths.thumbnail_folder,
        &h.paths.thumbnails_dir,
        &[],
        &movies,
        &[],
        &HashSet::new(),
        &HashSet::new(),
        &no_op_ffmpeg(),
        None,
        |processed, total| calls.lock().unwrap().push((processed, total)),
    );

    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            (0, count),
            (PROGRESS_NOTIFY_INTERVAL, count),
            (PROGRESS_NOTIFY_INTERVAL * 2, count),
            (count, count)
        ]
    );
}
