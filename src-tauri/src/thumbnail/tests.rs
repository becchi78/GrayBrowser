use super::*;
use crate::db::test_support::{init_temp_db, insert_test_video};
use gb_core::ports::ffmpeg::FfmpegAvailability;
use gb_core::testing::fake_ffmpeg::{FakeCall, FakeFfmpegAdapter};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use crate::test_support::{all_reachable, registry_over};
use crate::test_support::{thumbnail_attempts_of, CountingNotifier};
use gb_core::testing::fake_folder_probe::FakeFolderProbe;

/// Records progress notifications and counts `notify_changed` calls.
#[derive(Default)]
struct ProgressRecorder(Mutex<Vec<(u32, u32)>>, AtomicUsize);

impl CatalogNotifier for ProgressRecorder {
    fn notify_changed(&self) {
        self.1.fetch_add(1, Ordering::SeqCst);
    }

    fn notify_thumbnail_progress(&self, done: u32, total: u32) {
        self.0.lock().unwrap().push((done, total));
    }
}

fn pending_entry(id: &str) -> (String, PathBuf) {
    (id.to_string(), PathBuf::from(format!("C:/videos/{id}.mp4")))
}

fn failing_ffmpeg() -> Arc<FakeFfmpegAdapter> {
    Arc::new(FakeFfmpegAdapter {
        availability: Ok(FfmpegAvailability {
            ffmpeg_version: Some("7.0".into()),
            ffprobe_version: Some("7.0".into()),
        }),
        duration: Ok(Some(120.0)),
        extract_result: Box::new(|_seek| {
            Err(gb_core::ports::ffmpeg::FfmpegError::Spawn("boom".into()))
        }),
        ..Default::default()
    })
}

fn available_ffmpeg(extract_delay: Duration) -> Arc<FakeFfmpegAdapter> {
    Arc::new(FakeFfmpegAdapter {
        availability: Ok(FfmpegAvailability {
            ffmpeg_version: Some("7.0".into()),
            ffprobe_version: Some("7.0".into()),
        }),
        duration: Ok(Some(120.0)),
        extract_result: Box::new(move |_seek| {
            thread::sleep(extract_delay);
            Ok(())
        }),
        ..Default::default()
    })
}

fn unassigned_dir(root: &Path) -> PathBuf {
    root.join(gb_core::thumbnail_paths::THUMBNAIL_UNASSIGNED_SUBDIR)
}

fn thumbnail_ready_flag(db: &Db, id: &str) -> i64 {
    db.writer
        .lock()
        .unwrap()
        .query_row(
            "SELECT thumbnail_ready FROM videos WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .unwrap()
}

fn extract_count_for(ffmpeg: &FakeFfmpegAdapter, id: &str) -> usize {
    let prefix = format!("{id}_");
    ffmpeg
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| match c {
            FakeCall::ExtractThumbnail {
                output_tmp_path, ..
            } => output_tmp_path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(&prefix)),
            _ => false,
        })
        .count()
}

fn six_finals_exist(dir: &Path, id: &str) -> bool {
    (0..worker::THUMBNAILS_PER_VIDEO).all(|i| dir.join(format!("{id}_{i}.webp")).exists())
}

