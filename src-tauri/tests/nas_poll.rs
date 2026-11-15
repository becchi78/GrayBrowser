//! Integration tests for `watch::nas_poll::run_nas_diff_scan` against a real
//! tempdir standing in for a NAS share (genuine NAS I/O can't run in CI).
//!
//! Not attempted here: a "subdirectory temporarily inaccessible" scenario
//! via real Windows ACL manipulation (e.g. `icacls`) -- unlike the file-level
//! `share_mode(0)` trick used in `tests/scan_pipeline.rs`, denying access to a
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
//! `crates/gb-core/src/reconcile.rs`; its integration-level effect on a lone
//! video (confirmed deletion goes offline, an OS that still reports the file
//! holds it online) and the follow-up trigger on a changed catalog are covered
//! by the unit tests in `src/watch/nas_poll.rs`.

mod common;

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use common::{init_temp_db, real_registry, temp_thumbs_root};
use graybrowser_lib::db;
use graybrowser_lib::events::{CatalogNotifier, FakeCatalogNotifier};
use graybrowser_lib::watch::follow_up::FollowUp;
use graybrowser_lib::watch::nas_poll::run_nas_diff_scan;

/// One diff-scan pass over `folder` with the given stop flag and notifier, a
/// no-op follow-up and the real filesystem as the reachability probe.
fn scan_with(
    db: &db::Db,
    thumbs_root: &Path,
    folder: &str,
    stop_flag: &AtomicBool,
    notifier: &dyn CatalogNotifier,
) {
    run_nas_diff_scan(
        db,
        thumbs_root,
        folder,
        stop_flag,
        notifier,
        &FollowUp::noop(),
        &real_registry(),
    )
    .unwrap();
}

/// A scan that is not stopped and whose notifications nobody looks at.
fn scan(db: &db::Db, thumbs_root: &Path, folder: &str) {
    scan_with(
        db,
        thumbs_root,
        folder,
        &AtomicBool::new(false),
        &FakeCatalogNotifier::default(),
    );
}

/// A scan that is not stopped, reporting its notifications to `notifier`.
fn scan_notifying(db: &db::Db, thumbs_root: &Path, folder: &str, notifier: &dyn CatalogNotifier) {
    scan_with(db, thumbs_root, folder, &AtomicBool::new(false), notifier);
}

fn video_count(db: &db::Db) -> i64 {
    db.writer
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
        .unwrap()
}

/// A scan registers a new file, and the catalog is told once.
#[test]
fn registers_new_files_found_on_the_simulated_nas() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    fs::write(nas_dir.path().join("movie.mp4"), b"bytes").unwrap();
    let notifier = FakeCatalogNotifier::default();

    scan_notifying(
        &db,
        thumbs_root.path(),
        &nas_dir.path().to_string_lossy(),
        &notifier,
    );

    let conn = db.writer.lock().unwrap();
    let (status, file_name): (String, String) = conn
        .query_row("SELECT status, file_name FROM videos", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(status, "online");
    assert_eq!(file_name, "movie.mp4");
    assert_eq!(notifier.calls(), 1, "a fresh registration must notify");
}

/// Core guarantee: an unreachable NAS root must never mass-flip
/// previously-online videos offline in one poll cycle, and writes nothing, so
/// it must not notify either.
#[test]
fn an_unreachable_root_does_not_touch_any_existing_online_row_or_notify() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    fs::write(nas_dir.path().join("movie.mp4"), b"bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();

    scan(&db, thumbs_root.path(), &folder);

    // Simulate the NAS becoming completely unreachable (share disconnected,
    // drive unmapped, ...) by removing the root itself.
    drop(nas_dir); // TempDir::drop removes the directory tree

    let notifier = FakeCatalogNotifier::default();
    scan_notifying(&db, thumbs_root.path(), &folder, &notifier);

    let conn = db.writer.lock().unwrap();
    let status: String = conn
        .query_row("SELECT status FROM videos", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        status, "online",
        "an unreachable root must leave existing rows untouched, not flip them offline"
    );
    assert_eq!(
        notifier.calls(),
        0,
        "an unreachable root writes nothing, so it must not notify"
    );
}

// --- Cooperative cancellation ---

