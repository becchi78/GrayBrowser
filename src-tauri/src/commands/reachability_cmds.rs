use std::thread::{self, JoinHandle};
use std::time::Duration;

use tauri::State;

use crate::db::{queries, Db};
use crate::reachability::{ReachabilityRegistry, DEFAULT_OPEN_RECHECK_MIN_AGE};

/// The watch folders known to be unreachable right now. The frontend reads
/// this once on mount (events emitted before it subscribed are lost) and then
/// follows `reachability:changed`.
///
/// Reads memory only -- it never probes -- so it cannot block on a dead share.
pub fn current_unreachable_folders(registry: &ReachabilityRegistry) -> Vec<String> {
    registry.unreachable_folders()
}

#[tauri::command]
pub fn get_unreachable_folders(registry: State<ReachabilityRegistry>) -> Vec<String> {
    current_unreachable_folders(&registry)
}

/// Starts a background probe of `folder`, and returns at once without waiting
/// for it: a stalled share blocks a probe for ~20s, which must not hold up the
/// command or the UI. The answer reaches the screens through the registry's
/// `reachability:changed` event.
///
/// Only a folder that is exactly one of `watch_folders` is probed, so the
/// frontend cannot make the app touch an arbitrary path. Exact comparison is
/// right (no case or separator folding) because the registry keys its state by
/// the very strings in the watch-folder list, which are also what the sidebar
/// sends back; a differently spelled path would only create a second,
/// unrelated entry. Returns the thread so a test can wait for it; production
/// drops it.
pub fn start_folder_reachability_check(
    registry: &ReachabilityRegistry,
    watch_folders: &[String],
    folder: &str,
    min_age: Duration,
) -> Option<JoinHandle<()>> {
    if !watch_folders.iter().any(|f| f == folder) {
        return None;
    }
    let registry = registry.clone();
    let folder = folder.to_string();
    thread::Builder::new()
        .name("reachability-open-check".into())
        .spawn(move || {
            registry.refresh_if_stale(&folder, min_age);
        })
        .inspect_err(|e| log::warn!("could not start the folder reachability check: {e}"))
        .ok()
}

/// Re-checks one watch folder when the user opens it, so a stale "offline"
/// (or a missing one) is corrected without waiting for the next poll. Returns
/// immediately; unregistered folders are ignored.
#[tauri::command]
pub fn check_folder_reachability(
    db: State<Db>,
    registry: State<ReachabilityRegistry>,
    folder: String,
) -> Result<(), String> {
    let watch_folders = {
        let conn = db.read_pool.get().map_err(|e| e.to_string())?;
        queries::get_watch_folders(&conn).map_err(|e| e.to_string())?
    };
    start_folder_reachability_check(
        &registry,
        &watch_folders,
        &folder,
        DEFAULT_OPEN_RECHECK_MIN_AGE,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::NoopReachabilityNotifier;
    use gb_core::ports::folder_probe::FolderProbe;
    use gb_core::testing::fake_folder_probe::FakeFolderProbe;
    use std::sync::{Arc, Condvar, Mutex};

    const NAS: &str = "W:/video";
    const OTHER: &str = "D:/other";
    const HOUR: Duration = Duration::from_secs(3600);

    fn registry_over(probe: Arc<dyn FolderProbe>) -> ReachabilityRegistry {
        ReachabilityRegistry::new(probe, Arc::new(NoopReachabilityNotifier))
    }

    #[test]
    fn a_folder_that_is_not_a_watch_folder_is_not_probed() {
        let probe = Arc::new(FakeFolderProbe::default());
        let registry = registry_over(probe.clone());
        let watch_folders = vec![NAS.to_string()];

        for folder in [OTHER, "W:/VIDEO", "W:/video/", "W:\\video", ""] {
            assert!(
                start_folder_reachability_check(&registry, &watch_folders, folder, HOUR).is_none()
            );
        }

        assert!(probe.calls().is_empty());
    }

    #[test]
    fn a_watch_folder_is_probed() {
        let probe = Arc::new(FakeFolderProbe::with_unreachable(&[NAS]));
        let registry = registry_over(probe.clone());
        let watch_folders = vec![OTHER.to_string(), NAS.to_string()];

        let handle = start_folder_reachability_check(&registry, &watch_folders, NAS, HOUR)
            .expect("a registered folder starts a check");
        handle.join().unwrap();

        assert_eq!(probe.calls(), vec![NAS.to_string()]);
        assert_eq!(
            current_unreachable_folders(&registry),
            vec![NAS.to_string()]
        );
    }

    /// Blocks the probe until released, so the test controls when it ends.
    struct BlockedProbe {
        released: Mutex<bool>,
        cond: Condvar,
        entered: Mutex<bool>,
        entered_cond: Condvar,
    }

    impl FolderProbe for BlockedProbe {
        fn is_reachable(&self, _folder: &str) -> bool {
            *self.entered.lock().unwrap() = true;
            self.entered_cond.notify_all();
            let released = self.released.lock().unwrap();
            drop(self.cond.wait_while(released, |r| !*r).unwrap());
            true
        }
    }

    #[test]
    fn the_call_returns_before_the_probe_finishes() {
        let probe = Arc::new(BlockedProbe {
            released: Mutex::new(false),
            cond: Condvar::new(),
            entered: Mutex::new(false),
            entered_cond: Condvar::new(),
        });
        let registry = registry_over(probe.clone());
        let watch_folders = vec![NAS.to_string()];

        // If the call waited for the probe, it would never return.
        let handle = start_folder_reachability_check(&registry, &watch_folders, NAS, HOUR)
            .expect("a registered folder starts a check");

        // The probe really is running while the call is already over.
        drop(
            probe
                .entered_cond
                .wait_while(probe.entered.lock().unwrap(), |e| !*e)
                .unwrap(),
        );
        assert!(!handle.is_finished());

        *probe.released.lock().unwrap() = true;
        probe.cond.notify_all();
        handle.join().unwrap();
    }

    #[test]
    fn reports_only_folders_found_unreachable_and_never_probes() {
        let probe = Arc::new(FakeFolderProbe::with_unreachable(&["W:/video"]));
        let registry = ReachabilityRegistry::new(probe.clone(), Arc::new(NoopReachabilityNotifier));
        assert!(current_unreachable_folders(&registry).is_empty());
        assert!(probe.calls().is_empty());

        registry.is_reachable("W:/video");
        registry.is_reachable("C:/videos");

        assert_eq!(
            current_unreachable_folders(&registry),
            vec!["W:/video".to_string()]
        );
    }
}