#[test]
fn two_queues_racing_on_the_same_video_extract_only_once() {
    let (_db_dir, db) = init_temp_db();
    let thumbs = tempfile::tempdir().unwrap();
    insert_test_video(&db, "vid-race", "C:/videos/race.mp4");
    // Slow extraction keeps the first queue's claim held while the
    // second one has already listed the same (still missing) video.
    let ffmpeg = available_ffmpeg(Duration::from_millis(40));
    let notifier = Arc::new(CountingNotifier(AtomicUsize::new(0)));
    let queue = ThumbnailQueueHandle::default();
    let barrier = Arc::new(std::sync::Barrier::new(2));

    let threads: Vec<_> = (0..2)
        .map(|_| {
            let (db, root) = (db.clone(), thumbs.path().to_path_buf());
            let (queue, ffmpeg, notifier) =
                (queue.clone(), Arc::clone(&ffmpeg), Arc::clone(&notifier));
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                run_thumbnail_queue(db, root, queue, ffmpeg, notifier, all_reachable());
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }

    assert_eq!(
        extract_count_for(&ffmpeg, "vid-race"),
        worker::THUMBNAILS_PER_VIDEO,
        "the video must be extracted by exactly one of the queues"
    );
    assert!(six_finals_exist(&unassigned_dir(thumbs.path()), "vid-race"));
    assert_eq!(thumbnail_ready_flag(&db, "vid-race"), 1);
    assert_eq!(notifier.0.load(Ordering::SeqCst), 1);
}

#[test]
fn claim_is_exclusive_per_video_and_shared_across_clones() {
    let queue = ThumbnailQueueHandle::default();
    let clone = queue.clone();
    let guard = queue.try_claim("a").expect("first claim succeeds");
    assert!(clone.try_claim("a").is_none());
    assert!(
        clone.try_claim("b").is_some(),
        "other videos are unaffected"
    );
    drop(guard);
    assert!(clone.try_claim("a").is_some());
}

#[test]
fn claim_is_released_after_a_panic() {
    let queue = ThumbnailQueueHandle::default();
    let worker_queue = queue.clone();
    let result = thread::spawn(move || {
        let _guard = worker_queue.try_claim("v").unwrap();
        panic!("worker blew up");
    })
    .join();
    assert!(result.is_err());
    assert!(queue.try_claim("v").is_some(), "released after panic");
}

#[test]
fn orphan_tmp_files_are_removed_but_final_webp_files_are_kept() {
    let root = tempfile::tempdir().unwrap();
    let sub = root.path().join("folder-sub");
    std::fs::create_dir_all(sub.join("deeper")).unwrap();
    let tmp_names = [
        "id_0.webp.tmp",
        "id_1.webp.123-4.tmp",
        "id.wb_legacy.webp.tmp",
    ];
    let keep_names = ["id_0.webp", "id_1.webp", "notes.txt"];
    for dir in [root.path(), sub.as_path()] {
        for n in tmp_names.iter().chain(keep_names.iter()) {
            std::fs::write(dir.join(n), b"x").unwrap();
        }
    }
    // Two levels down is out of scope and must be left alone.
    std::fs::write(sub.join("deeper").join("id_9.webp.tmp"), b"x").unwrap();

    let removed = remove_orphan_tmp_files(root.path());

    assert_eq!(removed, tmp_names.len() * 2);
    for dir in [root.path(), sub.as_path()] {
        for n in tmp_names {
            assert!(!dir.join(n).exists(), "{n} should be removed");
        }
        for n in keep_names {
            assert!(dir.join(n).exists(), "{n} must be kept");
        }
    }
    assert!(sub.join("deeper").join("id_9.webp.tmp").exists());
    assert_eq!(remove_orphan_tmp_files(&root.path().join("missing")), 0);
}

#[test]
fn a_stalled_video_is_recovered_by_cleanup_then_requeue() {
    let (_db_dir, db) = init_temp_db();
    let thumbs = tempfile::tempdir().unwrap();
    let dir = unassigned_dir(thumbs.path());
    insert_test_video(&db, "vid-stuck", "C:/videos/stuck.mp4");
    // Stalled state: some slots published, tmp files stranded, no
    // attempt recorded.
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..3 {
        std::fs::write(dir.join(format!("vid-stuck_{i}.webp")), b"old").unwrap();
    }
    std::fs::write(dir.join("vid-stuck_3.webp.tmp"), b"old").unwrap();
    std::fs::write(dir.join("vid-stuck_4.webp.99-1.tmp"), b"old").unwrap();
    assert_eq!(thumbnail_ready_flag(&db, "vid-stuck"), 0);

    assert_eq!(remove_orphan_tmp_files(thumbs.path()), 2);
    let pending = worker::list_videos_missing_thumbnails(&db, thumbs.path(), &[]).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].0, "vid-stuck");

    let ffmpeg = available_ffmpeg(Duration::ZERO);
    run_thumbnail_queue(
        db.clone(),
        thumbs.path().to_path_buf(),
        ThumbnailQueueHandle::default(),
        Arc::clone(&ffmpeg),
        Arc::new(CountingNotifier(AtomicUsize::new(0))),
        all_reachable(),
    );

    assert!(six_finals_exist(&dir, "vid-stuck"));
    assert_eq!(thumbnail_ready_flag(&db, "vid-stuck"), 1);
    assert!(std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .all(|e| !e.file_name().to_string_lossy().ends_with(".tmp")));
}

