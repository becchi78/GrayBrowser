//! Local realtime file-watching orchestration. Wraps
//! `gb_core::ports::watcher::FileWatcher` and routes normalized events into
//! `crate::scan::process_detected_file`, the same shared entry point the
//! manual scan uses.
//!
//! `RealtimeWatchManager::reconfigure` classifies each watched folder by
//! drive type: local/removable folders get realtime `notify`-based
//! watching, network folders get `nas_poll`'s startup diff-scan + polling
//! instead. `reconfigure` is called once at startup and again whenever
//! `pick_watch_folders` changes the folder list, so a newly added folder is
//! picked up without an app restart.

pub mod follow_up;
pub mod nas_poll;
pub mod unreadable_retry;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gb_core::ports::drive_type::{DriveKind, DriveTypeDetector};
use gb_core::ports::watcher::{FileWatcher, WatchEvent, WatchEventKind, WatchHandle};
use gb_core::reconcile;
use gb_core::scan_pipeline;

use crate::adapters::long_path;
use crate::db::{queries, Db};
use crate::events::CatalogNotifier;
use crate::scan::{self, mtime_from_metadata};
use follow_up::FollowUp;
use nas_poll::NasPollHandle;
use unreadable_retry::{RetryOutcome, UnreadableRetry};

/// Holds the live watch subscriptions/pollers for as long as the app runs.
/// Managed via `app.manage()`. No explicit shutdown wiring at process exit:
/// dropping this drops every `WatchHandle` (whose `Drop`, through the
/// `notify` watcher it owns, unregisters the OS-level watch);
/// `NasPollHandle`s left un-stopped at process exit are
/// simply killed along with their thread by the OS, same reasoning.
/// Reconfiguration mid-run (folder list changes) goes through `reconfigure`
/// instead, which explicitly stops the old set before starting the new one.
/// Bundles `Db` and the `thumbnails/` root together for `RealtimeWatchManager::
/// reconfigure` -- keeps its own parameter count under
/// `clippy::too_many_arguments`'s default threshold now that `thumbnails_root`
/// sits alongside the folder-list/watcher/drive-type/notifier parameters
/// already there, mirroring `wb_import::pipeline::WbImportPaths`'s reason for
/// bundling.
pub struct WatchContext {
    pub db: Db,
    pub thumbnails_root: PathBuf,
}

pub struct RealtimeWatchManager {
    handles: Mutex<Vec<Box<dyn WatchHandle>>>,
    nas_pollers: Mutex<Vec<NasPollHandle>>,
    /// One-shot startup scans of local/removable folders. Kept apart from
    /// `nas_pollers` so that count keeps meaning "network folders polled";
    /// stopped by the next `reconfigure` the same way.
    startup_scans: Mutex<Vec<NasPollHandle>>,
    /// The retry worker of the current watch set; dropping it (on the next
    /// `reconfigure`) stops its thread.
    unreadable_retry: Mutex<Option<Arc<UnreadableRetry>>>,
    follow_up: Arc<FollowUp>,
    /// Held for the whole of `reconfigure`. Commands that change the folder
    /// list or the per-folder check setting run concurrently, and the per-field
    /// locks above only protect each field: two interleaved reconfigures could
    /// overwrite each other's handles and leave duplicate watchers/pollers.
    reconfigure_lock: Mutex<()>,
}

impl Default for RealtimeWatchManager {
    fn default() -> Self {
        Self::with_follow_up(Arc::new(FollowUp::noop()))
    }
}

impl RealtimeWatchManager {
    pub fn with_follow_up(follow_up: Arc<FollowUp>) -> Self {
        Self {
            handles: Mutex::default(),
            nas_pollers: Mutex::default(),
            startup_scans: Mutex::default(),
            unreadable_retry: Mutex::default(),
            follow_up,
            reconfigure_lock: Mutex::new(()),
        }
    }

