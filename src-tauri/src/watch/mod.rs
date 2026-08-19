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

pub mod nas_poll;

use std::path::Path;
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
use nas_poll::NasPollHandle;

/// Holds the live watch subscriptions/pollers for as long as the app runs.
/// Managed via `app.manage()`. No explicit shutdown wiring at process exit:
/// dropping this drops every `WatchHandle` (whose `Drop`, through the
/// `notify` watcher it owns, unregisters the OS-level watch);
/// `NasPollHandle`s left un-stopped at process exit are
/// simply killed along with their thread by the OS, same reasoning.
/// Reconfiguration mid-run (folder list changes) goes through `reconfigure`
/// instead, which explicitly stops the old set before starting the new one.
#[derive(Default)]
pub struct RealtimeWatchManager {
    handles: Mutex<Vec<Box<dyn WatchHandle>>>,
    nas_pollers: Mutex<Vec<NasPollHandle>>,
}

impl RealtimeWatchManager {
    /// Stops whatever this manager currently holds, classifies each folder
    /// in `folders` by drive type, and starts a fresh set: local/removable
    /// folders get realtime `notify` watching (via `start_watching`),
    /// network folders get `nas_poll::start_nas_polling`. A
    /// folder whose drive type can't be determined (`Unknown` or an `Err`
    /// from `drive_type`) falls back to polling -- the safer default, since
    /// polling still eventually detects changes, while assuming "local"
    /// for an actually-remote folder would just never fire at all.
    pub fn reconfigure<N: CatalogNotifier + 'static>(
        &self,
        db: Db,
        folders: &[String],
        watcher: &impl FileWatcher,
        drive_type: &impl DriveTypeDetector,
        nas_poll_interval: Duration,
        notifier: Arc<N>,
    ) {
        for mut handle in self.handles.lock().unwrap().drain(..) {
            handle.stop();
        }
        for poller in self.nas_pollers.lock().unwrap().drain(..) {
            poller.stop();
        }

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

        let new_handles =
            start_watching(db.clone(), &local_folders, watcher, Arc::clone(&notifier));
        let new_pollers: Vec<NasPollHandle> = nas_folders
            .into_iter()
            .map(|folder| {
                nas_poll::start_nas_polling(
                    db.clone(),
                    folder,
                    nas_poll_interval,
                    Arc::clone(&notifier),
                )
            })
            .collect();

        *self.handles.lock().unwrap() = new_handles;
        *self.nas_pollers.lock().unwrap() = new_pollers;
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
pub fn reconfigure_real_watch_manager(
    app: &tauri::AppHandle,
    db: &Db,
    watch_manager: &RealtimeWatchManager,
    folders: &[String],
    nas_poll_interval_secs: i64,
) {
    watch_manager.reconfigure(
        db.clone(),
        folders,
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
pub fn start_watching<N: CatalogNotifier + 'static>(
    db: Db,
    folders: &[String],
    watcher: &impl FileWatcher,
    notifier: Arc<N>,
) -> Vec<Box<dyn WatchHandle>> {
    folders
        .iter()
        .filter_map(|folder| {
            let db = db.clone();
            let notifier = Arc::clone(&notifier);
            let on_event: Box<dyn Fn(WatchEvent) + Send + Sync> =
                Box::new(move |event| handle_watch_event(&db, event, notifier.as_ref()));
            match watcher.watch(Path::new(folder), on_event) {
                Ok(handle) => Some(handle),
                Err(e) => {
                    log::error!("failed to start watching {folder}: {e}");
                    None
                }
            }
        })
        .collect()
}

/// Routes one normalized watch event to the shared per-file pipeline
/// (`Created`/`Modified`) or to `reconcile::decide_removal_outcome`
/// (`Removed`). Called directly from `notify`'s background
/// callback thread, once per event; there is no batch/loop here for a
/// single bad event to abort, so failures are logged and simply don't
/// propagate any further (the same catch-and-continue policy as a scan,
/// applied per-event rather than per-scan).
fn handle_watch_event(db: &Db, event: WatchEvent, notifier: &dyn CatalogNotifier) {
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
            let metadata = match std::fs::metadata(long_path::to_long_path(&event.path)) {
                Ok(m) => m,
                Err(_) => return, // vanished already; a later event or the next scan will catch it
            };
            let file_size = metadata.len();
            let mtime = match mtime_from_metadata(&metadata) {
                Ok(secs) => secs,
                Err(e) => {
                    log::warn!("skipping watch event for {}: {e}", event.path.display());
                    return;
                }
            };
            match scan::process_detected_file(db, &event.path, file_size, mtime) {
                Ok(
                    scan::ProcessOutcome::Registered
                    | scan::ProcessOutcome::Reconciled
                    | scan::ProcessOutcome::PathFollowed { .. },
                ) => notifier.notify_changed(),
                // Unchanged/SkippedInvalidName/SkippedUnreadable/
                // BlockedByCollision (this branch never actually
                // writes -- see scan::mod's register_new_path doc comment)
                // -- nothing changed in the catalog, so no emit.
                Ok(_) => {}
                Err(e) => {
                    log::error!(
                        "failed to process watch event for {}: {e}",
                        event.path.display()
                    );
                }
            }
        }
        WatchEventKind::Removed => {
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

    #[test]
    fn handle_watch_event_registers_a_created_video_file() {
        let (_db_dir, db) = init_temp_db();
        let scan_dir = tempfile::tempdir().unwrap();
        let video_path = scan_dir.path().join("movie.mp4");
        fs::write(&video_path, b"bytes").unwrap();

        handle_watch_event(
            &db,
            WatchEvent {
                kind: WatchEventKind::Created,
                path: video_path.clone(),
            },
            &FakeCatalogNotifier::default(),
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
        let scan_dir = tempfile::tempdir().unwrap();
        let text_path = scan_dir.path().join("notes.txt");
        fs::write(&text_path, b"not a video").unwrap();

        handle_watch_event(
            &db,
            WatchEvent {
                kind: WatchEventKind::Created,
                path: text_path,
            },
            &FakeCatalogNotifier::default(),
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
        let scan_dir = tempfile::tempdir().unwrap();
        let video_path = scan_dir.path().join("movie.mp4");
        fs::write(&video_path, b"bytes").unwrap();
        handle_watch_event(
            &db,
            WatchEvent {
                kind: WatchEventKind::Created,
                path: video_path.clone(),
            },
            &FakeCatalogNotifier::default(),
        );

        handle_watch_event(
            &db,
            WatchEvent {
                kind: WatchEventKind::Removed,
                path: video_path,
            },
            &FakeCatalogNotifier::default(),
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
        let scan_dir = tempfile::tempdir().unwrap();
        // Never registered (e.g. a directory removal, or a video that was
        // never scanned) -- must be a harmless no-op, not an error.
        handle_watch_event(
            &db,
            WatchEvent {
                kind: WatchEventKind::Removed,
                path: scan_dir.path().join("never_registered.mp4"),
            },
            &FakeCatalogNotifier::default(),
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
        let scan_dir = tempfile::tempdir().unwrap();
        let video_path = scan_dir.path().join("movie.mp4");
        fs::write(&video_path, b"bytes").unwrap();
        let notifier = FakeCatalogNotifier::default();

        handle_watch_event(
            &db,
            WatchEvent {
                kind: WatchEventKind::Created,
                path: video_path,
            },
            &notifier,
        );

        assert_eq!(notifier.calls(), 1, "a fresh registration must notify");
    }

    #[test]
    fn handle_watch_event_does_not_notify_on_an_unchanged_rescan() {
        let (_db_dir, db) = init_temp_db();
        let scan_dir = tempfile::tempdir().unwrap();
        let video_path = scan_dir.path().join("movie.mp4");
        fs::write(&video_path, b"stable bytes").unwrap();
        let event = || WatchEvent {
            kind: WatchEventKind::Created,
            path: video_path.clone(),
        };

        // First event registers (and notifies) the row.
        handle_watch_event(&db, event(), &FakeCatalogNotifier::default());

        // A second event for the exact same, unchanged file must classify
        // as Unchanged (no DB write) and therefore must not notify.
        let notifier = FakeCatalogNotifier::default();
        handle_watch_event(&db, event(), &notifier);

        assert_eq!(
            notifier.calls(),
            0,
            "an Unchanged outcome (no DB write) must not notify"
        );
    }

    #[test]
    fn handle_watch_event_removed_does_not_notify_for_an_untracked_path() {
        let (_db_dir, db) = init_temp_db();
        let scan_dir = tempfile::tempdir().unwrap();
        let notifier = FakeCatalogNotifier::default();

        handle_watch_event(
            &db,
            WatchEvent {
                kind: WatchEventKind::Removed,
                path: scan_dir.path().join("never_registered.mp4"),
            },
            &notifier,
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
        let scan_dir = tempfile::tempdir().unwrap();
        let video_path = scan_dir.path().join("movie.mp4");
        fs::write(&video_path, b"bytes").unwrap();
        handle_watch_event(
            &db,
            WatchEvent {
                kind: WatchEventKind::Created,
                path: video_path.clone(),
            },
            &FakeCatalogNotifier::default(),
        );

        let notifier = FakeCatalogNotifier::default();
        handle_watch_event(
            &db,
            WatchEvent {
                kind: WatchEventKind::Removed,
                path: video_path,
            },
            &notifier,
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
            &folders,
            &fake,
            Arc::new(FakeCatalogNotifier::default()),
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
        let scan_dir = tempfile::tempdir().unwrap();
        let video_path = scan_dir.path().join("movie.mp4");
        fs::write(&video_path, b"bytes").unwrap();

        let fake = FakeFileWatcher {
            watch_result: Ok(()),
            ..Default::default()
        };
        let folders = vec![scan_dir.path().to_string_lossy().to_string()];
        let handles = start_watching(
            db.clone(),
            &folders,
            &fake,
            Arc::new(FakeCatalogNotifier::default()),
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
            db,
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
            db,
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
            db.clone(),
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
            db,
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
    fn nas_folder_gets_diff_scanned_through_reconfigure() {
        let (_db_dir, db) = init_temp_db();
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
            db.clone(),
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
}
