//! Integration tests for `watch::nas_poll::run_nas_diff_scan`/
//! `start_nas_polling` against a real tempdir standing in for a NAS share
//! (genuine NAS I/O can't run in CI).
//!
//! Not attempted here: a "subdirectory temporarily inaccessible" scenario
//! via real Windows ACL manipulation (e.g. `icacls`) -- unlike the file-level
//! `share_mode(0)` trick used elsewhere in this suite, denying access to a
//! *directory* and reliably restoring it afterward (including on test
//! panic/early return) is meaningfully riskier in a shared CI runner, and a
//! failed cleanup could leave a broken ACL behind. That specific guarantee
//! (`decide_missing_video_ids` leaves a video under an inaccessible
//! directory alone rather than marking it offline) is already covered by a
//! synthetic-data unit test in `crates/gb-core/src/reconcile.rs`
//! (`missing_stays_inconclusive_for_a_video_under_an_inaccessible_subdir`).
//! This file covers the root-reachable/unreachable wiring instead, which is
//! a core guarantee and safe to reproduce with a plain missing path.
//! The broken-enumeration ratio/floor guard's exact threshold behavior is
//! covered exhaustively (with concrete counts) by unit tests in
//! `crates/gb-core/src/reconcile.rs`; this file only exercises its
//! integration-level effect (a lone missing file is held, not flipped).

use std::fs;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use graybrowser_lib::db;
use graybrowser_lib::events::FakeCatalogNotifier;
use graybrowser_lib::watch::nas_poll::{run_nas_diff_scan, start_nas_polling};

fn init_temp_db() -> (tempfile::TempDir, db::Db) {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db = db::init(&dir.path().join("app.db")).expect("db::init should succeed");
    (dir, db)
}

