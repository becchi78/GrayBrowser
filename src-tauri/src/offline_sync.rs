//! Keeps the stored `videos.status` in step with folder reachability.
//!
//! When a watch folder is confirmed unreachable (renamed, unplugged, share
//! down) every `online` video under it is flipped to `offline`, so the catalog
//! says what the screens show and a file moved elsewhere is followed instead of
//! registered a second time. When the folder is back, its regular diff scan
//! runs: it re-checks each file on disk and brings back only the rows whose
//! file is really there. Nothing here sets rows `online` directly.
//!
//! The reachability hooks run on whichever thread finished a probe, possibly
//! one that others wait behind, so they only enqueue a job. One worker thread
//! runs the jobs in the order they arrived, which keeps a down, up, down
//! sequence for one folder from being applied out of order. A queued
//! "unreachable" is also checked against the registry just before it is
//! applied, so a notification that went stale while it waited (the folder came
//! back, or was removed from the watch list) writes nothing.
//!
//! Jobs are not run in parallel on purpose: a recovery scan of a large share
//! can take a while and delays the jobs behind it. Disconnections are rare, so
//! that is cheaper than guarding every folder's writes against each other.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::db::{queries, Db};
use crate::events::CatalogNotifier;
use crate::reachability::ReachabilityRegistry;
use crate::watch::follow_up::FollowUp;
use crate::watch::nas_poll::run_nas_diff_scan;

enum Job {
    Unreachable(String),
    Recovered(String),
}

/// What the worker needs; bundled so `FolderStateSync::start` stays readable.
pub struct SyncContext {
    pub db: Db,
    pub thumbnails_root: PathBuf,
    pub notifier: Arc<dyn CatalogNotifier>,
    pub follow_up: Arc<FollowUp>,
    pub reachability: ReachabilityRegistry,
    /// Re-runs duplicate detection. A closure because the real one needs
    /// Tauri state that the tests do not have.
    pub refresh_duplicates: Arc<dyn Fn() + Send + Sync>,
}

/// Handle to the worker. Cheap to share; the worker exits when it is dropped.
pub struct FolderStateSync {
    // `Sender` is not `Sync` on every supported toolchain.
    jobs: Mutex<Sender<Job>>,
}

impl FolderStateSync {
    pub fn start(ctx: SyncContext) -> Self {
        let (tx, rx) = channel::<Job>();
        thread::Builder::new()
            .name("offline-sync".into())
            .spawn(move || {
                for job in rx {
                    // One bad job must not end the only worker.
                    if catch_unwind(AssertUnwindSafe(|| run_job(&ctx, &job))).is_err() {
                        log::error!("folder state sync job panicked");
                    }
                }
            })
            .expect("failed to start the folder state sync worker");
        Self {
            jobs: Mutex::new(tx),
        }
    }

    /// Queues "flip the rows under `folder` offline". Returns at once.
    pub fn on_unreachable(&self, folder: &str) {
        self.send(Job::Unreachable(folder.to_string()));
    }

    /// Queues "scan `folder` and bring its files back online". Returns at once.
    pub fn on_recovered(&self, folder: &str) {
        self.send(Job::Recovered(folder.to_string()));
    }

    fn send(&self, job: Job) {
        let tx = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if tx.send(job).is_err() {
            log::warn!("folder state sync worker is gone; dropping a job");
        }
    }
}

fn run_job(ctx: &SyncContext, job: &Job) {
    match job {
        Job::Unreachable(folder) => {
            // Confirmed when queued, but the answer can have changed since. A
            // newer probe's provisional flag does not count: its answer may
            // retract it, and no recovery would follow to undo the write.
            if !ctx.reachability.is_settled_unreachable(folder) {
                log::info!("not marking {folder} offline: it is no longer unreachable");
                return;
            }
            match mark_folder_offline(&ctx.db, folder) {
                Ok(0) => {}
                Ok(changed) => {
                    log::info!(
                        "{changed} video(s) under {folder} went offline: folder unreachable"
                    );
                    ctx.notifier.notify_changed();
                    (ctx.refresh_duplicates)();
                }
                Err(e) => log::error!("failed to mark videos under {folder} offline: {e:#}"),
            }
        }
        Job::Recovered(folder) => {
            if let Err(e) = run_nas_diff_scan(
                &ctx.db,
                &ctx.thumbnails_root,
                folder,
                &AtomicBool::new(false),
                ctx.notifier.as_ref(),
                &ctx.follow_up,
                &ctx.reachability,
            ) {
                log::error!("scan after {folder} came back failed: {e:#}");
            }
            // The scan notifies per change it makes; this one also covers rows
            // it reactivated without a notification of their own.
            ctx.notifier.notify_changed();
            (ctx.refresh_duplicates)();
        }
    }
}