#[test]
fn progress_climbs_and_ends_hidden_after_a_full_run() {
    let (_db_dir, db) = init_temp_db();
    let thumbs = tempfile::tempdir().unwrap();
    for id in ["a", "b", "c"] {
        insert_test_video(&db, id, &format!("C:/videos/{id}.mp4"));
    }
    let notifier = Arc::new(ProgressRecorder::default());
    let queue = ThumbnailQueueHandle::default();

    run_thumbnail_queue(
        db,
        thumbs.path().to_path_buf(),
        queue.clone(),
        available_ffmpeg(Duration::ZERO),
        Arc::clone(&notifier),
        all_reachable(),
    );

    let calls = notifier.0.lock().unwrap().clone();
    assert_eq!(calls.first(), Some(&(0, 3)));
    assert_eq!(calls.last(), Some(&(0, 0)));
    assert!(
        calls.iter().all(|&(_, total)| total == 3 || total == 0),
        "{calls:?}"
    );
    assert_eq!(queue.progress_snapshot(), ProgressSnapshot::default());
}

#[test]
fn overlapping_queues_do_not_inflate_the_total() {
    let (_db_dir, db) = init_temp_db();
    let thumbs = tempfile::tempdir().unwrap();
    for id in ["a", "b", "c"] {
        insert_test_video(&db, id, &format!("C:/videos/{id}.mp4"));
    }
    let notifier = Arc::new(ProgressRecorder::default());
    let queue = ThumbnailQueueHandle::default();
    // Another queue has already listed a and b but not started them.
    queue.track_pending(["a", "b"]);

    process_pending(
        db,
        thumbs.path().to_path_buf(),
        Vec::new(),
        queue.clone(),
        available_ffmpeg(Duration::ZERO),
        Arc::clone(&notifier),
        vec![pending_entry("b"), pending_entry("c")],
        all_reachable(),
    );

    let calls = notifier.0.lock().unwrap().clone();
    assert_eq!(calls.first(), Some(&(0, 3)), "b is one video, not two");
    assert_eq!(calls.last(), Some(&(2, 3)), "a is still waiting");
    queue.finish_tracked("a", FinishOutcome::Generated);
    assert_eq!(queue.progress_snapshot(), ProgressSnapshot::default());
}

#[test]
fn failures_count_as_processed_so_the_run_still_ends_hidden() {
    let (_db_dir, db) = init_temp_db();
    let thumbs = tempfile::tempdir().unwrap();
    for id in ["a", "b"] {
        insert_test_video(&db, id, &format!("C:/videos/{id}.mp4"));
    }
    let notifier = Arc::new(ProgressRecorder::default());

    run_thumbnail_queue(
        db,
        thumbs.path().to_path_buf(),
        ThumbnailQueueHandle::default(),
        failing_ffmpeg(),
        Arc::clone(&notifier),
        all_reachable(),
    );

    let calls = notifier.0.lock().unwrap().clone();
    assert_eq!(calls.first(), Some(&(0, 2)));
    assert_eq!(calls.last(), Some(&(0, 0)));
    assert!(calls.iter().all(|&(done, _)| done == 0), "{calls:?}");
}