    /// Stops whatever this manager currently holds, classifies each folder
    /// in `folders` by drive type, and starts a fresh set: local/removable
    /// folders get realtime `notify` watching (via `start_watching`),
    /// network folders get `nas_poll::start_nas_polling`. A
    /// folder whose drive type can't be determined (`Unknown` or an `Err`
    /// from `drive_type`) falls back to polling -- the safer default, since
    /// polling still eventually detects changes, while assuming "local"
    /// for an actually-remote folder would just never fire at all.
    ///
    /// Local/removable folders additionally get one diff scan right after the
    /// watch is registered, to pick up files added while the app was closed.
    /// Since `reconfigure` runs on every folder-list change, that rescans all
    /// local folders each time; the mtime/size filter keeps it cheap.
    pub fn reconfigure<N: CatalogNotifier + 'static>(
        &self,
        ctx: WatchContext,
        folders: &[String],
        watcher: &impl FileWatcher,
        drive_type: &impl DriveTypeDetector,
        nas_poll_interval: Duration,
        notifier: Arc<N>,
    ) {
        let _serialized = self
            .reconfigure_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let WatchContext {
            db,
            thumbnails_root,
        } = ctx;

        for mut handle in self.handles.lock().unwrap().drain(..) {
            handle.stop();
        }
        for poller in self.nas_pollers.lock().unwrap().drain(..) {
            poller.stop();
        }
        for scan in self.startup_scans.lock().unwrap().drain(..) {
            scan.stop();
        }
        self.unreadable_retry.lock().unwrap().take();

        let mut local_folders = Vec::new();
        let mut nas_folders = Vec::new();
        for folder in folders {
            match drive_type.detect(Path::new(folder)) {
                Ok(DriveKind::Local | DriveKind::Removable) => local_folders.push(folder.clone()),
                Ok(DriveKind::Network) => nas_folders.push(folder.clone()),
                Ok(DriveKind::Unknown) => {
                    log::warn!("drive type unknown for {folder}, falling back to NAS polling");
                    nas_folders.push(folder.clone());
                }
                Err(e) => {
                    log::warn!("failed to detect drive type for {folder}: {e}, falling back to NAS polling");
                    nas_folders.push(folder.clone());
                }
            }
        }

        let (new_handles, new_retry) = start_watching(
            db.clone(),
            thumbnails_root.clone(),
            &local_folders,
            watcher,
            Arc::clone(&notifier),
            Arc::clone(&self.follow_up),
        );
        // After start_watching, so a file created between the scan's walk and
        // the watch becoming active cannot be missed.
        let new_scans: Vec<NasPollHandle> = local_folders
            .into_iter()
            .map(|folder| {
                nas_poll::start_one_shot_diff_scan(
                    db.clone(),
                    thumbnails_root.clone(),
                    folder,
                    Arc::clone(&notifier),
                    Arc::clone(&self.follow_up),
                )
            })
            .collect();
        let new_pollers: Vec<NasPollHandle> = nas_folders
            .into_iter()
            .map(|folder| {
                nas_poll::start_nas_polling(
                    db.clone(),
                    thumbnails_root.clone(),
                    folder,
                    nas_poll_interval,
                    Arc::clone(&notifier),
                    Arc::clone(&self.follow_up),
                )
            })
            .collect();

        *self.handles.lock().unwrap() = new_handles;
        *self.nas_pollers.lock().unwrap() = new_pollers;
        *self.startup_scans.lock().unwrap() = new_scans;
        *self.unreadable_retry.lock().unwrap() = Some(new_retry);
    }

    pub fn active_handle_count(&self) -> usize {
        self.handles.lock().unwrap().len()
    }

    pub fn active_nas_poller_count(&self) -> usize {
        self.nas_pollers.lock().unwrap().len()
    }
}

/// Thin wiring helper around `RealtimeWatchManager::reconfigure`: bundles
/// this app's real adapters (`RealFileWatcher`, `RealDriveTypeDetector`,
/// `TauriCatalogNotifier`) and the `nas_poll_interval_secs -> Duration`
/// conversion (a negative value, which shouldn't occur but isn't validated
/// against at the DB layer, is clamped to 0 rather than panicking on the
/// `as u64` cast).
///
/// Previously this exact wiring -- adapters and all -- was copy-pasted at
/// every call site: `lib.rs`'s startup `.setup()` and each of
/// `commands::settings_cmds`'s three watch-folder-mutating commands
/// (`pick_watch_folders`/`remove_watch_folder`/`rename_watch_folder`).
///
/// `folders` is narrowed here to the folders whose update check is on, so every
/// caller gets the same behavior without passing the disabled list around.
/// Manual scans read `get_watch_folders` directly and so still cover the
/// disabled folders.
pub fn reconfigure_real_watch_manager(
    app: &tauri::AppHandle,
    db: &Db,
    watch_manager: &RealtimeWatchManager,
    thumbnails_root: &Path,
    folders: &[String],
    nas_poll_interval_secs: i64,
) {
    // If the disabled list can't be read, watch everything: missing a new
    // video is worse than checking a folder the user switched off.
    let disabled = db
        .read_pool
        .get()
        .map_err(anyhow::Error::from)
        .and_then(|conn| crate::db::queries::get_watch_check_disabled(&conn))
        .unwrap_or_else(|e| {
            log::warn!("failed to read the update-check-disabled folders; checking all: {e}");
            Vec::new()
        });
    let checked = gb_core::watch_folders::filter_checked(folders, &disabled);
    watch_manager.reconfigure(
        WatchContext {
            db: db.clone(),
            thumbnails_root: thumbnails_root.to_path_buf(),
        },
        &checked,
        &crate::adapters::watcher::RealFileWatcher,
        &crate::adapters::drive_type::RealDriveTypeDetector,
        Duration::from_secs(nas_poll_interval_secs.max(0) as u64),
        Arc::new(crate::events::TauriCatalogNotifier::new(app.clone())),
    );
}

/// Starts watching every folder in `folders` via `watcher`. No filtering by
/// drive type happens here -- that's the caller's job (see
/// `RealtimeWatchManager::reconfigure`) -- every folder passed in gets a
/// realtime subscription. A folder that fails to start watching is logged
/// and skipped; it does not prevent the others from being watched.
///
/// The returned `UnreadableRetry` is shared by all the folders' callbacks; its
/// worker stops once the caller and every callback have dropped it.
pub fn start_watching<N: CatalogNotifier + 'static>(
    db: Db,
    thumbnails_root: PathBuf,
    folders: &[String],
    watcher: &impl FileWatcher,
    notifier: Arc<N>,
    follow_up: Arc<FollowUp>,
) -> (Vec<Box<dyn WatchHandle>>, Arc<UnreadableRetry>) {
    let retry = {
        let (db, thumbnails_root) = (db.clone(), thumbnails_root.clone());
        let (notifier, follow_up) = (Arc::clone(&notifier), Arc::clone(&follow_up));
        Arc::new(UnreadableRetry::new(move |path| {
            process_changed_file(&db, &thumbnails_root, path, notifier.as_ref(), &follow_up)
        }))
    };
    let handles = folders
        .iter()
        .filter_map(|folder| {
            let db = db.clone();
            let thumbnails_root = thumbnails_root.clone();
            let notifier = Arc::clone(&notifier);
            let follow_up = Arc::clone(&follow_up);
            let retry = Arc::clone(&retry);
            let on_event: Box<dyn Fn(WatchEvent) + Send + Sync> = Box::new(move |event| {
                handle_watch_event(
                    &db,
                    &thumbnails_root,
                    event,
                    notifier.as_ref(),
                    &follow_up,
                    &retry,
                )
            });
            match watcher.watch(Path::new(folder), on_event) {
                Ok(handle) => Some(handle),
                Err(e) => {
                    log::error!("failed to start watching {folder}: {e}");
                    None
                }
            }
        })
        .collect();
    (handles, retry)
}

