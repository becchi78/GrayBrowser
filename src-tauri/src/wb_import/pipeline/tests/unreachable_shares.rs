//! Rows on a network share that cannot be reached are registered
//! offline without touching the share.

use super::common::{harness, lines_starting_with, no_op_ffmpeg, row_at, status_of, Harness, DASH};
use super::*;

// ---- unreachable shares ----

fn import_with_probe<P: FolderProbe>(
    h: &Harness,
    rows: Vec<WbMovieRow>,
    lister: &FakeDirLister,
    probe: &P,
    import_log: &FakeWbImportLog,
) -> WbImportSummary {
    let notifier = FakeWbImportNotifier::default();
    super::import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies: Ok(rows),
            ..Default::default()
        },
        &no_op_ffmpeg(),
        &notifier,
        &FakeCatalogNotifier::default(),
        None::<&FakeWatchReconfigurer>,
        lister,
        probe,
        &FakeVolumeLabelResolver::default(),
        &FakeWbImportClock::default(),
        import_log,
    );
    let summary = notifier.complete_calls.lock().unwrap()[0].clone();
    summary
}

#[test]
fn rows_on_an_unreachable_share_are_registered_offline_and_the_share_is_probed_once() {
    let h = harness();
    let probe = FakeFolderProbe::with_unreachable(&[r"W:\"]);
    let import_log = FakeWbImportLog::default();
    let paths: Vec<String> = (0..5).map(|i| format!(r"W:\videos\v{i}.mp4")).collect();
    let rows = paths
        .iter()
        .enumerate()
        .map(|(i, p)| row_at(i as i64 + 1, p))
        .collect();

    let summary = import_with_probe(&h, rows, &FakeDirLister::default(), &probe, &import_log);

    assert_eq!(summary.registered_offline, 5);
    assert_eq!(summary.registered_online, 0);
    assert_eq!(probe.calls(), vec![r"W:\".to_string()]);
    for p in &paths {
        assert_eq!(status_of(&h, p), ("offline".into(), String::new(), 0, 0));
    }
    let text = import_log.text();
    assert_eq!(
        lines_starting_with(&text, "到達できない共有のため").len(),
        1,
        "{text}"
    );
    assert!(text.contains(r"この共有上の行のファイル確認を飛ばしました: W:\"));
}

#[test]
fn a_reachable_share_is_checked_as_before() {
    let h = harness();
    let online_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(online_file.path(), b"hello").unwrap();
    let path = online_file.path().to_string_lossy().to_string();
    let probe = FakeFolderProbe::default();
    let import_log = FakeWbImportLog::default();

    let summary = import_with_probe(
        &h,
        vec![row_at(1, &path)],
        &FakeDirLister::default(),
        &probe,
        &import_log,
    );

    assert_eq!(summary.registered_online, 1);
    assert_eq!(status_of(&h, &path).0, "online");
    assert_eq!(probe.calls().len(), 1);
    assert!(!import_log.text().contains("到達できない共有のため"));
}

#[test]
fn only_the_rows_of_the_unreachable_share_go_offline() {
    let h = harness();
    let online_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(online_file.path(), b"hello").unwrap();
    let online_path = online_file.path().to_string_lossy().to_string();
    let down_path = r"W:\videos\a.mp4";
    let probe = FakeFolderProbe::with_unreachable(&[r"W:\"]);

    let summary = import_with_probe(
        &h,
        vec![row_at(1, &online_path), row_at(2, down_path)],
        &FakeDirLister::default(),
        &probe,
        &FakeWbImportLog::default(),
    );

    assert_eq!(summary.registered_online, 1);
    assert_eq!(summary.registered_offline, 1);
    assert_eq!(status_of(&h, &online_path).0, "online");
    assert_eq!(status_of(&h, down_path).0, "offline");
    assert_eq!(probe.calls().len(), 2);
}

#[test]
fn a_missing_relative_path_is_registered_offline_without_a_probe() {
    let h = harness();
    let probe = FakeFolderProbe::default();

    let summary = import_with_probe(
        &h,
        vec![row_at(1, r"no-such-dir-for-test\relative.mp4")],
        &FakeDirLister::default(),
        &probe,
        &FakeWbImportLog::default(),
    );

    assert!(probe.calls().is_empty());
    assert_eq!(summary.registered_offline, 1);
}

#[test]
fn case_and_separator_variants_of_a_share_are_probed_once() {
    let h = harness();
    let probe = FakeFolderProbe::with_unreachable(&[r"W:\"]);

    let summary = import_with_probe(
        &h,
        vec![row_at(1, r"w:\a.mp4"), row_at(2, "W:/b.mp4")],
        &FakeDirLister::default(),
        &probe,
        &FakeWbImportLog::default(),
    );

    assert_eq!(probe.calls(), vec![r"W:\".to_string()]);
    assert_eq!(summary.registered_offline, 2);
}

#[test]
fn a_panicking_probe_does_not_abort_the_import_and_counts_as_reachable() {
    struct PanicOnW;
    impl FolderProbe for PanicOnW {
        fn is_reachable(&self, folder: &str) -> bool {
            if folder == r"W:\" {
                panic!("probe failure for the test");
            }
            false
        }
    }
    let h = harness();
    let notifier = FakeWbImportNotifier::default();

    // X:\ answers "down" normally; W:\ panics and falls back to the
    // per-file check (the file is missing, so it still ends up offline).
    super::import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies: Ok(vec![row_at(1, r"W:\a.mp4"), row_at(2, r"X:\b.mp4")]),
            ..Default::default()
        },
        &no_op_ffmpeg(),
        &notifier,
        &FakeCatalogNotifier::default(),
        None::<&FakeWatchReconfigurer>,
        &FakeDirLister::default(),
        &PanicOnW,
        &FakeVolumeLabelResolver::default(),
        &FakeWbImportClock::default(),
        &FakeWbImportLog::default(),
    );

    assert_eq!(notifier.complete_calls.lock().unwrap().len(), 1);
    let shares = ShareReachability::probe(&PanicOnW, [r"W:\a.mp4", r"X:\b.mp4"].into_iter());
    assert!(shares.is_reachable_path(r"W:\a.mp4"));
    assert!(!shares.is_reachable_path(r"X:\b.mp4"));
}

#[test]
fn a_question_mark_row_on_an_unreachable_share_is_not_listed_or_restored() {
    let h = harness();
    let real = format!("a{DASH}.mp4");
    let probe = FakeFolderProbe::with_unreachable(&[r"W:\"]);
    // Would restore the row if the folder were listed.
    let lister = FakeDirLister::default().with_dir(r"W:\gone", &[&real]);
    let stored = r"W:\gone\a?.mp4";

    let summary = import_with_probe(
        &h,
        vec![row_at(1, stored)],
        &lister,
        &probe,
        &FakeWbImportLog::default(),
    );

    assert_eq!(summary.registered_offline, 1);
    assert!(lister.calls().is_empty());
    assert_eq!(status_of(&h, stored).0, "offline");
}

#[test]
fn every_share_is_probed_at_the_same_time() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// Each probe waits until every other probe is also in flight, so the
    /// test passes only if the probes overlap. The deadline turns a
    /// serial implementation into a failure instead of a hang.
    struct RendezvousProbe {
        expected: usize,
        in_flight: AtomicUsize,
        timed_out: AtomicBool,
    }
    impl FolderProbe for RendezvousProbe {
        fn is_reachable(&self, _folder: &str) -> bool {
            self.in_flight.fetch_add(1, Ordering::SeqCst);
            let deadline = Instant::now() + Duration::from_secs(10);
            while self.in_flight.load(Ordering::SeqCst) < self.expected {
                if Instant::now() > deadline {
                    self.timed_out.store(true, Ordering::SeqCst);
                    return false;
                }
                std::thread::yield_now();
            }
            true
        }
    }

    let h = harness();
    let probe = RendezvousProbe {
        expected: 3,
        in_flight: AtomicUsize::new(0),
        timed_out: AtomicBool::new(false),
    };

    import_with_probe(
        &h,
        vec![
            row_at(1, r"W:\a.mp4"),
            row_at(2, r"X:\b.mp4"),
            row_at(3, r"\\nas\share\c.mp4"),
            row_at(4, r"W:\d.mp4"),
        ],
        &FakeDirLister::default(),
        &probe,
        &FakeWbImportLog::default(),
    );

    assert!(!probe.timed_out.load(Ordering::SeqCst));
    assert_eq!(probe.in_flight.load(Ordering::SeqCst), 3);
}
