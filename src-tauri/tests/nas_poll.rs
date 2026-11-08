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
//! integration-level effect (a lone missing file goes offline once the OS
//! confirms it is not found, and is held while the OS still reports it).

use std::fs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gb_core::ports::path_existence::PathExistence;
use gb_core::testing::fake_path_existence::FakePathExistence;
use graybrowser_lib::adapters::folder_probe::RealFolderProbe;
use graybrowser_lib::adapters::path_existence::RealPathExistence;
use graybrowser_lib::db;
use graybrowser_lib::events::{CatalogNotifier, FakeCatalogNotifier, NoopReachabilityNotifier};
use graybrowser_lib::reachability::ReachabilityRegistry;
use graybrowser_lib::watch::follow_up::FollowUp;
use graybrowser_lib::watch::nas_poll::{
    run_nas_diff_scan, run_nas_diff_scan_with_existence, start_nas_polling,
};

/// Probes the real filesystem, standing in for the share, like the rest of
/// this suite.
fn real_registry() -> ReachabilityRegistry {
    ReachabilityRegistry::new(
        Arc::new(RealFolderProbe),
        Arc::new(NoopReachabilityNotifier),
    )
}

fn init_temp_db() -> (tempfile::TempDir, db::Db) {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db = db::init(&dir.path().join("app.db")).expect("db::init should succeed");
    (dir, db)
}

/// A throwaway `thumbnails/` root -- none of the scenarios below reactivate
/// a video that has cached thumbnails to move, so this only needs to be a
/// valid, empty directory.
fn temp_thumbs_root() -> tempfile::TempDir {
    tempfile::tempdir().expect("failed to create thumbs root tempdir")
}

