//! In-memory record of which watch folders can be reached right now.
//!
//! A video's `status` is deliberately left alone when its folder goes
//! unreachable: "cannot list the share" is not "the files were deleted", and
//! flipping rows offline on a transient outage would be the mass-offline
//! failure the scan guards exist to prevent. This registry carries the
//! "unreachable right now" fact separately, in memory only, so the screens can
//! show it and the background workers can avoid opening files on a dead share
//! (each attempt blocks for tens of seconds and burns a retry).
//!
//! Probing a dead share blocks inside the OS, so every probe happens on the
//! caller's thread and the registry makes sure a folder is never probed by two
//! threads at once: the second caller waits for the first one's answer.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use gb_core::paths;
use gb_core::ports::folder_probe::FolderProbe;

use crate::events::ReachabilityNotifier;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// A thread is probing this folder right now; others wait for its answer.
    Probing,
    Reachable,
    Unreachable,
}

struct Inner {
    probe: Arc<dyn FolderProbe>,
    notifier: Arc<dyn ReachabilityNotifier>,
    /// A folder with no entry has never been probed.
    slots: Mutex<HashMap<String, Slot>>,
    settled: Condvar,
    /// Serializes snapshot-then-emit so an older set cannot overtake a newer
    /// one on its way to the frontend.
    emit_gate: Mutex<()>,
    /// Called when a folder comes back. A hook rather than a `FollowUp` field
    /// because the follow-up's own action needs this registry to filter what
    /// it enqueues, so the two cannot be built in either order otherwise.
    on_recovered: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

/// Cheap to clone; every clone shares the same state.
#[derive(Clone)]
pub struct ReachabilityRegistry {
    inner: Arc<Inner>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Clears a `Probing` marker if the probe panics, so waiting threads are not
/// stuck forever behind a probe that will never answer.
struct ProbeGuard<'a> {
    inner: &'a Inner,
    folder: &'a str,
    armed: bool,
}

impl Drop for ProbeGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut slots = lock(&self.inner.slots);
        if slots.get(self.folder) == Some(&Slot::Probing) {
            slots.remove(self.folder);
        }
        self.inner.settled.notify_all();
    }
}

impl ReachabilityRegistry {
    pub fn new(probe: Arc<dyn FolderProbe>, notifier: Arc<dyn ReachabilityNotifier>) -> Self {
        Self {
            inner: Arc::new(Inner {
                probe,
                notifier,
                slots: Mutex::new(HashMap::new()),
                settled: Condvar::new(),
                emit_gate: Mutex::new(()),
                on_recovered: Mutex::new(None),
            }),
        }
    }

    /// Registers what to run when a previously unreachable folder is reachable
    /// again (used to resume the thumbnail/metadata work that was skipped).
    pub fn set_on_recovered(&self, hook: impl Fn() + Send + Sync + 'static) {
        *lock(&self.inner.on_recovered) = Some(Arc::new(hook));
    }

    /// Whether `folder` is reachable, probing it only if it has never been
    /// probed. May block for a long time on the first call for an unreachable
    /// share, so never call it from the UI thread. Concurrent first calls for
    /// the same folder share a single probe.
    pub fn is_reachable(&self, folder: &str) -> bool {
        self.resolve(folder, false)
    }

    /// Probes `folder` again regardless of what is cached and returns the
    /// fresh answer. A refresh that overlaps another probe of the same folder
    /// joins it instead of probing a second time.
    pub fn refresh(&self, folder: &str) -> bool {
        self.resolve(folder, true)
    }

    /// Whether `path` belongs to a watch folder that is currently unreachable.
    /// A path outside every watch folder is never reported unreachable.
    pub fn is_path_unreachable(&self, path: &str, watch_folders: &[String]) -> bool {
        paths::owning_folder(path, watch_folders).is_some_and(|folder| !self.is_reachable(folder))
    }

    /// Like `is_path_unreachable` but probes again first: used after a
    /// failure, to tell "the file is bad" from "the share just went away".
    pub fn confirm_path_unreachable(&self, path: &str, watch_folders: &[String]) -> bool {
        paths::owning_folder(path, watch_folders).is_some_and(|folder| !self.refresh(folder))
    }

    /// The folders currently known to be unreachable, sorted.
    pub fn unreachable_folders(&self) -> Vec<String> {
        let slots = lock(&self.inner.slots);
        let mut folders: Vec<String> = slots
            .iter()
            .filter(|(_, slot)| **slot == Slot::Unreachable)
            .map(|(folder, _)| folder.clone())
            .collect();
        folders.sort();
        folders
    }

    /// Forgets every folder not in `keep` (a folder removed, renamed, or no
    /// longer polled). A forgotten folder is probed afresh the next time
    /// something asks, so a stale "unreachable" cannot outlive the folder's
    /// registration.
    pub fn retain_folders(&self, keep: &[String]) {
        let dropped_unreachable = {
            let mut slots = lock(&self.inner.slots);
            let mut dropped_unreachable = false;
            slots.retain(|folder, slot| {
                let kept = keep.contains(folder);
                if !kept && *slot == Slot::Unreachable {
                    dropped_unreachable = true;
                }
                kept
            });
            self.inner.settled.notify_all();
            dropped_unreachable
        };
        if dropped_unreachable {
            self.emit_current();
        }
    }