/// The per-file step shared by a live `Created`/`Modified` event and the
/// deferred re-evaluation of a file that was locked earlier, so both have the
/// same side effects (`notify_changed`, `follow_up.trigger()`).
fn process_changed_file(
    db: &Db,
    thumbnails_root: &Path,
    path: &Path,
    notifier: &dyn CatalogNotifier,
    follow_up: &FollowUp,
) -> RetryOutcome {
    // Known, accepted limitation: a large file still being written
    // (a copy in progress) can fire several Created/Modified events
    // before it's complete. Each one that reaches here computes
    // quick_hash (head 1MB + tail 1MB + file_size) against whatever
    // bytes exist *at that instant* -- necessarily different from
    // the final value while the tail is still being written. This
    // self-corrects on the *next* event once the copy finishes: no
    // debounce is implemented, `process_detected_file` is
    // idempotent and simply re-classifies/rehashes on each call.
    // But if the final Created/Modified event for a given copy is
    // ever missed (e.g. the app closes mid-copy, or a burst of
    // events gets dropped upstream), a stale/incomplete quick_hash
    // can persist in the DB -- this then undermines path-follow
    // (which matches moved files by quick_hash+file_size). Same
    // category of known limitation as the mtime-preserving-copy gap
    // in `scan::reconcile_known_path`.
    //
    // A copy that holds the destination exclusively (Explorer does) makes the
    // file unreadable instead, so it surfaces as `SkippedUnreadable`; the
    // caller retries those until the lock is released.
    let metadata = match std::fs::metadata(long_path::to_long_path(path)) {
        Ok(m) => m,
        Err(_) => return RetryOutcome::Gone, // vanished already; a later event or the next scan will catch it
    };
    let file_size = metadata.len();
    let mtime = match mtime_from_metadata(&metadata) {
        Ok(secs) => secs,
        Err(e) => {
            log::warn!("skipping watch event for {}: {e}", path.display());
            return RetryOutcome::Done;
        }
    };
    match scan::process_detected_file(db, thumbnails_root, path, file_size, mtime) {
        Ok(
            scan::ProcessOutcome::Registered
            | scan::ProcessOutcome::Reconciled
            | scan::ProcessOutcome::PathFollowed { .. },
        ) => {
            notifier.notify_changed();
            follow_up.trigger();
            RetryOutcome::Done
        }
        // Locked by a writer (e.g. a copy in progress): the caller retries
        // later, as no further event is guaranteed once the lock is released.
        Ok(scan::ProcessOutcome::SkippedUnreadable) => RetryOutcome::StillUnreadable,
        // Unchanged/SkippedInvalidName/BlockedByCollision (this branch never
        // actually writes -- see scan::mod's register_new_path doc comment)
        // -- nothing changed in the catalog, so no emit.
        Ok(_) => RetryOutcome::Done,
        Err(e) => {
            log::error!("failed to process watch event for {}: {e}", path.display());
            RetryOutcome::Done
        }
    }
}

