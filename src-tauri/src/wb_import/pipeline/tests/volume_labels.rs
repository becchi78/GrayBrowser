//! `::LABEL\...` paths rewritten onto the drive of that volume label.

use super::common::{
    harness, no_op_ffmpeg, row_at, status_of, stored_watch_folders, Harness, OnlineDir, DASH,
};
use super::*;

// ---- volume labels ----

fn import_with_labels(
    h: &Harness,
    rows: Vec<WbMovieRow>,
    resolver: &FakeVolumeLabelResolver,
    probe: &FakeFolderProbe,
    reconf: Option<&FakeWatchReconfigurer>,
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
        reconf,
        &FakeDirLister::default(),
        probe,
        resolver,
        &FakeWbImportClock::default(),
        import_log,
    );
    let summary = notifier.complete_calls.lock().unwrap()[0].clone();
    summary
}

#[test]
fn a_connected_label_is_rewritten_and_registered_online_with_its_folder_watched() {
    let h = harness();
    let dir = OnlineDir::new("a.mp4");
    std::fs::write(dir.dir.path().join("b.mp4"), b"more bytes").unwrap();
    let resolver = FakeVolumeLabelResolver::default().with(
        "esd-usb",
        LabelResolution::Resolved(format!("{}\\", dir.folder())),
    );
    let log = FakeWbImportLog::default();
    let reconf = FakeWatchReconfigurer::default();

    let summary = import_with_labels(
        &h,
        vec![row_at(1, r"::ESD-USB\a.mp4"), row_at(2, r"::esd-usb\b.mp4")],
        &resolver,
        &FakeFolderProbe::default(),
        Some(&reconf),
        &log,
    );

    assert_eq!(summary.registered_online, 2);
    assert_eq!(summary.volume_label_resolved, 2);
    assert_eq!(summary.volume_label_unresolved, 0);
    assert!(summary.volume_label_unresolved_labels.is_empty());
    assert_eq!(status_of(&h, &dir.file("a.mp4")).0, "online");
    assert_eq!(status_of(&h, &dir.file("b.mp4")).0, "online");
    assert_eq!(summary.watch_folders_added, vec![dir.folder()]);
    // Both rows share one label, so the drive is asked about once.
    assert_eq!(resolver.calls.lock().unwrap().as_slice(), ["ESD-USB"]);
    let text = log.text();
    assert!(text.contains("ボリュームラベルのパス: 1 種類"));
    // The log keeps the path the .wb stored, not the rewritten one.
    assert!(text.contains(r"::ESD-USB\a.mp4"));
}

/// Pins the documented limitation: the rewrite happens before
/// registration, so importing again once the drive is connected adds a
/// second row under the drive letter and leaves the `::` row behind.
#[test]
fn reimporting_with_the_drive_connected_adds_a_drive_letter_row_beside_the_old_label_row() {
    let h = harness();
    let dir = OnlineDir::new("a.mp4");
    let rows = || vec![row_at(1, r"::ESD-USB\a.mp4")];
    let count = || -> i64 {
        h.db.writer
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap()
    };

    let missing = FakeVolumeLabelResolver::default();
    let first = import_with_labels(
        &h,
        rows(),
        &missing,
        &FakeFolderProbe::default(),
        None,
        &FakeWbImportLog::default(),
    );
    assert_eq!(first.registered_offline, 1);
    assert_eq!(count(), 1);

    // Still disconnected: the same `::` path is already registered.
    let again = import_with_labels(
        &h,
        rows(),
        &missing,
        &FakeFolderProbe::default(),
        None,
        &FakeWbImportLog::default(),
    );
    assert_eq!(again.skipped, 1);
    assert_eq!(count(), 1);

    let connected = FakeVolumeLabelResolver::default().with(
        "ESD-USB",
        LabelResolution::Resolved(format!("{}\\", dir.folder())),
    );
    let second = import_with_labels(
        &h,
        rows(),
        &connected,
        &FakeFolderProbe::default(),
        None,
        &FakeWbImportLog::default(),
    );

    assert_eq!(second.registered_online, 1);
    assert_eq!(second.skipped, 0);
    assert_eq!(count(), 2);
    assert_eq!(status_of(&h, r"::ESD-USB\a.mp4").0, "offline");
    assert_eq!(status_of(&h, &dir.file("a.mp4")).0, "online");
}