    fn resolve(&self, folder: &str, force: bool) -> bool {
        let inner = &*self.inner;
        let mut slots = lock(&inner.slots);
        let mut waited = false;
        let previous = loop {
            match slots.get(folder).copied() {
                Some(Slot::Probing) => {
                    slots = inner.settled.wait(slots).unwrap_or_else(|e| e.into_inner());
                    waited = true;
                }
                // A forced refresh that waited for someone else's probe takes
                // that answer: it is as fresh as one it would have made.
                Some(Slot::Reachable) if !force || waited => return true,
                Some(Slot::Unreachable) if !force || waited => return false,
                other => break other,
            }
        };
        slots.insert(folder.to_string(), Slot::Probing);
        drop(slots);

        let mut guard = ProbeGuard {
            inner,
            folder,
            armed: true,
        };
        let reachable = inner.probe.is_reachable(folder);
        guard.armed = false;

        let stored = {
            let mut slots = lock(&inner.slots);
            // The entry is gone if the folder was forgotten mid-probe; the
            // answer is still returned to the caller but not recorded.
            let stored = slots.get(folder) == Some(&Slot::Probing);
            if stored {
                slots.insert(
                    folder.to_string(),
                    if reachable {
                        Slot::Reachable
                    } else {
                        Slot::Unreachable
                    },
                );
            }
            inner.settled.notify_all();
            stored
        };

        if stored {
            let was_unreachable = previous == Some(Slot::Unreachable);
            if was_unreachable != !reachable {
                // The unreachable set changed (the folder went in or out).
                self.emit_current();
            }
            if was_unreachable && reachable {
                log::info!("folder is reachable again: {folder}");
                let hook = lock(&inner.on_recovered).clone();
                if let Some(hook) = hook {
                    hook();
                }
            } else if !reachable && !was_unreachable {
                log::warn!("folder is unreachable: {folder}");
            }
        }
        reachable
    }