/// Checkpoint 1: a stop flag already set before `run_nas_diff_scan` is even
/// called must prevent any DB write for that cycle.
#[test]
fn a_pre_set_stop_flag_prevents_any_processing_this_cycle() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    fs::write(nas_dir.path().join("movie.mp4"), b"bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();

    scan_with(
        &db,
        thumbs_root.path(),
        &folder,
        &AtomicBool::new(true),
        &FakeCatalogNotifier::default(),
    );

    assert_eq!(
        video_count(&db),
        0,
        "a pre-set stop flag must prevent registration from happening at all"
    );
}

// --- Emit-triggering conditions (via FakeCatalogNotifier, no
// AppHandle/mock_app() needed -- see events.rs's doc comment for why) ---

#[test]
fn run_nas_diff_scan_does_not_notify_on_an_unchanged_rescan() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    fs::write(nas_dir.path().join("movie.mp4"), b"stable bytes").unwrap();
    let folder = nas_dir.path().to_string_lossy().to_string();

    scan(&db, thumbs_root.path(), &folder);

    let notifier = FakeCatalogNotifier::default();
    scan_notifying(&db, thumbs_root.path(), &folder, &notifier);

    assert_eq!(
        notifier.calls(),
        0,
        "an unchanged rescan (cheap-filter short-circuit, no DB write) must not notify"
    );
    assert_eq!(
        video_count(&db),
        1,
        "rescanning must not insert a duplicate row"
    );
}

/// Several videos going offline in one cycle are one catalog change, not
/// one per video: the frontend reloads its list on each notification, so a
/// per-row notify would make it re-query once per deleted file. Also the
/// flip side of the broken-enumeration guard: a couple of files missing out
/// of a large-enough library (20% loss, well under the 80% ratio threshold)
/// are confirmed, ordinary deletions and go offline while the rest stay.
#[test]
fn run_nas_diff_scan_notifies_once_when_several_files_go_offline() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let nas_dir = tempfile::tempdir().expect("failed to create nas tempdir");
    let paths: Vec<_> = (0..10)
        .map(|i| nas_dir.path().join(format!("movie{i}.mp4")))
        .collect();
    for p in &paths {
        fs::write(p, b"bytes").unwrap();
    }
    let folder = nas_dir.path().to_string_lossy().to_string();

    scan(&db, thumbs_root.path(), &folder);
    fs::remove_file(&paths[0]).unwrap();
    fs::remove_file(&paths[1]).unwrap();

    let notifier = FakeCatalogNotifier::default();
    scan_notifying(&db, thumbs_root.path(), &folder, &notifier);

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
    let count_with_status = |status: &str| -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM videos WHERE status = ?1",
            [status],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(count_with_status("offline"), 2);
    assert_eq!(count_with_status("online"), 8);
    assert_eq!(
        notifier.calls(),
        1,
        "several offline transitions in one cycle must notify exactly once"
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

    scan(&db, thumbs_root.path(), &folder);
    fs::remove_file(&known_path).unwrap();
    fs::write(nas_dir.path().join("new.mp4"), b"brand new bytes").unwrap();

    let stop_flag = Arc::new(AtomicBool::new(false));
    let stopping = StopOnNotify {
        stop_flag: Arc::clone(&stop_flag),
    };
    let plain = FakeCatalogNotifier::default();
    let notifier: &dyn CatalogNotifier = if stop_on_notify { &stopping } else { &plain };
    scan_with(&db, thumbs_root.path(), &folder, &stop_flag, notifier);
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

    scan_with(
        &db,
        thumbs_root.path(),
        &nas_dir.path().to_string_lossy(),
        &stop_flag,
        &notifier,
    );

    assert_eq!(
        video_count(&db),
        1,
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

    scan(&db, thumbs_root.path(), &folder);

    let new_bytes: &[u8] = b"rewritten with noticeably longer bytes";
    fs::write(&video_path, new_bytes).unwrap();
    let notifier = FakeCatalogNotifier::default();
    scan_notifying(&db, thumbs_root.path(), &folder, &notifier);

    assert_eq!(
        notifier.calls(),
        1,
        "a changed file must be reprocessed and notify exactly once"
    );
    let file_size: i64 = db
        .writer
        .lock()
        .unwrap()
        .query_row("SELECT file_size FROM videos", [], |r| r.get(0))
        .unwrap();
    assert_eq!(file_size, new_bytes.len() as i64);
    assert_eq!(
        video_count(&db),
        1,
        "reprocessing must update the row, not add one"
    );
}