#[test]
fn a_rewritten_path_is_what_the_share_probe_sees() {
    let h = harness();
    let resolver = FakeVolumeLabelResolver::default()
        .with("NAS-DISK", LabelResolution::Resolved(r"W:\".to_string()));
    let probe = FakeFolderProbe::with_unreachable(&[r"W:\"]);
    let log = FakeWbImportLog::default();

    let summary = import_with_labels(
        &h,
        vec![row_at(1, r"::NAS-DISK\v\a.mp4")],
        &resolver,
        &probe,
        None,
        &log,
    );

    assert_eq!(summary.registered_offline, 1);
    assert_eq!(summary.volume_label_resolved, 1);
    assert_eq!(status_of(&h, r"W:\v\a.mp4").0, "offline");
    assert!(log.text().contains(r"W:\"));
}

#[test]
fn question_marks_in_a_rewritten_path_are_restored() {
    let h = harness();
    let real = format!("a{DASH}.mp4");
    let resolver = FakeVolumeLabelResolver::default()
        .with("ESD-USB", LabelResolution::Resolved(r"E:\".to_string()));
    // Lists only the drive-letter folder, so the restoration can only
    // succeed if it was given the rewritten path.
    let lister = FakeDirLister::default().with_dir(r"E:\v", &[&real]);
    let notifier = FakeWbImportNotifier::default();
    super::import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies: Ok(vec![row_at(1, r"::ESD-USB\v\a?.mp4")]),
            ..Default::default()
        },
        &no_op_ffmpeg(),
        &notifier,
        &FakeCatalogNotifier::default(),
        None::<&FakeWatchReconfigurer>,
        &lister,
        &FakeFolderProbe::default(),
        &resolver,
        &FakeWbImportClock::default(),
        &FakeWbImportLog::default(),
    );

    assert_eq!(
        lister.calls(),
        vec![r"E:\v".to_string()],
        "the folder listed is the rewritten one"
    );
    assert_eq!(status_of(&h, &format!(r"E:\v\{real}")).0, "offline");
}

#[test]
fn unresolved_labels_keep_the_raw_path_register_offline_and_are_never_watched() {
    let h = harness();
    let resolver = FakeVolumeLabelResolver::default().with("TWIN", LabelResolution::Ambiguous);
    let log = FakeWbImportLog::default();
    let reconf = FakeWatchReconfigurer::default();

    let summary = import_with_labels(
        &h,
        vec![
            row_at(1, r"::ESD-USB\a.mp4"),
            row_at(2, r"::ESD-USB\sub\b.mp4"),
            row_at(3, r"::TWIN\c.mp4"),
        ],
        &resolver,
        &FakeFolderProbe::default(),
        Some(&reconf),
        &log,
    );

    assert_eq!(summary.registered_offline, 3);
    assert_eq!(summary.volume_label_resolved, 0);
    assert_eq!(summary.volume_label_unresolved, 3);
    assert_eq!(
        summary.volume_label_unresolved_labels,
        vec!["ESD-USB".to_string(), "TWIN".to_string()]
    );
    assert_eq!(status_of(&h, r"::ESD-USB\a.mp4").0, "offline");
    assert_eq!(status_of(&h, r"::TWIN\c.mp4").0, "offline");
    assert!(summary.watch_folders_added.is_empty());
    assert!(summary.watch_folders_skipped_roots.is_empty());
    assert!(stored_watch_folders(&h).is_empty());
    assert!(reconf.calls.lock().unwrap().is_empty());
    let text = log.text();
    assert!(text.contains("直せませんでした (2 行)"));
    assert!(text.contains("同じラベルのドライブが複数あるため"));
    assert!(text.contains("ドライブ文字に直した 0 / 直せなかった 3"));
}

#[test]
fn a_wb_without_labels_never_asks_the_resolver_or_mentions_labels() {
    let h = harness();
    let resolver = FakeVolumeLabelResolver::default();
    let log = FakeWbImportLog::default();

    let summary = import_with_labels(
        &h,
        vec![row_at(1, r"Z:\v\a.mp4")],
        &resolver,
        &FakeFolderProbe::default(),
        None,
        &log,
    );

    assert_eq!(summary.volume_label_resolved, 0);
    assert_eq!(summary.volume_label_unresolved, 0);
    assert!(resolver.calls.lock().unwrap().is_empty());
    assert!(!log.text().contains("ボリュームラベル"));
}