    fn emit_current(&self) {
        let _gate = lock(&self.inner.emit_gate);
        let folders = self.unreachable_folders();
        self.inner.notifier.notify_changed(&folders);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::FakeReachabilityNotifier;
    use gb_core::testing::fake_folder_probe::FakeFolderProbe;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    const NAS: &str = "W:/video";
    const LOCAL: &str = "C:/videos";

    fn registry_with(
        probe: FakeFolderProbe,
    ) -> (
        ReachabilityRegistry,
        Arc<FakeFolderProbe>,
        Arc<FakeReachabilityNotifier>,
    ) {
        let probe = Arc::new(probe);
        let notifier = Arc::new(FakeReachabilityNotifier::default());
        let registry = ReachabilityRegistry::new(probe.clone(), notifier.clone());
        (registry, probe, notifier)
    }

    fn events(n: &FakeReachabilityNotifier) -> Vec<Vec<String>> {
        n.calls.lock().unwrap().clone()
    }

    #[test]
    fn an_unprobed_folder_is_probed_once_and_then_cached() {
        let (registry, probe, _) = registry_with(FakeFolderProbe::with_unreachable(&[NAS]));

        assert!(!registry.is_reachable(NAS));
        assert!(!registry.is_reachable(NAS));
        assert!(registry.is_reachable(LOCAL));
        assert!(registry.is_reachable(LOCAL));

        assert_eq!(probe.call_count_for(NAS), 1);
        assert_eq!(probe.call_count_for(LOCAL), 1);
    }

    #[test]
    fn concurrent_first_checks_share_one_probe() {
        let probe =
            FakeFolderProbe::with_unreachable(&[NAS]).with_delay(Duration::from_millis(150));
        let (registry, probe, _) = registry_with(probe);

        let threads: Vec<_> = (0..8)
            .map(|_| {
                let registry = registry.clone();
                thread::spawn(move || registry.is_reachable(NAS))
            })
            .collect();
        for t in threads {
            assert!(!t.join().unwrap());
        }

        assert_eq!(probe.call_count_for(NAS), 1);
    }

    #[test]
    fn overlapping_refreshes_join_the_probe_in_flight() {
        let probe = FakeFolderProbe::default().with_delay(Duration::from_millis(150));
        let (registry, probe, _) = registry_with(probe);

        let threads: Vec<_> = (0..4)
            .map(|_| {
                let registry = registry.clone();
                thread::spawn(move || registry.refresh(LOCAL))
            })
            .collect();
        for t in threads {
            assert!(t.join().unwrap());
        }

        assert_eq!(probe.call_count_for(LOCAL), 1);
    }

    #[test]
    fn refresh_probes_again_even_when_cached() {
        let (registry, probe, _) = registry_with(FakeFolderProbe::default());
        assert!(registry.is_reachable(NAS));
        probe.set_reachable(NAS, false);

        assert!(!registry.refresh(NAS));

        assert_eq!(probe.call_count_for(NAS), 2);
        assert_eq!(registry.unreachable_folders(), vec![NAS.to_string()]);
    }

    #[test]
    fn an_event_is_emitted_only_when_the_unreachable_set_changes() {
        let (registry, probe, notifier) = registry_with(FakeFolderProbe::with_unreachable(&[NAS]));

        registry.is_reachable(LOCAL);
        assert!(
            events(&notifier).is_empty(),
            "reachable first probe: no change"
        );

        registry.is_reachable(NAS);
        assert_eq!(events(&notifier), vec![vec![NAS.to_string()]]);

        registry.refresh(NAS);
        registry.refresh(LOCAL);
        assert_eq!(events(&notifier).len(), 1, "unchanged answers emit nothing");

        probe.set_reachable(NAS, true);
        registry.refresh(NAS);
        assert_eq!(
            events(&notifier),
            vec![vec![NAS.to_string()], Vec::<String>::new()]
        );
    }

    #[test]
    fn a_folder_going_down_after_being_reachable_emits() {
        let (registry, probe, notifier) = registry_with(FakeFolderProbe::default());
        registry.is_reachable(NAS);
        probe.set_reachable(NAS, false);

        registry.refresh(NAS);

        assert_eq!(events(&notifier), vec![vec![NAS.to_string()]]);
    }

    #[test]
    fn recovery_runs_the_hook_once_and_only_on_recovery() {
        let (registry, probe, _) = registry_with(FakeFolderProbe::with_unreachable(&[NAS]));
        let fired = Arc::new(AtomicUsize::new(0));
        {
            let fired = fired.clone();
            registry.set_on_recovered(move || {
                fired.fetch_add(1, Ordering::SeqCst);
            });
        }

        registry.is_reachable(NAS);
        registry.refresh(NAS);
        assert_eq!(fired.load(Ordering::SeqCst), 0, "still down: no recovery");

        probe.set_reachable(NAS, true);
        registry.refresh(NAS);
        assert_eq!(fired.load(Ordering::SeqCst), 1);

        registry.refresh(NAS);
        assert_eq!(
            fired.load(Ordering::SeqCst),
            1,
            "already up: no second fire"
        );
    }

    #[test]
    fn a_first_probe_that_finds_the_folder_up_is_not_a_recovery() {
        let (registry, _, _) = registry_with(FakeFolderProbe::default());
        let fired = Arc::new(AtomicUsize::new(0));
        {
            let fired = fired.clone();
            registry.set_on_recovered(move || {
                fired.fetch_add(1, Ordering::SeqCst);
            });
        }

        registry.is_reachable(NAS);

        assert_eq!(fired.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn path_checks_use_the_owning_folder_and_respect_boundaries() {
        let (registry, probe, _) = registry_with(FakeFolderProbe::with_unreachable(&[NAS]));
        let folders = vec![NAS.to_string(), LOCAL.to_string()];

        assert!(registry.is_path_unreachable("W:/video/a.mp4", &folders));
        assert!(!registry.is_path_unreachable("W:/video2/a.mp4", &folders));
        assert!(!registry.is_path_unreachable("C:/videos/a.mp4", &folders));
        assert!(!registry.is_path_unreachable("D:/other/a.mp4", &folders));

        assert_eq!(probe.call_count_for(NAS), 1);
        assert_eq!(probe.call_count_for(LOCAL), 1);
        assert_eq!(probe.calls().len(), 2, "unowned paths probe nothing");
    }

    #[test]
    fn confirm_path_unreachable_reprobes() {
        let (registry, probe, _) = registry_with(FakeFolderProbe::default());
        let folders = vec![NAS.to_string()];
        assert!(!registry.is_path_unreachable("W:/video/a.mp4", &folders));

        probe.set_reachable(NAS, false);

        assert!(registry.confirm_path_unreachable("W:/video/a.mp4", &folders));
        assert_eq!(probe.call_count_for(NAS), 2);
    }

    #[test]
    fn retain_forgets_removed_folders_and_emits_if_one_was_unreachable() {
        let (registry, probe, notifier) = registry_with(FakeFolderProbe::with_unreachable(&[NAS]));
        registry.is_reachable(NAS);
        registry.is_reachable(LOCAL);
        assert_eq!(events(&notifier).len(), 1);

        registry.retain_folders(&[LOCAL.to_string()]);

        assert!(registry.unreachable_folders().is_empty());
        assert_eq!(
            events(&notifier),
            vec![vec![NAS.to_string()], Vec::<String>::new()]
        );
        // Forgotten, so the next question probes again; the kept one is cached.
        registry.is_reachable(NAS);
        registry.is_reachable(LOCAL);
        assert_eq!(probe.call_count_for(NAS), 2);
        assert_eq!(probe.call_count_for(LOCAL), 1);
    }

    #[test]
    fn retain_that_drops_only_reachable_folders_emits_nothing() {
        let (registry, _, notifier) = registry_with(FakeFolderProbe::default());
        registry.is_reachable(LOCAL);

        registry.retain_folders(&[]);

        assert!(events(&notifier).is_empty());
    }
}