#[test]
fn a_video_held_by_an_untracked_claimant_is_dropped_and_the_run_ends_hidden() {
    let (_db_dir, db) = init_temp_db();
    let thumbs = tempfile::tempdir().unwrap();
    insert_test_video(&db, "a", "C:/videos/a.mp4");
    let queue = ThumbnailQueueHandle::default();
    // Stands in for a manual retry, which claims but never tracks.
    let _held = queue.try_claim("a").unwrap();
    let notifier = Arc::new(ProgressRecorder::default());

    process_pending(
        db,
        thumbs.path().to_path_buf(),
        Vec::new(),
        queue.clone(),
        available_ffmpeg(Duration::ZERO),
        Arc::clone(&notifier),
        vec![pending_entry("a")],
        all_reachable(),
    );

    let calls = notifier.0.lock().unwrap().clone();
    assert_eq!(calls, vec![(0, 1), (0, 0)]);
    assert_eq!(queue.progress_snapshot(), ProgressSnapshot::default());
}

#[test]
fn re_adding_a_video_whose_claim_holder_already_finished_does_not_get_stuck() {
    let (_db_dir, db) = init_temp_db();
    let thumbs = tempfile::tempdir().unwrap();
    insert_test_video(&db, "x", "C:/videos/x.mp4");
    let queue = ThumbnailQueueHandle::default();
    // Worker A: finished x but still holds the claim until iteration end.
    queue.track_pending(["x"]);
    let held = queue.try_claim("x").unwrap();
    queue.finish_tracked("x", FinishOutcome::Generated);
    let notifier = Arc::new(ProgressRecorder::default());

    // Queue B lists x from a stale snapshot and loses the claim.
    process_pending(
        db,
        thumbs.path().to_path_buf(),
        Vec::new(),
        queue.clone(),
        available_ffmpeg(Duration::ZERO),
        Arc::clone(&notifier),
        vec![pending_entry("x")],
        all_reachable(),
    );
    drop(held);

    assert_eq!(notifier.0.lock().unwrap().last(), Some(&(0, 0)));
    assert_eq!(queue.progress_snapshot(), ProgressSnapshot::default());
}

#[test]
fn already_complete_videos_in_a_stale_list_are_not_regenerated_count_as_done_and_end_hidden() {
    let (_db_dir, db) = init_temp_db();
    let thumbs = tempfile::tempdir().unwrap();
    let dir = unassigned_dir(thumbs.path());
    std::fs::create_dir_all(&dir).unwrap();
    for id in ["a", "b"] {
        insert_test_video(&db, id, &format!("C:/videos/{id}.mp4"));
        for i in 0..worker::THUMBNAILS_PER_VIDEO {
            std::fs::write(dir.join(format!("{id}_{i}.webp")), b"original").unwrap();
        }
    }
    let notifier = Arc::new(ProgressRecorder::default());
    let queue = ThumbnailQueueHandle::default();
    let ffmpeg = available_ffmpeg(Duration::ZERO);

    process_pending(
        db.clone(),
        thumbs.path().to_path_buf(),
        Vec::new(),
        queue.clone(),
        Arc::clone(&ffmpeg),
        Arc::clone(&notifier),
        vec![pending_entry("a"), pending_entry("b")],
        all_reachable(),
    );

    for id in ["a", "b"] {
        assert_eq!(extract_count_for(&ffmpeg, id), 0, "{id}");
        for i in 0..worker::THUMBNAILS_PER_VIDEO {
            let bytes = std::fs::read(dir.join(format!("{id}_{i}.webp"))).unwrap();
            assert_eq!(bytes, b"original", "{id} slot {i} must not be overwritten");
        }
        assert_eq!(thumbnail_ready_flag(&db, id), 1, "{id}");
    }
    assert_eq!(
        notifier.1.load(Ordering::SeqCst),
        0,
        "nothing was generated, so nothing changed"
    );
    let calls = notifier.0.lock().unwrap().clone();
    assert!(
        calls.contains(&(1, 2)),
        "skipped-as-complete counts as done: {calls:?}"
    );
    assert_eq!(calls.last(), Some(&(0, 0)));
    assert_eq!(queue.progress_snapshot(), ProgressSnapshot::default());
}