/// Routes one normalized watch event to the shared per-file pipeline
/// (`Created`/`Modified`) or to `reconcile::decide_removal_outcome`
/// (`Removed`). Called directly from `notify`'s background
/// callback thread, once per event; there is no batch/loop here for a
/// single bad event to abort, so failures are logged and simply don't
/// propagate any further (the same catch-and-continue policy as a scan,
/// applied per-event rather than per-scan).
fn handle_watch_event(
    db: &Db,
    thumbnails_root: &Path,
    event: WatchEvent,
    notifier: &dyn CatalogNotifier,
    follow_up: &FollowUp,
    retry: &UnreadableRetry,
) {
    let file_name = match event.path.file_name() {
        Some(n) => n.to_string_lossy().to_string(),
        None => return,
    };
    // Extension filter (the caller's responsibility, per
    // process_detected_file's contract). Cheaply reject before calling
    // std::fs::metadata -- notify also fires for non-video file events.
    if !scan_pipeline::is_video_file(&file_name) {
        return;
    }

    match event.kind {
        WatchEventKind::Created | WatchEventKind::Modified => {
            if process_changed_file(db, thumbnails_root, &event.path, notifier, follow_up)
                == RetryOutcome::StillUnreadable
            {
                retry.defer(&event.path);
            }
        }
        WatchEventKind::Removed => {
            retry.remove(&event.path);
            // No broken-enumeration guard needed here (unlike
            // reconcile_missing_videos'/run_nas_diff_scan's use of
            // decide_missing_video_ids): a `notify` Removed event is a
            // single OS-confirmed fact about one path, not derived from a
            // WalkDir-style listing that could have partially failed.
            let file_path = event.path.to_string_lossy().to_string();
            let known = {
                let conn = match db.read_pool.get() {
                    Ok(conn) => conn,
                    Err(e) => {
                        log::error!("failed to look up {}: {e}", event.path.display());
                        return;
                    }
                };
                match queries::find_video_by_path(&conn, &file_path) {
                    Ok(known) => known,
                    Err(e) => {
                        log::error!("failed to look up {}: {e}", event.path.display());
                        return;
                    }
                }
            };
            let Some(known) = known else {
                log::info!(
                    "detected removal (untracked path): {}",
                    event.path.display()
                );
                return;
            };
            if let Some(new_status) = reconcile::decide_removal_outcome(&known.status) {
                let conn = db.writer.lock().unwrap();
                if let Err(e) = queries::update_video_status(&conn, &known.id, new_status) {
                    log::error!("failed to mark {} offline: {e}", event.path.display());
                    return;
                }
                drop(conn);
                notifier.notify_changed();
                log::info!("video {} went offline: {}", known.id, event.path.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::init_temp_db;
    use crate::events::FakeCatalogNotifier;
    use gb_core::ports::drive_type::DriveTypeError;
    use gb_core::ports::watcher::WatchEventKind;
    use gb_core::testing::fake_drive_type::FakeDriveTypeDetector;
    use gb_core::testing::fake_watcher::FakeFileWatcher;
    use std::fs;

    /// A throwaway `thumbnails/` root for tests that don't exercise the
    /// thumbnail-moving side of a reactivation -- just needs to be a valid,
    /// empty directory.
    fn temp_thumbs_root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn handle_watch_event_registers_a_created_video_file() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let scan_dir = tempfile::tempdir().unwrap();
        let video_path = scan_dir.path().join("movie.mp4");
        fs::write(&video_path, b"bytes").unwrap();

        handle_watch_event(
            &db,
            thumbs_root.path(),
            WatchEvent {
                kind: WatchEventKind::Created,
                path: video_path.clone(),
            },
            &FakeCatalogNotifier::default(),
            &FollowUp::noop(),
            &UnreadableRetry::noop(),
        );

        let conn = db.writer.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn handle_watch_event_ignores_a_non_video_extension() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let scan_dir = tempfile::tempdir().unwrap();
        let text_path = scan_dir.path().join("notes.txt");
        fs::write(&text_path, b"not a video").unwrap();

        handle_watch_event(
            &db,
            thumbs_root.path(),
            WatchEvent {
                kind: WatchEventKind::Created,
                path: text_path,
            },
            &FakeCatalogNotifier::default(),
            &FollowUp::noop(),
            &UnreadableRetry::noop(),
        );

        let conn = db.writer.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    /// Verifies `Removed` transitions a known online video to offline.
    #[test]
    fn handle_watch_event_removed_transitions_a_known_online_video_to_offline() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let scan_dir = tempfile::tempdir().unwrap();
        let video_path = scan_dir.path().join("movie.mp4");
        fs::write(&video_path, b"bytes").unwrap();
        handle_watch_event(
            &db,
            thumbs_root.path(),
            WatchEvent {
                kind: WatchEventKind::Created,
                path: video_path.clone(),
            },
            &FakeCatalogNotifier::default(),
            &FollowUp::noop(),
            &UnreadableRetry::noop(),
        );

        handle_watch_event(
            &db,
            thumbs_root.path(),
            WatchEvent {
                kind: WatchEventKind::Removed,
                path: video_path,
            },
            &FakeCatalogNotifier::default(),
            &FollowUp::noop(),
            &UnreadableRetry::noop(),
        );

        let conn = db.writer.lock().unwrap();
        let (status, count): (String, i64) = (
            conn.query_row("SELECT status FROM videos", [], |r| r.get(0))
                .unwrap(),
            conn.query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
                .unwrap(),
        );
        assert_eq!(status, "offline");
        assert_eq!(count, 1, "the row must be kept, not deleted");
    }

    #[test]
    fn handle_watch_event_removed_ignores_a_path_with_no_known_row() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let scan_dir = tempfile::tempdir().unwrap();
        // Never registered (e.g. a directory removal, or a video that was
        // never scanned) -- must be a harmless no-op, not an error.
        handle_watch_event(
            &db,
            thumbs_root.path(),
            WatchEvent {
                kind: WatchEventKind::Removed,
                path: scan_dir.path().join("never_registered.mp4"),
            },
            &FakeCatalogNotifier::default(),
            &FollowUp::noop(),
            &UnreadableRetry::noop(),
        );

        let conn = db.writer.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    // --- Emit-triggering conditions (via FakeCatalogNotifier, no
    // AppHandle/mock_app() needed -- see events.rs's doc comment for why) ---

    #[test]
    fn handle_watch_event_notifies_on_a_created_registration() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let scan_dir = tempfile::tempdir().unwrap();
        let video_path = scan_dir.path().join("movie.mp4");
        fs::write(&video_path, b"bytes").unwrap();
        let notifier = FakeCatalogNotifier::default();

        handle_watch_event(
            &db,
            thumbs_root.path(),
            WatchEvent {
                kind: WatchEventKind::Created,
                path: video_path,
            },
            &notifier,
            &FollowUp::noop(),
            &UnreadableRetry::noop(),
        );

        assert_eq!(notifier.calls(), 1, "a fresh registration must notify");
    }

    #[test]
    fn handle_watch_event_does_not_notify_on_an_unchanged_rescan() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let scan_dir = tempfile::tempdir().unwrap();
        let video_path = scan_dir.path().join("movie.mp4");
        fs::write(&video_path, b"stable bytes").unwrap();
        let event = || WatchEvent {
            kind: WatchEventKind::Created,
            path: video_path.clone(),
        };

        // First event registers (and notifies) the row.
        handle_watch_event(
            &db,
            thumbs_root.path(),
            event(),
            &FakeCatalogNotifier::default(),
            &FollowUp::noop(),
            &UnreadableRetry::noop(),
        );

        // A second event for the exact same, unchanged file must classify
        // as Unchanged (no DB write) and therefore must not notify.
        let notifier = FakeCatalogNotifier::default();
        handle_watch_event(
            &db,
            thumbs_root.path(),
            event(),
            &notifier,
            &FollowUp::noop(),
            &UnreadableRetry::noop(),
        );

        assert_eq!(
            notifier.calls(),
            0,
            "an Unchanged outcome (no DB write) must not notify"
        );
    }

    #[test]
    fn handle_watch_event_removed_does_not_notify_for_an_untracked_path() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let scan_dir = tempfile::tempdir().unwrap();
        let notifier = FakeCatalogNotifier::default();

        handle_watch_event(
            &db,
            thumbs_root.path(),
            WatchEvent {
                kind: WatchEventKind::Removed,
                path: scan_dir.path().join("never_registered.mp4"),
            },
            &notifier,
            &FollowUp::noop(),
            &UnreadableRetry::noop(),
        );

        assert_eq!(
            notifier.calls(),
            0,
            "an untracked path's Removed event must not notify (nothing changed)"
        );
    }

    #[test]
    fn handle_watch_event_removed_notifies_when_a_known_online_video_goes_offline() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let scan_dir = tempfile::tempdir().unwrap();
        let video_path = scan_dir.path().join("movie.mp4");
        fs::write(&video_path, b"bytes").unwrap();
        handle_watch_event(
            &db,
            thumbs_root.path(),
            WatchEvent {
                kind: WatchEventKind::Created,
                path: video_path.clone(),
            },
            &FakeCatalogNotifier::default(),
            &FollowUp::noop(),
            &UnreadableRetry::noop(),
        );

        let notifier = FakeCatalogNotifier::default();
        handle_watch_event(
            &db,
            thumbs_root.path(),
            WatchEvent {
                kind: WatchEventKind::Removed,
                path: video_path,
            },
            &notifier,
            &FollowUp::noop(),
            &UnreadableRetry::noop(),
        );

        assert_eq!(
            notifier.calls(),
            1,
            "an actual offline transition must notify"
        );
    }

    /// Exercises the full realtime path (via `FakeFileWatcher`, not a
    /// direct `handle_watch_event` call) for Created -> Removed -> Created
    /// at the *same* path: online -> offline ->
    /// online again (via `reconcile_known_path`'s reactivation,
    /// requiring no new code). `id`/`quick_hash` must stay identical
    /// throughout -- the row is reused, never replaced.
    #[test]
    fn created_removed_created_at_the_same_path_cycles_through_fake_watcher_preserving_identity() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let scan_dir = tempfile::tempdir().unwrap();
        let video_path = scan_dir.path().join("movie.mp4");
        fs::write(&video_path, b"bytes").unwrap();

        let fake = FakeFileWatcher {
            watch_result: Ok(()),
            ..Default::default()
        };
        let folders = vec![scan_dir.path().to_string_lossy().to_string()];
        start_watching(
            db.clone(),
            thumbs_root.path().to_path_buf(),
            &folders,
            &fake,
            Arc::new(FakeCatalogNotifier::default()),
            Arc::new(FollowUp::noop()),
        );

        let event = |kind| WatchEvent {
            kind,
            path: video_path.clone(),
        };

        fake.emit(scan_dir.path(), event(WatchEventKind::Created));
        let (id1, hash1, status1) = query_video(&db);
        assert_eq!(status1, "online");

        fake.emit(scan_dir.path(), event(WatchEventKind::Removed));
        let (id2, hash2, status2) = query_video(&db);
        assert_eq!(status2, "offline");
        assert_eq!(id2, id1);
        assert_eq!(hash2, hash1);

        fake.emit(scan_dir.path(), event(WatchEventKind::Created));
        let (id3, hash3, status3) = query_video(&db);
        assert_eq!(status3, "online");
        assert_eq!(id3, id1);
        assert_eq!(hash3, hash1);

        let conn = db.writer.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 1,
            "the cycle must reuse one row, never insert a second"
        );
    }

    fn query_video(db: &Db) -> (String, String, String) {
        let conn = db.writer.lock().unwrap();
        conn.query_row("SELECT id, quick_hash, status FROM videos", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap()
    }

    #[test]
    fn start_watching_calls_watch_once_per_folder_and_routes_events_to_handle_watch_event() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let scan_dir = tempfile::tempdir().unwrap();
        let video_path = scan_dir.path().join("movie.mp4");
        fs::write(&video_path, b"bytes").unwrap();

        let fake = FakeFileWatcher {
            watch_result: Ok(()),
            ..Default::default()
        };
        let folders = vec![scan_dir.path().to_string_lossy().to_string()];
        let (handles, _retry) = start_watching(
            db.clone(),
            thumbs_root.path().to_path_buf(),
            &folders,
            &fake,
            Arc::new(FakeCatalogNotifier::default()),
            Arc::new(FollowUp::noop()),
        );

        assert_eq!(handles.len(), 1);
        assert_eq!(fake.watched_folders(), vec![scan_dir.path().to_path_buf()]);

        fake.emit(
            scan_dir.path(),
            WatchEvent {
                kind: WatchEventKind::Created,
                path: video_path,
            },
        );

        let conn = db.writer.lock().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn reconfigure_routes_local_and_removable_to_watching_and_network_to_polling() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let local_dir = tempfile::tempdir().unwrap();
        let removable_dir = tempfile::tempdir().unwrap();
        let nas_dir = tempfile::tempdir().unwrap();

        let mut drive_type = FakeDriveTypeDetector::default();
        drive_type
            .results
            .insert(local_dir.path().to_path_buf(), Ok(DriveKind::Local));
        drive_type
            .results
            .insert(removable_dir.path().to_path_buf(), Ok(DriveKind::Removable));
        drive_type
            .results
            .insert(nas_dir.path().to_path_buf(), Ok(DriveKind::Network));

        let watcher = FakeFileWatcher {
            watch_result: Ok(()),
            ..Default::default()
        };
        let manager = RealtimeWatchManager::default();
        let folders = vec![
            local_dir.path().to_string_lossy().to_string(),
            removable_dir.path().to_string_lossy().to_string(),
            nas_dir.path().to_string_lossy().to_string(),
        ];

        manager.reconfigure(
            WatchContext {
                db,
                thumbnails_root: thumbs_root.path().to_path_buf(),
            },
            &folders,
            &watcher,
            &drive_type,
            Duration::from_secs(600),
            Arc::new(FakeCatalogNotifier::default()),
        );

        assert_eq!(
            manager.active_handle_count(),
            2,
            "local + removable should both be watched via notify"
        );
        assert_eq!(
            manager.active_nas_poller_count(),
            1,
            "the network folder should be polled instead"
        );
    }

    #[test]
    fn reconfigure_falls_back_to_polling_when_drive_type_is_unknown_or_undetectable() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let unknown_dir = tempfile::tempdir().unwrap();
        let error_dir = tempfile::tempdir().unwrap();

        let mut drive_type = FakeDriveTypeDetector::default();
        drive_type
            .results
            .insert(unknown_dir.path().to_path_buf(), Ok(DriveKind::Unknown));
        drive_type.results.insert(
            error_dir.path().to_path_buf(),
            Err(DriveTypeError::DetectionFailed {
                path: error_dir.path().to_string_lossy().to_string(),
                message: "boom".to_string(),
            }),
        );

        let watcher = FakeFileWatcher {
            watch_result: Ok(()),
            ..Default::default()
        };
        let manager = RealtimeWatchManager::default();
        let folders = vec![
            unknown_dir.path().to_string_lossy().to_string(),
            error_dir.path().to_string_lossy().to_string(),
        ];

        manager.reconfigure(
            WatchContext {
                db,
                thumbnails_root: thumbs_root.path().to_path_buf(),
            },
            &folders,
            &watcher,
            &drive_type,
            Duration::from_secs(600),
            Arc::new(FakeCatalogNotifier::default()),
        );

        assert_eq!(manager.active_handle_count(), 0);
        assert_eq!(
            manager.active_nas_poller_count(),
            2,
            "Unknown and Err must both fall back to the safer polling path"
        );
    }

    #[test]
    fn reconfigure_replaces_the_previous_set_of_handles_and_pollers() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let local_dir = tempfile::tempdir().unwrap();
        let nas_dir = tempfile::tempdir().unwrap();

        let mut drive_type = FakeDriveTypeDetector::default();
        drive_type
            .results
            .insert(local_dir.path().to_path_buf(), Ok(DriveKind::Local));
        drive_type
            .results
            .insert(nas_dir.path().to_path_buf(), Ok(DriveKind::Network));
        let watcher = FakeFileWatcher {
            watch_result: Ok(()),
            ..Default::default()
        };
        let manager = RealtimeWatchManager::default();

        manager.reconfigure(
            WatchContext {
                db: db.clone(),
                thumbnails_root: thumbs_root.path().to_path_buf(),
            },
            &[
                local_dir.path().to_string_lossy().to_string(),
                nas_dir.path().to_string_lossy().to_string(),
            ],
            &watcher,
            &drive_type,
            Duration::from_secs(600),
            Arc::new(FakeCatalogNotifier::default()),
        );
        assert_eq!(manager.active_handle_count(), 1);
        assert_eq!(manager.active_nas_poller_count(), 1);

        // Reconfiguring with an empty list must replace the manager's
        // tracked set immediately -- NasPollHandle::stop() no
        // longer joins its thread, so this only proves the *bookkeeping*
        // (active_nas_poller_count) was updated synchronously, not that the
        // old background thread has actually exited yet (it may still be
        // finishing its current cycle in the background).
        manager.reconfigure(
            WatchContext {
                db,
                thumbnails_root: thumbs_root.path().to_path_buf(),
            },
            &[],
            &watcher,
            &drive_type,
            Duration::from_secs(600),
            Arc::new(FakeCatalogNotifier::default()),
        );
        assert_eq!(manager.active_handle_count(), 0);
        assert_eq!(manager.active_nas_poller_count(), 0);
    }

    #[test]
    fn concurrent_reconfigure_calls_leave_exactly_one_set_of_handles() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let local_dir = tempfile::tempdir().unwrap();
        let nas_dir = tempfile::tempdir().unwrap();

        let mut drive_type = FakeDriveTypeDetector::default();
        drive_type
            .results
            .insert(local_dir.path().to_path_buf(), Ok(DriveKind::Local));
        drive_type
            .results
            .insert(nas_dir.path().to_path_buf(), Ok(DriveKind::Network));
        let watcher = FakeFileWatcher {
            watch_result: Ok(()),
            ..Default::default()
        };
        let manager = RealtimeWatchManager::default();
        let folders = [
            local_dir.path().to_string_lossy().to_string(),
            nas_dir.path().to_string_lossy().to_string(),
        ];

        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    for _ in 0..5 {
                        manager.reconfigure(
                            WatchContext {
                                db: db.clone(),
                                thumbnails_root: thumbs_root.path().to_path_buf(),
                            },
                            &folders,
                            &watcher,
                            &drive_type,
                            Duration::from_secs(600),
                            Arc::new(FakeCatalogNotifier::default()),
                        );
                    }
                });
            }
        });

        assert_eq!(manager.active_handle_count(), 1);
        assert_eq!(manager.active_nas_poller_count(), 1);
    }

    #[test]
    fn nas_folder_gets_diff_scanned_through_reconfigure() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let nas_dir = tempfile::tempdir().unwrap();
        fs::write(nas_dir.path().join("movie.mp4"), b"bytes").unwrap();

        let mut drive_type = FakeDriveTypeDetector::default();
        drive_type
            .results
            .insert(nas_dir.path().to_path_buf(), Ok(DriveKind::Network));
        let watcher = FakeFileWatcher {
            watch_result: Ok(()),
            ..Default::default()
        };
        let manager = RealtimeWatchManager::default();

        manager.reconfigure(
            WatchContext {
                db: db.clone(),
                thumbnails_root: thumbs_root.path().to_path_buf(),
            },
            &[nas_dir.path().to_string_lossy().to_string()],
            &watcher,
            &drive_type,
            Duration::from_secs(600),
            Arc::new(FakeCatalogNotifier::default()),
        );

        // The initial diff-scan runs on start_nas_polling's own background
        // thread (never blocking the reconfigure caller -- important for
        // .setup()/pick_watch_folders responsiveness), so its effect is
        // only observable asynchronously; poll with a bounded timeout
        // rather than assuming it's already done the instant reconfigure
        // returns.
        let mut count = 0;
        for _ in 0..50 {
            count = db
                .writer
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
                .unwrap();
            if count == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(
            count, 1,
            "the initial diff-scan should register the file within 5s"
        );
    }

    /// A `FollowUp` that never fires during a test, so `has_pending` can be
    /// asserted deterministically.
    fn parked_follow_up() -> FollowUp {
        FollowUp::with_timing(Duration::from_secs(60), Duration::from_secs(60), || {})
    }

    fn created_event(path: &Path) -> WatchEvent {
        WatchEvent {
            kind: WatchEventKind::Created,
            path: path.to_path_buf(),
        }
    }

    #[test]
    fn handle_watch_event_triggers_follow_up_on_a_new_registration() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let scan_dir = tempfile::tempdir().unwrap();
        let video_path = scan_dir.path().join("movie.mp4");
        fs::write(&video_path, b"bytes").unwrap();
        let follow_up = parked_follow_up();

        handle_watch_event(
            &db,
            thumbs_root.path(),
            created_event(&video_path),
            &FakeCatalogNotifier::default(),
            &follow_up,
            &UnreadableRetry::noop(),
        );

        assert!(follow_up.has_pending());
    }

    #[test]
    fn handle_watch_event_does_not_trigger_follow_up_when_unchanged() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let scan_dir = tempfile::tempdir().unwrap();
        let video_path = scan_dir.path().join("movie.mp4");
        fs::write(&video_path, b"bytes").unwrap();
        let notifier = FakeCatalogNotifier::default();
        handle_watch_event(
            &db,
            thumbs_root.path(),
            created_event(&video_path),
            &notifier,
            &FollowUp::noop(),
            &UnreadableRetry::noop(),
        );

        let follow_up = parked_follow_up();
        handle_watch_event(
            &db,
            thumbs_root.path(),
            created_event(&video_path),
            &notifier,
            &follow_up,
            &UnreadableRetry::noop(),
        );

        assert!(!follow_up.has_pending());
    }

    #[test]
    fn handle_watch_event_does_not_trigger_follow_up_on_removal() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let scan_dir = tempfile::tempdir().unwrap();
        let video_path = scan_dir.path().join("movie.mp4");
        fs::write(&video_path, b"bytes").unwrap();
        let notifier = FakeCatalogNotifier::default();
        handle_watch_event(
            &db,
            thumbs_root.path(),
            created_event(&video_path),
            &notifier,
            &FollowUp::noop(),
            &UnreadableRetry::noop(),
        );

        let follow_up = parked_follow_up();
        handle_watch_event(
            &db,
            thumbs_root.path(),
            WatchEvent {
                kind: WatchEventKind::Removed,
                path: video_path,
            },
            &notifier,
            &follow_up,
            &UnreadableRetry::noop(),
        );

        assert!(!follow_up.has_pending());
    }

    #[cfg(windows)]
    fn open_exclusively(path: &Path) -> fs::File {
        use std::os::windows::fs::OpenOptionsExt;
        // Share mode 0 is what an in-progress Explorer copy holds: every other
        // open of the file fails with a sharing violation (os error 32).
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .share_mode(0)
            .open(path)
            .unwrap()
    }

    #[cfg(windows)]
    fn video_count(db: &Db) -> i64 {
        db.writer
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap()
    }

    /// Starts a real `UnreadableRetry` wired like `start_watching` does, with a
    /// short interval.
    #[cfg(windows)]
    fn fast_retry(
        db: &Db,
        thumbs_root: &Path,
        notifier: &Arc<FakeCatalogNotifier>,
    ) -> UnreadableRetry {
        let (db, thumbs_root, notifier) =
            (db.clone(), thumbs_root.to_path_buf(), Arc::clone(notifier));
        UnreadableRetry::with_timing(
            Duration::from_millis(30),
            Duration::from_secs(60),
            move |path| {
                process_changed_file(
                    &db,
                    &thumbs_root,
                    path,
                    notifier.as_ref(),
                    &FollowUp::noop(),
                )
            },
        )
    }

    #[cfg(windows)]
    #[test]
    fn a_locked_file_is_deferred_and_registered_once_the_lock_is_released() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let scan_dir = tempfile::tempdir().unwrap();
        let video_path = scan_dir.path().join("copying.mp4");
        let lock = open_exclusively(&video_path);
        // The lock must really make the file unreadable, or this test would
        // pass without reproducing anything.
        assert_eq!(
            fs::File::open(&video_path).unwrap_err().raw_os_error(),
            Some(32)
        );
        let notifier = Arc::new(FakeCatalogNotifier::default());
        let retry = fast_retry(&db, thumbs_root.path(), &notifier);

        handle_watch_event(
            &db,
            thumbs_root.path(),
            created_event(&video_path),
            notifier.as_ref(),
            &FollowUp::noop(),
            &retry,
        );
        assert_eq!(retry.pending_count(), 1);

        // Several retry rounds pass while still locked: nothing registers.
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(video_count(&db), 0);
        assert_eq!(retry.pending_count(), 1);

        {
            use std::io::Write;
            let mut lock = lock;
            lock.write_all(b"finished copy").unwrap();
        }

        let mut registered = false;
        for _ in 0..100 {
            if video_count(&db) == 1 && retry.pending_count() == 0 {
                registered = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        assert!(registered, "the retry must register the file after unlock");
        assert_eq!(notifier.calls(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn a_removed_event_drops_a_deferred_file() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let scan_dir = tempfile::tempdir().unwrap();
        let video_path = scan_dir.path().join("copying.mp4");
        let _lock = open_exclusively(&video_path);
        let notifier = Arc::new(FakeCatalogNotifier::default());
        // Parked worker: the pending count is deterministic.
        let retry =
            UnreadableRetry::with_timing(Duration::from_secs(60), Duration::from_secs(60), |_| {
                RetryOutcome::StillUnreadable
            });
        handle_watch_event(
            &db,
            thumbs_root.path(),
            created_event(&video_path),
            notifier.as_ref(),
            &FollowUp::noop(),
            &retry,
        );
        assert_eq!(retry.pending_count(), 1);

        handle_watch_event(
            &db,
            thumbs_root.path(),
            WatchEvent {
                kind: WatchEventKind::Removed,
                path: video_path,
            },
            notifier.as_ref(),
            &FollowUp::noop(),
            &retry,
        );

        assert_eq!(retry.pending_count(), 0);
    }

    #[test]
    fn a_file_that_vanished_is_not_deferred() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let scan_dir = tempfile::tempdir().unwrap();
        let retry =
            UnreadableRetry::with_timing(Duration::from_secs(60), Duration::from_secs(60), |_| {
                RetryOutcome::StillUnreadable
            });

        handle_watch_event(
            &db,
            thumbs_root.path(),
            created_event(&scan_dir.path().join("gone.mp4")),
            &FakeCatalogNotifier::default(),
            &FollowUp::noop(),
            &retry,
        );

        assert_eq!(retry.pending_count(), 0);
    }

    #[test]
    fn local_folder_gets_a_startup_diff_scan_that_is_stopped_by_the_next_reconfigure() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = temp_thumbs_root();
        let local_dir = tempfile::tempdir().unwrap();
        fs::write(local_dir.path().join("movie.mp4"), b"bytes").unwrap();

        let mut drive_type = FakeDriveTypeDetector::default();
        drive_type
            .results
            .insert(local_dir.path().to_path_buf(), Ok(DriveKind::Local));
        let watcher = FakeFileWatcher {
            watch_result: Ok(()),
            ..Default::default()
        };
        let manager = RealtimeWatchManager::default();
        let ctx = || WatchContext {
            db: db.clone(),
            thumbnails_root: thumbs_root.path().to_path_buf(),
        };

        manager.reconfigure(
            ctx(),
            &[local_dir.path().to_string_lossy().to_string()],
            &watcher,
            &drive_type,
            Duration::from_secs(600),
            Arc::new(FakeCatalogNotifier::default()),
        );

        assert_eq!(manager.active_handle_count(), 1);
        assert_eq!(manager.active_nas_poller_count(), 0);
        assert_eq!(manager.startup_scans.lock().unwrap().len(), 1);
        let stop_flag = manager.startup_scans.lock().unwrap()[0].stop_flag();
        assert!(!stop_flag.load(std::sync::atomic::Ordering::SeqCst));

        let mut count: i64 = 0;
        for _ in 0..50 {
            count = db
                .writer
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
                .unwrap();
            if count == 1 {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(
            count, 1,
            "the startup scan should register the file within 5s"
        );

        manager.reconfigure(
            ctx(),
            &[],
            &watcher,
            &drive_type,
            Duration::from_secs(600),
            Arc::new(FakeCatalogNotifier::default()),
        );
        assert!(stop_flag.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(manager.startup_scans.lock().unwrap().len(), 0);
    }
}
