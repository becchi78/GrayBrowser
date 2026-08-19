//! Integration test for `adapters::watcher::RealFileWatcher` against a real
//! tempdir: proves the `notify` wrapper actually normalizes a real OS
//! filesystem event into `gb_core::ports::watcher::WatchEvent`. Bounded via
//! `mpsc::recv_timeout` since `notify`'s Windows backend delivers events
//! asynchronously from a background thread with no fixed latency guarantee.

use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use gb_core::ports::watcher::{FileWatcher, WatchEvent, WatchEventKind};
use graybrowser_lib::adapters::watcher::RealFileWatcher;

const EVENT_TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn creating_a_file_produces_a_normalized_created_event() {
    let scan_dir = tempfile::tempdir().expect("failed to create scan tempdir");
    let (tx, rx) = mpsc::channel::<WatchEvent>();

    let watcher = RealFileWatcher;
    let _handle = watcher
        .watch(
            scan_dir.path(),
            Box::new(move |event| {
                let _ = tx.send(event);
            }),
        )
        .expect("watch() should succeed against a real tempdir");

    let video_path = scan_dir.path().join("movie.mp4");
    fs::write(&video_path, b"fake video bytes").expect("failed to write test file");

    let mut saw_created_for_video = false;
    // Windows can also deliver Modify events shortly after Create for the
    // same write; drain a few events looking specifically for the Created
    // one instead of asserting on just the first event received.
    for _ in 0..10 {
        match rx.recv_timeout(EVENT_TIMEOUT) {
            Ok(event)
                if event.kind == WatchEventKind::Created
                    && paths_match(&event.path, &video_path) =>
            {
                saw_created_for_video = true;
                break;
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }

    assert!(
        saw_created_for_video,
        "expected a Created WatchEvent for {} within {EVENT_TIMEOUT:?}",
        video_path.display()
    );
}

/// `notify` may report a canonicalized path (e.g. Windows' `\\?\` long-path
/// form) that differs textually from the plain path we wrote to, so compare
/// file names rather than requiring exact `PathBuf` equality.
fn paths_match(a: &std::path::Path, b: &std::path::Path) -> bool {
    a.file_name() == b.file_name() && a.parent().map(canon) == b.parent().map(canon)
}

fn canon(p: &std::path::Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}