/// Blocks inside the first notification until released, then records it.
struct BlockingFirstNotifier {
    calls: Mutex<Vec<(u32, u32)>>,
    entered: std::sync::mpsc::Sender<()>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
    first: AtomicBool,
}

impl CatalogNotifier for BlockingFirstNotifier {
    fn notify_changed(&self) {}

    fn notify_thumbnail_progress(&self, done: u32, total: u32) {
        if self.first.swap(false, Ordering::SeqCst) {
            self.entered.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
        }
        self.calls.lock().unwrap().push((done, total));
    }
}

#[test]
fn notifications_arrive_in_snapshot_order_even_if_a_notifier_is_slow() {
    let queue = ThumbnailQueueHandle::default();
    queue.track_pending(["a", "b"]);
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let notifier = Arc::new(BlockingFirstNotifier {
        calls: Mutex::new(Vec::new()),
        entered: entered_tx,
        release: Mutex::new(release_rx),
        first: AtomicBool::new(true),
    });

    let first = {
        let (queue, notifier) = (queue.clone(), Arc::clone(&notifier));
        thread::spawn(move || queue.publish_progress(notifier.as_ref()))
    };
    // The first publisher took its (0, 2) snapshot and is stuck notifying.
    entered_rx.recv().unwrap();
    queue.finish_tracked("a", FinishOutcome::Generated);
    let second = {
        let (queue, notifier) = (queue.clone(), Arc::clone(&notifier));
        thread::spawn(move || queue.publish_progress(notifier.as_ref()))
    };
    // Give the second publisher time to overtake if nothing orders them.
    thread::sleep(Duration::from_millis(100));
    release_tx.send(()).unwrap();
    first.join().unwrap();
    second.join().unwrap();

    assert_eq!(*notifier.calls.lock().unwrap(), vec![(0, 2), (1, 2)]);
}

const NAS: &str = "W:/video";

fn nas_entry(id: &str) -> (String, PathBuf) {
    (id.to_string(), PathBuf::from(format!("{NAS}/{id}.mp4")))
}

fn watch_folders() -> Vec<String> {
    vec![NAS.to_string(), "C:/videos".to_string()]
}

#[test]
fn videos_in_an_unreachable_folder_are_skipped_without_spending_attempts() {
    let (_db_dir, db) = init_temp_db();
    let thumbs = tempfile::tempdir().unwrap();
    insert_test_video(&db, "nas-1", "W:/video/nas-1.mp4");
    insert_test_video(&db, "nas-2", "W:/video/nas-2.mp4");
    insert_test_video(&db, "local-1", "C:/videos/local-1.mp4");
    let probe = Arc::new(FakeFolderProbe::with_unreachable(&[NAS]));
    let ffmpeg = failing_ffmpeg();
    let recorder = Arc::new(ProgressRecorder::default());
    let queue = ThumbnailQueueHandle::default();

    process_pending(
        db.clone(),
        thumbs.path().to_path_buf(),
        watch_folders(),
        queue.clone(),
        Arc::clone(&ffmpeg),
        Arc::clone(&recorder),
        vec![
            nas_entry("nas-1"),
            nas_entry("nas-2"),
            pending_entry("local-1"),
        ],
        registry_over(probe.clone()),
    );

    assert_eq!(thumbnail_attempts_of(&db, "nas-1"), 0);
    assert_eq!(thumbnail_attempts_of(&db, "nas-2"), 0);
    assert_eq!(thumbnail_attempts_of(&db, "local-1"), 1);
    assert_eq!(extract_count_for(&ffmpeg, "nas-1"), 0);
    assert_eq!(extract_count_for(&ffmpeg, "nas-2"), 0);
    assert_eq!(probe.call_count_for(NAS), 1, "probed once, then cached");
    // Skipped videos retire from the batch, so the progress finishes
    // (resets to 0/0) instead of waiting on them forever.
    let reported = recorder.0.lock().unwrap().clone();
    assert_eq!(reported.last(), Some(&(0, 0)));
    assert_eq!(queue.progress_snapshot().total, 0);
}