#[test]
fn registers_new_files_found_on_the_simulated_nas() {
    let (_db_dir, db) = init_temp_db();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    fs::write(nas_dir.path().join("movie.mp4"), b"bytes").unwrap();

    run_nas_diff_scan(
        &db,
        &nas_dir.path().to_string_lossy(),
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
    )
    .unwrap();

    let conn = db.writer.lock().unwrap();
    let (status, file_name): (String, String) = conn
        .query_row("SELECT status, file_name FROM videos", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(status, "online");
    assert_eq!(file_name, "movie.mp4");
}

#[test]
fn rescanning_an_unchanged_file_does_not_recompute_quick_hash() {
    let (_db_dir, db) = init_temp_db();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    fs::write(nas_dir.path().join("movie.mp4"), b"stable bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();

    run_nas_diff_scan(
        &db,
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
    )
    .unwrap();
    let quick_hash_before: String = {
        let conn = db.writer.lock().unwrap();
        conn.query_row("SELECT quick_hash FROM videos", [], |r| r.get(0))
            .unwrap()
    };

    run_nas_diff_scan(
        &db,
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
    )
    .unwrap();
    let (quick_hash_after, count): (String, i64) = {
        let conn = db.writer.lock().unwrap();
        (
            conn.query_row("SELECT quick_hash FROM videos", [], |r| r.get(0))
                .unwrap(),
            conn.query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
                .unwrap(),
        )
    };
    assert_eq!(quick_hash_after, quick_hash_before);
    assert_eq!(count, 1, "rescanning must not insert a duplicate row");
}

/// Broken-enumeration guard: with only one known video in the
/// folder, its disappearance makes `discovered_paths` empty -- the
/// `NothingDiscovered` floor holds it online rather than treating this as a
/// confirmed deletion. This is the accepted trade-off: a
/// genuinely-deleted lone file in a small folder is, from a single poll
/// cycle's data alone, indistinguishable from a broken listing.
#[test]
fn a_lone_known_videos_disappearance_is_held_online_not_flipped_offline() {
    let (_db_dir, db) = init_temp_db();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    let video_path = nas_dir.path().join("movie.mp4");
    fs::write(&video_path, b"bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();

    run_nas_diff_scan(
        &db,
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
    )
    .unwrap();
    fs::remove_file(&video_path).unwrap();
    run_nas_diff_scan(
        &db,
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
    )
    .unwrap();

    let conn = db.writer.lock().unwrap();
    let (status, count): (String, i64) = (
        conn.query_row("SELECT status FROM videos", [], |r| r.get(0))
            .unwrap(),
        conn.query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap(),
    );
    assert_eq!(
        status, "online",
        "a lone missing file must be held online, not flipped offline"
    );
    assert_eq!(count, 1, "the row must be kept regardless");
}

/// The flip side of the guard: a single file missing out of a large-enough
/// library (17% loss, well under the 80% ratio threshold) is a confirmed,
/// ordinary deletion and must still transition to offline as before.
#[test]
fn a_single_file_missing_from_a_larger_library_is_flipped_offline() {
    let (_db_dir, db) = init_temp_db();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    let folder = nas_dir.path().to_string_lossy().to_string();
    let paths: Vec<_> = (0..6)
        .map(|i| nas_dir.path().join(format!("movie{i}.mp4")))
        .collect();
    for p in &paths {
        fs::write(p, b"bytes").unwrap();
    }

    run_nas_diff_scan(
        &db,
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
    )
    .unwrap();
    fs::remove_file(&paths[0]).unwrap();
    run_nas_diff_scan(
        &db,
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
    )
    .unwrap();

    let conn = db.writer.lock().unwrap();
    let online_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM videos WHERE status = 'online'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let offline_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM videos WHERE status = 'offline'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(online_count, 5);
    assert_eq!(offline_count, 1);
}

/// Core guarantee: an unreachable NAS root must never mass-flip
/// previously-online videos offline in one poll cycle.
#[test]
fn an_unreachable_root_does_not_touch_any_existing_online_row() {
    let (_db_dir, db) = init_temp_db();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    let video_path = nas_dir.path().join("movie.mp4");
    fs::write(&video_path, b"bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();

    run_nas_diff_scan(
        &db,
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
    )
    .unwrap();

    // Simulate the NAS becoming completely unreachable (share disconnected,
    // drive unmapped, ...) by removing the root itself.
    drop(nas_dir); // TempDir::drop removes the directory tree

    run_nas_diff_scan(
        &db,
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
    )
    .unwrap();

    let conn = db.writer.lock().unwrap();
    let status: String = conn
        .query_row("SELECT status FROM videos", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        status, "online",
        "an unreachable root must leave existing rows untouched, not flip them offline"
    );
}

// --- Cooperative cancellation ---

/// `NasPollHandle::stop()` must never block on the polling thread -- that's
/// what keeps `RealtimeWatchManager::reconfigure` (called synchronously
/// from the `pick_watch_folders` command handler) from freezing the UI for
/// as long as an unreachable NAS's OS-level timeout (~21s observed).
#[test]
fn stop_returns_almost_immediately_regardless_of_the_poller_thread() {
    let (_db_dir, db) = init_temp_db();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    let folder = nas_dir.path().to_string_lossy().to_string();

    let handle = start_nas_polling(
        db,
        folder,
        Duration::from_secs(600),
        Arc::new(FakeCatalogNotifier::default()),
    );
    // Let the background thread actually get going before measuring --
    // this test is about stop()'s own latency, not thread-startup latency.
    std::thread::sleep(Duration::from_millis(100));

    let t0 = Instant::now();
    handle.stop();
    let elapsed = t0.elapsed();

    assert!(
        elapsed < Duration::from_millis(50),
        "stop() must return almost immediately (no join): took {elapsed:?}"
    );
}

/// Checkpoint 1: a stop flag already set before `run_nas_diff_scan` is even
/// called must prevent any DB write for that cycle.
#[test]
fn a_pre_set_stop_flag_prevents_any_processing_this_cycle() {
    let (_db_dir, db) = init_temp_db();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    fs::write(nas_dir.path().join("movie.mp4"), b"bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();

    run_nas_diff_scan(
        &db,
        &folder,
        &AtomicBool::new(true),
        &FakeCatalogNotifier::default(),
    )
    .unwrap();

    let conn = db.writer.lock().unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        count, 0,
        "a pre-set stop flag must prevent registration from happening at all"
    );
}

// --- Emit-triggering conditions (via FakeCatalogNotifier, no
// AppHandle/mock_app() needed -- see events.rs's doc comment for why) ---

#[test]
fn run_nas_diff_scan_notifies_when_a_new_file_is_registered() {
    let (_db_dir, db) = init_temp_db();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    fs::write(nas_dir.path().join("movie.mp4"), b"bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();
    let notifier = FakeCatalogNotifier::default();

    run_nas_diff_scan(&db, &folder, &AtomicBool::new(false), &notifier).unwrap();

    assert_eq!(notifier.calls(), 1, "a fresh registration must notify");
}

#[test]
fn run_nas_diff_scan_does_not_notify_on_an_unchanged_rescan() {
    let (_db_dir, db) = init_temp_db();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    fs::write(nas_dir.path().join("movie.mp4"), b"stable bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();

    run_nas_diff_scan(
        &db,
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
    )
    .unwrap();

    let notifier = FakeCatalogNotifier::default();
    run_nas_diff_scan(&db, &folder, &AtomicBool::new(false), &notifier).unwrap();

    assert_eq!(
        notifier.calls(),
        0,
        "an unchanged rescan (cheap-filter short-circuit, no DB write) must not notify"
    );
}

#[test]
fn run_nas_diff_scan_notifies_when_a_file_goes_offline() {
    let (_db_dir, db) = init_temp_db();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    let paths: Vec<_> = (0..6)
        .map(|i| nas_dir.path().join(format!("movie{i}.mp4")))
        .collect();
    for p in &paths {
        fs::write(p, b"bytes").unwrap();
    }
    let folder = nas_dir.path().to_string_lossy().to_string();

    run_nas_diff_scan(
        &db,
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
    )
    .unwrap();
    fs::remove_file(&paths[0]).unwrap();

    let notifier = FakeCatalogNotifier::default();
    run_nas_diff_scan(&db, &folder, &AtomicBool::new(false), &notifier).unwrap();

    assert_eq!(
        notifier.calls(),
        1,
        "a confirmed offline transition must notify"
    );
}

#[test]
fn run_nas_diff_scan_does_not_notify_when_the_root_is_unreachable() {
    let (_db_dir, db) = init_temp_db();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    fs::write(nas_dir.path().join("movie.mp4"), b"bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();

    run_nas_diff_scan(
        &db,
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
    )
    .unwrap();
    drop(nas_dir);

    let notifier = FakeCatalogNotifier::default();
    run_nas_diff_scan(&db, &folder, &AtomicBool::new(false), &notifier).unwrap();

    assert_eq!(
        notifier.calls(),
        0,
        "an unreachable root writes nothing, so it must not notify"
    );
}
