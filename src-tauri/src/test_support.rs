//! Helpers shared by the in-crate `#[cfg(test)]` modules. Database fixtures
//! live in `db::test_support`; this module holds what is not tied to the
//! database: reachability fixtures, polling, gates and small value builders.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use gb_core::ports::folder_probe::FolderProbe;
use gb_core::testing::fake_folder_probe::FakeFolderProbe;

use crate::db::Db;
use crate::events::{CatalogNotifier, NoopReachabilityNotifier};
use crate::reachability::ReachabilityRegistry;

/// A registry over `probe` whose reachability changes are not observed.
pub fn registry_over(probe: Arc<dyn FolderProbe>) -> ReachabilityRegistry {
    ReachabilityRegistry::new(probe, Arc::new(NoopReachabilityNotifier))
}

/// A registry in which every folder is reachable.
pub fn all_reachable() -> ReachabilityRegistry {
    registry_over(Arc::new(FakeFolderProbe::default()))
}

/// Upper bound for [`wait_until`]. A passing test returns as soon as the
/// condition holds, so this only bounds how long a failing one hangs.
const WAIT_BUDGET: Duration = Duration::from_secs(5);
/// Longest a thread waits at a closed [`Gate`].
const GATE_LIMIT: Duration = Duration::from_secs(20);
const WAIT_STEP: Duration = Duration::from_millis(10);

/// Polls `cond` until it holds or the default budget runs out; returns the
/// last answer.
pub fn wait_until(cond: impl FnMut() -> bool) -> bool {
    wait_until_within(WAIT_BUDGET, cond)
}

/// Like [`wait_until`] with an explicit budget, for tests whose point is that
/// something happens well before a longer default timer would fire.
pub fn wait_until_within(budget: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + budget;
    loop {
        if cond() {
            return true;
        }
        if Instant::now() >= deadline {
            return cond();
        }
        thread::sleep(WAIT_STEP);
    }
}

/// Counts `notify_changed` calls.
pub struct CountingNotifier(pub AtomicUsize);

impl CatalogNotifier for CountingNotifier {
    fn notify_changed(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// A closable gate: threads that reach `pass` while it is closed block until
/// `release`. Lets a test decide exactly when a blocked step may continue
/// instead of racing a sleep against it.
pub struct Gate {
    closed: Mutex<bool>,
    cond: Condvar,
    waiting: AtomicUsize,
}

impl Gate {
    pub fn new(closed: bool) -> Self {
        Self {
            closed: Mutex::new(closed),
            cond: Condvar::new(),
            waiting: AtomicUsize::new(0),
        }
    }

    pub fn close(&self) {
        *self.closed.lock().unwrap() = true;
    }

    pub fn release(&self) {
        *self.closed.lock().unwrap() = false;
        self.cond.notify_all();
    }

    /// How many calls have reached a closed gate so far (it is not
    /// decremented when they are let go).
    pub fn waiting(&self) -> usize {
        self.waiting.load(Ordering::SeqCst)
    }

    /// Returns at once if the gate is open; otherwise blocks until released
    /// and returns `true`. The lock is not held on return, so the caller may
    /// panic afterwards without poisoning it.
    pub fn pass(&self) -> bool {
        let closed = self.closed.lock().unwrap();
        if !*closed {
            return false;
        }
        self.waiting.fetch_add(1, Ordering::SeqCst);
        // Bounded, so a gate nobody opens fails the test that depends on it
        // instead of hanging the whole run.
        drop(
            self.cond
                .wait_timeout_while(closed, GATE_LIMIT, |c| *c)
                .unwrap(),
        );
        true
    }
}

pub fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

pub fn thumbnail_attempts_of(db: &Db, id: &str) -> i64 {
    db.writer
        .lock()
        .unwrap()
        .query_row(
            "SELECT thumbnail_attempts FROM videos WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .unwrap()
}

/// The `io::Error` text a rename of a plain file onto `blocker` (an existing
/// directory) produces on this machine, so expected messages don't hard-code
/// an OS- and locale-dependent string.
pub fn rename_onto_dir_error_text(blocker: &Path) -> String {
    let probe_dir = tempfile::tempdir().unwrap();
    let probe = probe_dir.path().join("probe.tmp");
    std::fs::write(&probe, b"probe").unwrap();
    std::fs::rename(&probe, blocker).unwrap_err().to_string()
}

/// A temp directory holding a database, one registered watch folder
/// (`folder`, created) and a thumbnails root (`thumbs`, not created), the
/// common starting point of the command-layer tests that touch real files.
pub struct LibraryEnv {
    // Kept alive so the temp directory (DB, files) outlives the test.
    pub dir: tempfile::TempDir,
    pub db: Db,
    pub folder: PathBuf,
    pub thumbs: PathBuf,
}

impl LibraryEnv {
    pub fn new() -> Self {
        let (dir, db) = crate::db::test_support::init_temp_db();
        let folder = dir.path().join("lib");
        std::fs::create_dir_all(&folder).unwrap();
        let thumbs = dir.path().join("thumbs");
        {
            let conn = db.writer.lock().unwrap();
            crate::db::queries::set_watch_folders(&conn, &[folder.to_string_lossy().into_owned()])
                .unwrap();
        }
        Self {
            dir,
            db,
            folder,
            thumbs,
        }
    }

    /// Inserts a row for `path` (`file_name` taken from it, `quick_hash` `h`).
    pub fn insert_row(&self, id: &str, path: &Path, status: &str, size: i64) {
        let conn = self.db.writer.lock().unwrap();
        conn.execute(
            "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status)
             VALUES (?1, ?2, ?3, ?4, 'h', ?5)",
            rusqlite::params![
                id,
                path.to_string_lossy(),
                path.file_name().unwrap().to_string_lossy(),
                size,
                status
            ],
        )
        .unwrap();
    }

    /// A real file with `content` inside the registered folder plus its row.
    pub fn add_video_with(&self, id: &str, status: &str, content: &[u8]) -> PathBuf {
        let path = self.folder.join(format!("{id}.mp4"));
        std::fs::write(&path, content).unwrap();
        self.insert_row(id, &path, status, content.len() as i64);
        path
    }

    pub fn add_tag(&self, id: &str) {
        let mut conn = self.db.writer.lock().unwrap();
        crate::db::queries::assign_tag_to_video(&mut conn, id, "tag").unwrap();
    }

    pub fn row_exists(&self, id: &str) -> bool {
        crate::db::queries::find_video_by_id(&self.db.read_pool, id)
            .unwrap()
            .is_some()
    }

    pub fn tag_count(&self, id: &str) -> i64 {
        let conn = self.db.read_pool.get().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM video_tags WHERE video_id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    }
}