/// Flips every `online` video under `folder` to `offline` in one transaction.
/// Returns how many rows changed.
pub fn mark_folder_offline(db: &Db, folder: &str) -> anyhow::Result<usize> {
    let ids: Vec<String> = queries::list_online_videos_under(&db.read_pool, folder)?
        .into_iter()
        .map(|v| v.id)
        .collect();
    if ids.is_empty() {
        return Ok(0);
    }
    let mut conn = db.writer.lock().unwrap_or_else(|e| e.into_inner());
    Ok(queries::mark_videos_offline(&mut conn, &ids)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::init_temp_db;
    use crate::events::{FakeCatalogNotifier, NoopReachabilityNotifier};
    use crate::scan::mtime_from_metadata;
    use gb_core::ports::folder_probe::FolderProbe;
    use gb_core::testing::fake_folder_probe::FakeFolderProbe;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Condvar;
    use std::time::Duration;

    fn insert(db: &Db, id: &str, path: &str, status: &str, size: i64, mtime: i64, hash: &str) {
        let conn = db.writer.lock().unwrap();
        conn.execute(
            "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status, mtime)
             VALUES (?1, ?2, 'v.mp4', ?3, ?4, ?5, ?6)",
            rusqlite::params![id, path, size, hash, status, mtime],
        )
        .unwrap();
    }

    fn status_of(db: &Db, id: &str) -> String {
        let conn = db.read_pool.get().unwrap();
        conn.query_row("SELECT status FROM videos WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .unwrap()
    }

    fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..400 {
            if cond() {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        cond()
    }

    struct Harness {
        db: Db,
        _db_dir: tempfile::TempDir,
        _thumbs: tempfile::TempDir,
        probe: Arc<FakeFolderProbe>,
        registry: ReachabilityRegistry,
        notifier: Arc<FakeCatalogNotifier>,
        dedup_runs: Arc<AtomicUsize>,
        sync: FolderStateSync,
    }

    fn harness(unreachable: &[&str]) -> Harness {
        let probe = Arc::new(FakeFolderProbe::with_unreachable(unreachable));
        let registry = ReachabilityRegistry::new(probe.clone(), Arc::new(NoopReachabilityNotifier));
        // Prime the registry so the worker's "still unreachable?" check sees
        // the state a real probe would have recorded.
        for folder in unreachable {
            registry.is_reachable(folder);
        }
        harness_over(probe, registry, |_| Arc::new(|| {}))
    }

    /// `on_dedup` builds what runs inside the duplicate-refresh step, which
    /// the worker reaches at the end of a job; tests use it to pause or steer
    /// the worker at a known point.
    fn harness_over(
        probe: Arc<FakeFolderProbe>,
        registry: ReachabilityRegistry,
        on_dedup: impl FnOnce(&Db) -> Arc<dyn Fn() + Send + Sync>,
    ) -> Harness {
        let (db_dir, db) = init_temp_db();
        let thumbs = tempfile::tempdir().unwrap();
        let on_dedup = on_dedup(&db);
        let notifier = Arc::new(FakeCatalogNotifier::default());
        let dedup_runs = Arc::new(AtomicUsize::new(0));
        let sync = FolderStateSync::start(SyncContext {
            db: db.clone(),
            thumbnails_root: thumbs.path().to_path_buf(),
            notifier: notifier.clone(),
            follow_up: Arc::new(FollowUp::noop()),
            reachability: registry.clone(),
            refresh_duplicates: {
                let dedup_runs = dedup_runs.clone();
                Arc::new(move || {
                    dedup_runs.fetch_add(1, Ordering::SeqCst);
                    on_dedup();
                })
            },
        });
        Harness {
            db,
            _db_dir: db_dir,
            _thumbs: thumbs,
            probe,
            registry,
            notifier,
            dedup_runs,
            sync,
        }
    }

    #[test]
    fn only_online_rows_under_the_folder_go_offline() {
        let h = harness(&["D:\\video"]);
        insert(&h.db, "in", "D:\\video\\a.mp4", "online", 1, 1, "h1");
        insert(&h.db, "deep", "d:\\VIDEO\\sub\\b.mp4", "online", 1, 1, "h2");
        insert(&h.db, "sibling", "D:\\video2\\c.mp4", "online", 1, 1, "h3");
        insert(&h.db, "other", "E:\\video\\d.mp4", "online", 1, 1, "h4");
        insert(&h.db, "already", "D:\\video\\e.mp4", "offline", 1, 1, "h5");
        insert(&h.db, "placeholder", "::label\\f.mp4", "offline", 0, 0, "");

        h.sync.on_unreachable("D:\\video");

        assert!(wait_until(|| h.dedup_runs.load(Ordering::SeqCst) == 1));
        assert_eq!(status_of(&h.db, "in"), "offline");
        assert_eq!(status_of(&h.db, "deep"), "offline");
        assert_eq!(status_of(&h.db, "sibling"), "online");
        assert_eq!(status_of(&h.db, "other"), "online");
        assert_eq!(status_of(&h.db, "already"), "offline");
        assert_eq!(status_of(&h.db, "placeholder"), "offline");
        assert_eq!(h.notifier.calls(), 1);
    }

    #[test]
    fn a_folder_with_nothing_online_notifies_nobody() {
        let h = harness(&["D:\\empty", "D:\\video"]);
        insert(&h.db, "off", "D:\\empty\\a.mp4", "offline", 1, 1, "h1");
        insert(&h.db, "in", "D:\\video\\b.mp4", "online", 1, 1, "h2");

        h.sync.on_unreachable("D:\\empty");
        // Jobs run in order, so this finishing means the first one did too.
        h.sync.on_unreachable("D:\\video");

        assert!(wait_until(|| h.dedup_runs.load(Ordering::SeqCst) == 1));
        assert_eq!(h.notifier.calls(), 1, "only the second folder changed rows");
    }

    #[test]
    fn a_notification_gone_stale_writes_nothing() {
        // D:\video is reachable by the time the job runs.
        let h = harness(&["D:\\other"]);
        h.registry.is_reachable("D:\\video");
        insert(&h.db, "in", "D:\\video\\a.mp4", "online", 1, 1, "h1");
        insert(&h.db, "o", "D:\\other\\a.mp4", "online", 1, 1, "h2");

        h.sync.on_unreachable("D:\\video");
        h.sync.on_unreachable("D:\\other");

        assert!(wait_until(|| h.dedup_runs.load(Ordering::SeqCst) == 1));
        assert_eq!(status_of(&h.db, "in"), "online");
        assert_eq!(status_of(&h.db, "o"), "offline");
    }

    #[test]
    fn a_forgotten_folder_is_not_marked_offline() {
        let h = harness(&["D:\\video", "D:\\other"]);
        insert(&h.db, "in", "D:\\video\\a.mp4", "online", 1, 1, "h1");
        insert(&h.db, "o", "D:\\other\\a.mp4", "online", 1, 1, "h2");
        h.registry.retain_folders(&["D:\\other".to_string()]);

        h.sync.on_unreachable("D:\\video");
        h.sync.on_unreachable("D:\\other");

        assert!(wait_until(|| h.dedup_runs.load(Ordering::SeqCst) == 1));
        assert_eq!(status_of(&h.db, "in"), "online");
        assert_eq!(status_of(&h.db, "o"), "offline");
    }

    #[test]
    fn more_rows_than_one_statement_can_bind_all_go_offline() {
        let h = harness(&["D:\\video"]);
        {
            let mut conn = h.db.writer.lock().unwrap();
            let tx = conn.transaction().unwrap();
            for i in 0..2500 {
                tx.execute(
                    "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status, mtime)
                     VALUES (?1, ?2, 'v.mp4', 1, ?3, 'online', 1)",
                    rusqlite::params![format!("id{i}"), format!("D:\\video\\{i}.mp4"), format!("h{i}")],
                )
                .unwrap();
            }
            tx.commit().unwrap();
        }
        insert(&h.db, "sibling", "D:\\video2\\x.mp4", "online", 1, 1, "hx");

        h.sync.on_unreachable("D:\\video");

        assert!(wait_until(|| h.dedup_runs.load(Ordering::SeqCst) == 1));
        let conn = h.db.read_pool.get().unwrap();
        let online: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM videos WHERE status = 'online'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(online, 1, "only the sibling folder's row stays online");
    }

    #[test]
    fn a_folder_coming_back_brings_back_only_files_that_are_there() {
        let h = harness(&[]);
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().to_string_lossy().to_string();
        let present = dir.path().join("present.mp4");
        std::fs::write(&present, b"bytes").unwrap();
        let meta = std::fs::metadata(&present).unwrap();
        let mtime = mtime_from_metadata(&meta).unwrap();
        let path = |name: &str| dir.path().join(name).to_string_lossy().to_string();
        // Same size and mtime as on disk: the scan sees "unchanged", which is
        // exactly the case that used to leave an offline row offline.
        insert(
            &h.db,
            "present",
            &path("present.mp4"),
            "offline",
            meta.len() as i64,
            mtime,
            "h1",
        );
        insert(&h.db, "gone", &path("gone.mp4"), "offline", 5, 1, "h2");
        insert(&h.db, "placeholder", "::label\\f.mp4", "offline", 0, 0, "");

        h.sync.on_recovered(&folder);

        assert!(wait_until(|| h.dedup_runs.load(Ordering::SeqCst) == 1));
        assert_eq!(status_of(&h.db, "present"), "online");
        assert_eq!(status_of(&h.db, "gone"), "offline");
        assert_eq!(status_of(&h.db, "placeholder"), "offline");
        assert!(h.notifier.calls() >= 1);
    }

    #[test]
    fn a_recovery_scan_of_a_folder_that_is_down_again_changes_nothing() {
        let h = harness(&[]);
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().to_string_lossy().to_string();
        let file = dir.path().join("a.mp4");
        std::fs::write(&file, b"bytes").unwrap();
        let meta = std::fs::metadata(&file).unwrap();
        insert(
            &h.db,
            "a",
            &file.to_string_lossy(),
            "offline",
            meta.len() as i64,
            mtime_from_metadata(&meta).unwrap(),
            "h1",
        );
        h.probe.set_reachable(&folder, false);

        h.sync.on_recovered(&folder);

        assert!(wait_until(|| h.dedup_runs.load(Ordering::SeqCst) == 1));
        assert_eq!(status_of(&h.db, "a"), "offline");
    }

    /// Probes of one folder block while the gate is closed and answer
    /// "unreachable" at once while it is open; once a blocked probe is let go
    /// it answers "reachable". Every other folder answers "unreachable".
    struct BlockingProbe {
        blocked: String,
        closed: Mutex<bool>,
        cond: Condvar,
        waiting: AtomicUsize,
    }

    impl BlockingProbe {
        fn new(blocked: &str, closed: bool) -> Arc<Self> {
            Arc::new(Self {
                blocked: blocked.into(),
                closed: Mutex::new(closed),
                cond: Condvar::new(),
                waiting: AtomicUsize::new(0),
            })
        }

        fn close(&self) {
            *self.closed.lock().unwrap() = true;
        }

        fn release(&self) {
            *self.closed.lock().unwrap() = false;
            self.cond.notify_all();
        }
    }

    impl FolderProbe for BlockingProbe {
        fn is_reachable(&self, folder: &str) -> bool {
            if folder != self.blocked {
                return false;
            }
            let closed = self.closed.lock().unwrap();
            if !*closed {
                return false;
            }
            self.waiting.fetch_add(1, Ordering::SeqCst);
            drop(self.cond.wait_while(closed, |c| *c).unwrap());
            true
        }
    }

    fn short_provisional_registry(probe: &Arc<BlockingProbe>) -> ReachabilityRegistry {
        ReachabilityRegistry::with_provisional_after(
            probe.clone(),
            Arc::new(NoopReachabilityNotifier),
            Duration::from_millis(30),
        )
    }

    fn is_published(registry: &ReachabilityRegistry, folder: &str) -> bool {
        registry.unreachable_folders().contains(&folder.to_string())
    }

    #[test]
    fn a_folder_shown_unreachable_only_provisionally_is_not_marked_offline() {
        let probe = BlockingProbe::new("D:\\video", true);
        let registry = short_provisional_registry(&probe);
        assert!(!registry.is_reachable("D:\\other"));
        let slow = {
            let registry = registry.clone();
            thread::spawn(move || registry.is_reachable("D:\\video"))
        };
        assert!(wait_until(|| is_published(&registry, "D:\\video")));
        let h = harness_over(Arc::new(FakeFolderProbe::default()), registry, |_| {
            Arc::new(|| {})
        });
        insert(&h.db, "in", "D:\\video\\a.mp4", "online", 1, 1, "h1");
        insert(&h.db, "o", "D:\\other\\a.mp4", "online", 1, 1, "h2");

        h.sync.on_unreachable("D:\\video");
        h.sync.on_unreachable("D:\\other");

        // Jobs run in order, so the second one finishing means the first one
        // has been decided.
        assert!(wait_until(|| h.dedup_runs.load(Ordering::SeqCst) == 1));
        assert_eq!(status_of(&h.db, "in"), "online");
        assert_eq!(status_of(&h.db, "o"), "offline");

        probe.release();
        assert!(slow.join().unwrap());
    }

    #[test]
    fn a_forgotten_folder_probed_again_and_shown_unreachable_is_not_marked_offline() {
        let probe = BlockingProbe::new("D:\\video", false);
        let registry = short_provisional_registry(&probe);
        assert!(!registry.is_reachable("D:\\video"));
        assert!(!registry.is_reachable("D:\\other"));
        registry.retain_folders(&["D:\\other".to_string()]);
        probe.close();
        let slow = {
            let registry = registry.clone();
            thread::spawn(move || registry.is_reachable("D:\\video"))
        };
        assert!(wait_until(|| is_published(&registry, "D:\\video")));
        let h = harness_over(Arc::new(FakeFolderProbe::default()), registry, |_| {
            Arc::new(|| {})
        });
        insert(&h.db, "in", "D:\\video\\a.mp4", "online", 1, 1, "h1");
        insert(&h.db, "o", "D:\\other\\a.mp4", "online", 1, 1, "h2");

        h.sync.on_unreachable("D:\\video");
        h.sync.on_unreachable("D:\\other");

        assert!(wait_until(|| h.dedup_runs.load(Ordering::SeqCst) == 1));
        assert_eq!(status_of(&h.db, "in"), "online");

        probe.release();
        assert!(slow.join().unwrap());
    }

    #[test]
    fn a_folder_being_rechecked_after_going_down_is_still_marked_offline() {
        let probe = BlockingProbe::new("D:\\video", false);
        let registry = short_provisional_registry(&probe);
        assert!(!registry.is_reachable("D:\\video"));
        probe.close();
        let recheck = {
            let registry = registry.clone();
            thread::spawn(move || registry.refresh("D:\\video"))
        };
        assert!(wait_until(|| probe.waiting.load(Ordering::SeqCst) == 1));
        let h = harness_over(Arc::new(FakeFolderProbe::default()), registry, |_| {
            Arc::new(|| {})
        });
        insert(&h.db, "in", "D:\\video\\a.mp4", "online", 1, 1, "h1");

        h.sync.on_unreachable("D:\\video");

        assert!(wait_until(|| status_of(&h.db, "in") == "offline"));

        probe.release();
        assert!(recheck.join().unwrap());
    }

    #[test]
    fn jobs_queued_together_for_one_folder_apply_in_the_order_they_were_queued() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().to_string_lossy().to_string();
        let file = dir.path().join("a.mp4");
        std::fs::write(&file, b"bytes").unwrap();
        let meta = std::fs::metadata(&file).unwrap();

        let probe = Arc::new(FakeFolderProbe::with_unreachable(&[folder.as_str()]));
        let registry = ReachabilityRegistry::new(probe.clone(), Arc::new(NoopReachabilityNotifier));
        registry.refresh(&folder);

        // The first job stops in its duplicate-refresh step, with the worker
        // held there while the other two are queued. The second job's step
        // takes the folder down again, so that the third job is current when
        // it runs; the status seen at each step shows the order.
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let h = harness_over(probe.clone(), registry.clone(), {
            let (gate, seen) = (gate.clone(), seen.clone());
            let (probe, registry, folder) = (probe.clone(), registry.clone(), folder.clone());
            move |db| {
                let db = db.clone();
                Arc::new(move || {
                    let step = {
                        let mut seen = seen.lock().unwrap();
                        seen.push(status_of(&db, "a"));
                        seen.len()
                    };
                    if step == 1 {
                        let (open, cond) = &*gate;
                        drop(cond.wait_while(open.lock().unwrap(), |o| !*o).unwrap());
                    } else if step == 2 {
                        probe.set_reachable(&folder, false);
                        registry.refresh(&folder);
                    }
                })
            }
        });
        insert(
            &h.db,
            "a",
            &file.to_string_lossy(),
            "online",
            meta.len() as i64,
            mtime_from_metadata(&meta).unwrap(),
            "h1",
        );

        h.sync.on_unreachable(&folder);
        assert!(wait_until(|| h.dedup_runs.load(Ordering::SeqCst) == 1));
        probe.set_reachable(&folder, true);
        h.sync.on_recovered(&folder);
        h.sync.on_unreachable(&folder);
        // All three are queued before the worker is let go.
        assert_eq!(h.dedup_runs.load(Ordering::SeqCst), 1);
        {
            let (open, cond) = &*gate;
            *open.lock().unwrap() = true;
            cond.notify_all();
        }

        assert!(wait_until(|| h.dedup_runs.load(Ordering::SeqCst) == 3));
        assert_eq!(*seen.lock().unwrap(), vec!["offline", "online", "offline"]);
    }
}