#[test]
fn registers_new_files_found_on_the_simulated_nas() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    fs::write(nas_dir.path().join("movie.mp4"), b"bytes").unwrap();

    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &nas_dir.path().to_string_lossy(),
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
        &FollowUp::noop(),
        &real_registry(),
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
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    fs::write(nas_dir.path().join("movie.mp4"), b"stable bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();

    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();
    let quick_hash_before: String = {
        let conn = db.writer.lock().unwrap();
        conn.query_row("SELECT quick_hash FROM videos", [], |r| r.get(0))
            .unwrap()
    };

    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
        &FollowUp::noop(),
        &real_registry(),
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

/// Registers a lone `movie.mp4` in a fresh folder, deletes it, and runs a
/// second diff scan. Its per-path follow-up check asks the real filesystem,
/// or, with `os_still_reports_it`, a fake that claims the file is still
/// there. Returns the row's `(status, row count)` afterwards.
fn diff_scan_after_deleting_a_lone_video(os_still_reports_it: bool) -> (String, i64) {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    let video_path = nas_dir.path().join("movie.mp4");
    fs::write(&video_path, b"bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();
    let fake = FakePathExistence::with_paths(&[&video_path.to_string_lossy()]);
    let existence: &dyn PathExistence = if os_still_reports_it {
        &fake
    } else {
        &RealPathExistence
    };

    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();
    fs::remove_file(&video_path).unwrap();
    run_nas_diff_scan_with_existence(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
        &FollowUp::noop(),
        &real_registry(),
        existence,
    )
    .unwrap();

    let conn = db.writer.lock().unwrap();
    (
        conn.query_row("SELECT status FROM videos", [], |r| r.get(0))
            .unwrap(),
        conn.query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap(),
    )
}

/// With only one known video in the folder, its disappearance makes
/// `discovered_paths` empty and trips the `NothingDiscovered` floor -- but
/// the follow-up check of that one path gets NotFound from the real
/// filesystem, so it is a confirmed deletion and goes offline.
#[test]
fn a_lone_known_videos_deletion_is_confirmed_and_flipped_offline() {
    let (status, count) = diff_scan_after_deleting_a_lone_video(false);
    assert_eq!(status, "offline");
    assert_eq!(count, 1, "the row must be kept, only its status changes");
}

/// Same empty listing, but the OS claims the file is still there (what a
/// broken listing of a live share looks like): the `NothingDiscovered`
/// floor's hold stands.
#[test]
fn a_lone_known_video_is_held_online_while_the_os_still_reports_it() {
    let (status, count) = diff_scan_after_deleting_a_lone_video(true);
    assert_eq!(status, "online");
    assert_eq!(count, 1);
}

/// The flip side of the guard: a single file missing out of a large-enough
/// library (17% loss, well under the 80% ratio threshold) is a confirmed,
/// ordinary deletion and must still transition to offline as before.
#[test]
fn a_single_file_missing_from_a_larger_library_is_flipped_offline() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
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
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();
    fs::remove_file(&paths[0]).unwrap();
    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
        &FollowUp::noop(),
        &real_registry(),
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
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    let video_path = nas_dir.path().join("movie.mp4");
    fs::write(&video_path, b"bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();

    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();

    // Simulate the NAS becoming completely unreachable (share disconnected,
    // drive unmapped, ...) by removing the root itself.
    drop(nas_dir); // TempDir::drop removes the directory tree

    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
        &FollowUp::noop(),
        &real_registry(),
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
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    let folder = nas_dir.path().to_string_lossy().to_string();

    let handle = start_nas_polling(
        db,
        thumbs_root.path().to_path_buf(),
        folder,
        Duration::from_secs(600),
        Arc::new(FakeCatalogNotifier::default()),
        Arc::new(FollowUp::noop()),
        real_registry(),
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
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    fs::write(nas_dir.path().join("movie.mp4"), b"bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();

    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(true),
        &FakeCatalogNotifier::default(),
        &FollowUp::noop(),
        &real_registry(),
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
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    fs::write(nas_dir.path().join("movie.mp4"), b"bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();
    let notifier = FakeCatalogNotifier::default();

    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &notifier,
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();

    assert_eq!(notifier.calls(), 1, "a fresh registration must notify");
}

#[test]
fn run_nas_diff_scan_does_not_notify_on_an_unchanged_rescan() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    fs::write(nas_dir.path().join("movie.mp4"), b"stable bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();

    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();

    let notifier = FakeCatalogNotifier::default();
    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &notifier,
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();

    assert_eq!(
        notifier.calls(),
        0,
        "an unchanged rescan (cheap-filter short-circuit, no DB write) must not notify"
    );
}

#[test]
fn run_nas_diff_scan_notifies_when_a_file_goes_offline() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
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
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();
    fs::remove_file(&paths[0]).unwrap();

    let notifier = FakeCatalogNotifier::default();
    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &notifier,
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();

    assert_eq!(
        notifier.calls(),
        1,
        "a confirmed offline transition must notify"
    );
}

/// Several videos going offline in one cycle are one catalog change, not
/// one per video: the frontend reloads its list on each notification, so a
/// per-row notify would make it re-query once per deleted file.
#[test]
fn run_nas_diff_scan_notifies_once_when_several_files_go_offline() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    // 2 of 10 missing (20%) stays well under the broken-enumeration guard's
    // ratio threshold, so both deletions are taken at face value.
    let paths: Vec<_> = (0..10)
        .map(|i| nas_dir.path().join(format!("movie{i}.mp4")))
        .collect();
    for p in &paths {
        fs::write(p, b"bytes").unwrap();
    }
    let folder = nas_dir.path().to_string_lossy().to_string();

    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();
    fs::remove_file(&paths[0]).unwrap();
    fs::remove_file(&paths[1]).unwrap();

    let notifier = FakeCatalogNotifier::default();
    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &notifier,
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();

    let conn = db.writer.lock().unwrap();
    for p in &paths[..2] {
        let status: String = conn
            .query_row(
                "SELECT status FROM videos WHERE file_path = ?1",
                [p.to_string_lossy().as_ref()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "offline", "{} must go offline", p.display());
    }
    let offline_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM videos WHERE status = 'offline'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(offline_count, 2);
    assert_eq!(
        notifier.calls(),
        1,
        "several offline transitions in one cycle must notify exactly once"
    );
}

#[test]
fn run_nas_diff_scan_does_not_notify_when_the_root_is_unreachable() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    fs::write(nas_dir.path().join("movie.mp4"), b"bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();

    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();
    drop(nas_dir);

    let notifier = FakeCatalogNotifier::default();
    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &notifier,
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();

    assert_eq!(
        notifier.calls(),
        0,
        "an unreachable root writes nothing, so it must not notify"
    );
}

// --- Stop requested mid-scan ---

/// Raises the scan's own stop flag the moment the scan reports a catalog
/// change -- the only hook a test has *inside* a running scan, standing in
/// for `NasPollHandle::stop` being called from another thread mid-walk.
struct StopOnNotify {
    stop_flag: Arc<AtomicBool>,
}

impl CatalogNotifier for StopOnNotify {
    fn notify_changed(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }
}

/// Registers `known.mp4`, deletes it, adds `new.mp4` (different bytes, so it
/// is a fresh registration rather than a path-follow of the deleted one), and
/// runs a second diff scan. With `stop_on_notify`, registering `new.mp4`
/// raises the stop flag; otherwise the scan runs to completion. Returns the
/// deleted video's status and the row count afterwards.
///
/// The deleted video is the only known one, so it is withheld by the
/// broken-enumeration guard and settled by the per-path follow-up; that
/// check asks the real filesystem here, which reports NotFound -- so absent
/// a stop it goes offline, and staying online can only be the stop's doing.
fn diff_scan_with_a_new_file_and_a_deleted_known_video(stop_on_notify: bool) -> (String, i64) {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    let known_path = nas_dir.path().join("known.mp4");
    fs::write(&known_path, b"known bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();

    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();
    fs::remove_file(&known_path).unwrap();
    fs::write(nas_dir.path().join("new.mp4"), b"brand new bytes").unwrap();

    let stop_flag = Arc::new(AtomicBool::new(false));
    let stopping = StopOnNotify {
        stop_flag: Arc::clone(&stop_flag),
    };
    let plain = FakeCatalogNotifier::default();
    let notifier: &dyn CatalogNotifier = if stop_on_notify { &stopping } else { &plain };
    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &stop_flag,
        notifier,
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();
    assert_eq!(
        stop_flag.load(Ordering::SeqCst),
        stop_on_notify,
        "the stop flag must be raised exactly when the stopping notifier is used"
    );

    let conn = db.writer.lock().unwrap();
    (
        conn.query_row(
            "SELECT status FROM videos WHERE file_path = ?1",
            [known_path.to_string_lossy().as_ref()],
            |r| r.get(0),
        )
        .unwrap(),
        conn.query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap(),
    )
}

/// A stop requested after the walk has started must skip this cycle's
/// missing-video verdict: the deleted video stays online. `known.mp4` is
/// already gone, so the walk only yields the root and `new.mp4`; the stop
/// raised by `new.mp4`'s registration therefore always lands after the last
/// entry and is caught by the check before the final write, which is what
/// this test pins. The per-entry check during the walk is pinned separately
/// by `a_stop_requested_mid_walk_processes_no_further_entries`. The new
/// file's registration, which happened before the stop, is kept.
#[test]
fn a_stop_requested_mid_scan_does_not_flip_anything_offline() {
    let (status, count) = diff_scan_with_a_new_file_and_a_deleted_known_video(true);
    assert_eq!(status, "online");
    assert_eq!(count, 2, "the registration made before the stop is kept");
}

/// Control for the test above: the identical setup without a stop does flip
/// the deleted video offline.
#[test]
fn the_same_scan_without_a_stop_flips_the_deleted_video_offline() {
    let (status, count) = diff_scan_with_a_new_file_and_a_deleted_known_video(false);
    assert_eq!(status, "offline");
    assert_eq!(count, 2);
}

/// A stop raised while the walk still has entries left must end the walk
/// before the next entry, not just skip the final write. Several new files
/// with distinct contents are laid out and the first registration (whichever
/// file the walk happens to reach first) raises the stop, so exactly one row
/// exists afterwards regardless of walk order.
#[test]
fn a_stop_requested_mid_walk_processes_no_further_entries() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    for i in 0..5 {
        fs::write(
            nas_dir.path().join(format!("movie{i}.mp4")),
            format!("distinct bytes {i}"),
        )
        .unwrap();
    }
    let stop_flag = Arc::new(AtomicBool::new(false));
    let notifier = StopOnNotify {
        stop_flag: Arc::clone(&stop_flag),
    };

    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &nas_dir.path().to_string_lossy(),
        &stop_flag,
        &notifier,
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();

    let conn = db.writer.lock().unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        count, 1,
        "only the entry processed before the stop may be registered"
    );
}

// --- Changed content ---

/// A registered file whose size changed fails the cheap mtime/size filter,
/// so it is reprocessed (not skipped as unchanged) and the catalog is told
/// once.
#[test]
fn run_nas_diff_scan_reprocesses_and_notifies_once_when_a_files_content_changes() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    let video_path = nas_dir.path().join("movie.mp4");
    fs::write(&video_path, b"original bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();

    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();

    let new_bytes: &[u8] = b"rewritten with noticeably longer bytes";
    fs::write(&video_path, new_bytes).unwrap();
    let notifier = FakeCatalogNotifier::default();
    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &notifier,
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();

    assert_eq!(
        notifier.calls(),
        1,
        "a changed file must be reprocessed and notify exactly once"
    );
    let conn = db.writer.lock().unwrap();
    let (file_size, count): (i64, i64) = (
        conn.query_row("SELECT file_size FROM videos", [], |r| r.get(0))
            .unwrap(),
        conn.query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap(),
    );
    assert_eq!(file_size, new_bytes.len() as i64);
    assert_eq!(count, 1, "reprocessing must update the row, not add one");
}

// --- Follow-up triggering ---

const FOLLOW_UP_QUIET: Duration = Duration::from_millis(5);
const FOLLOW_UP_MAX_WAIT: Duration = Duration::from_millis(20);

/// A `FollowUp` with millisecond timings whose action only counts its runs.
fn counting_follow_up() -> (FollowUp, Arc<AtomicUsize>) {
    let runs = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&runs);
    let follow_up = FollowUp::with_timing(FOLLOW_UP_QUIET, FOLLOW_UP_MAX_WAIT, move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    (follow_up, runs)
}

/// The action runs on the follow-up's worker thread after the quiet period,
/// so poll with a generous deadline instead of a single fixed sleep that a
/// loaded CI runner could overshoot.
fn wait_for_a_run(runs: &AtomicUsize) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if runs.load(Ordering::SeqCst) > 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

#[test]
fn run_nas_diff_scan_runs_the_follow_up_after_registering_a_new_file() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    fs::write(nas_dir.path().join("movie.mp4"), b"bytes").unwrap();
    let (follow_up, runs) = counting_follow_up();

    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &nas_dir.path().to_string_lossy(),
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
        &follow_up,
        &real_registry(),
    )
    .unwrap();

    assert!(
        wait_for_a_run(&runs),
        "registering a new file must eventually run the follow-up action"
    );
}

#[test]
fn run_nas_diff_scan_does_not_run_the_follow_up_on_an_unchanged_rescan() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    fs::write(nas_dir.path().join("movie.mp4"), b"stable bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();

    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();

    let (follow_up, runs) = counting_follow_up();
    run_nas_diff_scan(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
        &follow_up,
        &real_registry(),
    )
    .unwrap();

    // Had the scan triggered, the action would run within quiet/max_wait (a
    // few ms); waiting far longer than that makes "still zero" mean "never
    // triggered" rather than "not run yet".
    std::thread::sleep(FOLLOW_UP_MAX_WAIT * 25);
    assert_eq!(
        runs.load(Ordering::SeqCst),
        0,
        "an unchanged rescan must not trigger the follow-up"
    );
}