#[test]
fn a_failure_while_the_folder_just_went_down_does_not_spend_an_attempt() {
    let (_db_dir, db) = init_temp_db();
    let thumbs = tempfile::tempdir().unwrap();
    insert_test_video(&db, "nas-1", "W:/video/nas-1.mp4");
    let probe = Arc::new(FakeFolderProbe::default());
    let registry = registry_over(probe.clone());
    // Known reachable, so the worker goes ahead; the share then drops.
    assert!(registry.is_reachable(NAS));
    probe.set_reachable(NAS, false);

    process_pending(
        db.clone(),
        thumbs.path().to_path_buf(),
        watch_folders(),
        ThumbnailQueueHandle::default(),
        failing_ffmpeg(),
        Arc::new(CountingNotifier(AtomicUsize::new(0))),
        vec![nas_entry("nas-1")],
        registry.clone(),
    );

    assert_eq!(thumbnail_attempts_of(&db, "nas-1"), 0);
    assert_eq!(registry.unreachable_folders(), vec![NAS.to_string()]);
}

#[test]
fn a_failure_in_a_reachable_folder_spends_an_attempt_and_notifies_the_catalog() {
    let (_db_dir, db) = init_temp_db();
    let thumbs = tempfile::tempdir().unwrap();
    insert_test_video(&db, "nas-1", "W:/video/nas-1.mp4");
    let notifier = Arc::new(CountingNotifier(AtomicUsize::new(0)));

    process_pending(
        db.clone(),
        thumbs.path().to_path_buf(),
        watch_folders(),
        ThumbnailQueueHandle::default(),
        failing_ffmpeg(),
        Arc::clone(&notifier),
        vec![nas_entry("nas-1")],
        all_reachable(),
    );

    assert_eq!(thumbnail_attempts_of(&db, "nas-1"), 1);
    assert_eq!(thumbnail_ready_flag(&db, "nas-1"), 0);
    assert_eq!(
        notifier.0.load(Ordering::SeqCst),
        1,
        "a failed generation also notifies the catalog changed"
    );
}

#[test]
fn workers_racing_on_one_unprobed_folder_probe_it_once() {
    let (_db_dir, db) = init_temp_db();
    let thumbs = tempfile::tempdir().unwrap();
    let ids: Vec<String> = (0..8).map(|i| format!("nas-{i}")).collect();
    for id in &ids {
        insert_test_video(&db, id, &format!("W:/video/{id}.mp4"));
    }
    let probe =
        Arc::new(FakeFolderProbe::with_unreachable(&[NAS]).with_delay(Duration::from_millis(100)));

    process_pending(
        db.clone(),
        thumbs.path().to_path_buf(),
        watch_folders(),
        ThumbnailQueueHandle::default(),
        failing_ffmpeg(),
        Arc::new(CountingNotifier(AtomicUsize::new(0))),
        ids.iter().map(|id| nas_entry(id)).collect(),
        registry_over(probe.clone()),
    );

    assert_eq!(probe.call_count_for(NAS), 1);
    for id in &ids {
        assert_eq!(thumbnail_attempts_of(&db, id), 0);
    }
}

#[test]
fn each_successful_generation_notifies_the_catalog_once() {
    let (_db_dir, db) = init_temp_db();
    let thumbs = tempfile::tempdir().unwrap();
    for id in ["ok-1", "ok-2"] {
        insert_test_video(&db, id, &format!("C:/videos/{id}.mp4"));
    }
    let notifier = Arc::new(CountingNotifier(AtomicUsize::new(0)));

    process_pending(
        db.clone(),
        thumbs.path().to_path_buf(),
        Vec::new(),
        ThumbnailQueueHandle::default(),
        available_ffmpeg(Duration::ZERO),
        Arc::clone(&notifier),
        vec![pending_entry("ok-1"), pending_entry("ok-2")],
        all_reachable(),
    );

    assert_eq!(notifier.0.load(Ordering::SeqCst), 2);
}
