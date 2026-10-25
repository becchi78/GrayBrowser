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
//!
//! A dead share can keep a probe blocked for a minute, during which the
//! screens would show the folder as available. So a probe that has not
//! answered after `DEFAULT_PROVISIONAL_AFTER` is published as unreachable while
//! it keeps running, and its answer then confirms or retracts that. Threads
//! waiting for the answer never see the provisional state; they get the real
//! result, so the workers' skip and retry decisions are unaffected.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use gb_core::paths;
use gb_core::ports::folder_probe::FolderProbe;

use crate::events::ReachabilityNotifier;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// A thread is probing this folder right now; others wait for its answer.
    /// `token` identifies this probe, so a watcher or a finishing probe can
    /// tell its own marker from a newer probe's after the folder was forgotten
    /// and asked about again. `shown` is set once the probe has run long
    /// enough that the folder is published as unreachable ahead of the answer.
    Probing {
        token: u64,
        shown: bool,
    },
    Reachable,
    Unreachable,
}

impl Slot {
    /// Whether the screens are told this folder is unreachable.
    fn is_published_unreachable(self) -> bool {
        matches!(self, Slot::Unreachable | Slot::Probing { shown: true, .. })
    }
}

struct Inner {
    probe: Arc<dyn FolderProbe>,
    notifier: Arc<dyn ReachabilityNotifier>,
    /// A folder with no entry has never been probed.
    slots: Mutex<HashMap<String, Slot>>,
    settled: Condvar,
    next_token: AtomicU64,
    /// How long a probe may run before its folder is published as unreachable.
    provisional_after: Duration,
    /// Watcher threads still running; lets a test prove they all exit.
    #[cfg(test)]
    watchers_alive: std::sync::atomic::AtomicUsize,
    /// Serializes snapshot-then-emit so an older set cannot overtake a newer
    /// one on its way to the frontend.
    emit_gate: Mutex<()>,
    /// Called when a folder comes back. A hook rather than a `FollowUp` field
    /// because the follow-up's own action needs this registry to filter what
    /// it enqueues, so the two cannot be built in either order otherwise.
    on_recovered: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl Inner {
    fn unreachable_folders(&self) -> Vec<String> {
        let slots = lock(&self.slots);
        let mut folders: Vec<String> = slots
            .iter()
            .filter(|(_, slot)| slot.is_published_unreachable())
            .map(|(folder, _)| folder.clone())
            .collect();
        folders.sort();
        folders
    }

    /// Takes the snapshot inside the gate, after any earlier emit has finished,
    /// so the last event always carries the latest set. Never call with
    /// `slots` held: the gate is taken before `slots`, never after.
    fn emit_current(&self) {
        let _gate = lock(&self.emit_gate);
        let folders = self.unreachable_folders();
        self.notifier.notify_changed(&folders);
    }
}

/// How long a probe may run before its folder is published as unreachable
/// while the probe carries on. Short enough that a stalled share is flagged
/// well before the OS gives up on it (20s to a minute), long enough that a
/// slow but alive share does not flicker offline.
pub const DEFAULT_PROVISIONAL_AFTER: Duration = Duration::from_secs(5);

/// How often folders that are currently unreachable are probed again. Long
/// enough that a share which keeps failing costs next to nothing, short enough
/// that a drive plugged back in or a NAS that woke up is noticed within about
/// a minute rather than at the next (much slower) poll or a restart.
pub const DEFAULT_RECHECK_INTERVAL: Duration = Duration::from_secs(60);

/// Stops the thread started by `ReachabilityRegistry::start_recheck`. Dropping
/// it does not stop the thread: the thread is meant to live as long as the
/// process, and `stop` exists for tests and orderly shutdown.
pub struct RecheckHandle {
    stop: Arc<(Mutex<bool>, Condvar)>,
    #[cfg(test)]
    thread: Option<thread::JoinHandle<()>>,
}

impl RecheckHandle {
    /// Returns immediately; the thread exits at its next wake-up (at once if
    /// it is waiting, after the current probe if it is probing).
    pub fn stop(&self) {
        *lock(&self.stop.0) = true;
        self.stop.1.notify_all();
    }

    /// Stops the thread and waits until it has actually exited, so a test can
    /// assert "nothing happens afterwards" without guessing at timing.
    #[cfg(test)]
    fn stop_and_join(mut self) {
        self.stop();
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
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
/// stuck forever behind a probe that will never answer. A provisional
/// "unreachable" already published goes with it, and the screens are told, so
/// it cannot outlive the probe that justified it.
struct ProbeGuard<'a> {
    inner: &'a Inner,
    folder: &'a str,
    token: u64,
    armed: bool,
}

impl Drop for ProbeGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let was_shown = {
            let mut slots = lock(&self.inner.slots);
            let shown = match slots.get(self.folder).copied() {
                Some(Slot::Probing { token, shown }) if token == self.token => {
                    slots.remove(self.folder);
                    shown
                }
                _ => false,
            };
            self.inner.settled.notify_all();
            shown
        };
        if was_shown {
            // Runs while unwinding, where a second panic (a failing notifier)
            // would abort the process.
            let inner = self.inner;
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| inner.emit_current()));
        }
    }
}

/// Decrements the live-watcher count however the watcher thread ends.
#[cfg(test)]
struct WatcherCount<'a>(&'a Inner);

#[cfg(test)]
impl Drop for WatcherCount<'_> {
    fn drop(&mut self) {
        self.0.watchers_alive.fetch_sub(1, Ordering::SeqCst);
    }
}

impl ReachabilityRegistry {
    pub fn new(probe: Arc<dyn FolderProbe>, notifier: Arc<dyn ReachabilityNotifier>) -> Self {
        Self::with_provisional_after(probe, notifier, DEFAULT_PROVISIONAL_AFTER)
    }

    fn with_provisional_after(
        probe: Arc<dyn FolderProbe>,
        notifier: Arc<dyn ReachabilityNotifier>,
        provisional_after: Duration,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                probe,
                notifier,
                slots: Mutex::new(HashMap::new()),
                settled: Condvar::new(),
                next_token: AtomicU64::new(0),
                provisional_after,
                #[cfg(test)]
                watchers_alive: std::sync::atomic::AtomicUsize::new(0),
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
        self.resolve(folder, false, false)
    }

    /// Probes `folder` again regardless of what is cached and returns the
    /// fresh answer. A refresh that overlaps another probe of the same folder
    /// joins it instead of probing a second time.
    pub fn refresh(&self, folder: &str) -> bool {
        self.resolve(folder, true, false)
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

    /// The folders currently published as unreachable, sorted. This includes a
    /// folder whose probe is still running but has gone on long enough to be
    /// flagged provisionally.
    pub fn unreachable_folders(&self) -> Vec<String> {
        self.inner.unreachable_folders()
    }

    /// Starts the one thread that probes the currently unreachable folders
    /// again every `interval`. Only unreachable folders are probed, so a
    /// healthy folder costs nothing; this is what lets a folder that no poller
    /// covers (local, removable, update-check off) come back, and it shortens
    /// the NAS case too. A probe against a cold share may block for ~20s, which
    /// only delays this thread. The probes go through `refresh`, so they join
    /// any probe already in flight and fire the same event and recovery hook.
    ///
    /// The folders are probed one after another, and the next wait starts only
    /// after the whole pass is done, so a cold share stretches the interval by
    /// the time it blocks. That is accepted: recovery within about a minute
    /// does not need precision, and probing in parallel would multiply the
    /// threads stuck in the OS on a dead share for no gain. A panic while
    /// probing one folder is logged and the pass carries on, so one bad probe
    /// cannot silently end the rechecking for the rest of the session.
    pub fn start_recheck(&self, interval: Duration) -> RecheckHandle {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let registry = self.clone();
        let flag = Arc::clone(&stop);
        let thread = thread::spawn(move || loop {
            {
                let (stopped, cond) = &*flag;
                let guard = lock(stopped);
                let (guard, _) = cond
                    .wait_timeout_while(guard, interval, |stopped| !*stopped)
                    .unwrap_or_else(|e| e.into_inner());
                if *guard {
                    return;
                }
            }
            for folder in registry.unreachable_folders() {
                if *lock(&flag.0) {
                    return;
                }
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    registry.resolve(&folder, true, true)
                }));
                if outcome.is_err() {
                    log::error!("rechecking folder {folder} panicked; continuing");
                }
            }
        });
        #[cfg(not(test))]
        drop(thread);
        RecheckHandle {
            stop,
            #[cfg(test)]
            thread: Some(thread),
        }
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
                if !kept && slot.is_published_unreachable() {
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

    /// `only_if_known` makes a folder with no entry answer "unreachable"
    /// without probing: the rechecker works from a snapshot, and a folder
    /// forgotten since then must not be probed and registered again.
    fn resolve(&self, folder: &str, force: bool, only_if_known: bool) -> bool {
        let inner = &*self.inner;
        let mut slots = lock(&inner.slots);
        let mut waited = false;
        let previous = loop {
            match slots.get(folder).copied() {
                Some(Slot::Probing { .. }) => {
                    slots = inner.settled.wait(slots).unwrap_or_else(|e| e.into_inner());
                    waited = true;
                }
                // A forced refresh that waited for someone else's probe takes
                // that answer: it is as fresh as one it would have made.
                Some(Slot::Reachable) if !force || waited => return true,
                Some(Slot::Unreachable) if !force || waited => return false,
                None if only_if_known => return false,
                other => break other,
            }
        };
        let token = inner.next_token.fetch_add(1, Ordering::Relaxed);
        slots.insert(
            folder.to_string(),
            Slot::Probing {
                token,
                shown: false,
            },
        );
        drop(slots);

        let mut guard = ProbeGuard {
            inner,
            folder,
            token,
            armed: true,
        };
        self.spawn_watcher(folder, token);
        let reachable = inner.probe.is_reachable(folder);
        guard.armed = false;

        // `shown` is read under the lock that replaces the marker, so it is
        // exactly what the screens had been told when the answer arrived.
        let shown = {
            let mut slots = lock(&inner.slots);
            // The marker is gone (or is a newer probe's) if the folder was
            // forgotten mid-probe; the answer is still returned to the caller
            // but not recorded.
            let shown = match slots.get(folder).copied() {
                Some(Slot::Probing { token: t, shown }) if t == token => Some(shown),
                _ => None,
            };
            if shown.is_some() {
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
            shown
        };

        if let Some(shown) = shown {
            let was_unreachable = previous == Some(Slot::Unreachable);
            // Compare what the screens were told before with what they should
            // be told now: a provisional flag that the answer confirms needs
            // no second event.
            let was_published = was_unreachable || shown;
            if was_published != !reachable {
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

    /// Starts the short-lived thread that flags `folder` as unreachable if the
    /// probe `token` is still running after `provisional_after`. One per probe
    /// rather than one shared timer: probes are rare (startup, the rechecks,
    /// the NAS polls), and nearly all of them answer at once, which ends their
    /// watcher at once.
    fn spawn_watcher(&self, folder: &str, token: u64) {
        let registry = self.clone();
        let folder = folder.to_string();
        #[cfg(test)]
        self.inner.watchers_alive.fetch_add(1, Ordering::SeqCst);
        let spawned = thread::Builder::new()
            .name("reachability-watch".into())
            .spawn(move || {
                #[cfg(test)]
                let _count = WatcherCount(&registry.inner);
                registry.watch(&folder, token);
            });
        if spawned.is_err() {
            // Without a watcher the folder is just not flagged early; the
            // probe's answer still decides the final state.
            #[cfg(test)]
            self.inner.watchers_alive.fetch_sub(1, Ordering::SeqCst);
            log::warn!("could not start the reachability watcher");
        }
    }

    /// Waits until the probe `token` ends (any change of its marker, including
    /// the folder being forgotten) or `provisional_after` passes. Only the
    /// second case does anything: flag the folder and tell the screens. The
    /// token check keeps this from flagging a later probe of the same folder.
    fn watch(&self, folder: &str, token: u64) {
        let inner = &*self.inner;
        let deadline = Instant::now() + inner.provisional_after;
        let mut slots = lock(&inner.slots);
        loop {
            match slots.get(folder).copied() {
                Some(Slot::Probing { token: t, .. }) if t == token => {}
                _ => return,
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            // Spurious and unrelated wake-ups just re-check the marker.
            slots = inner
                .settled
                .wait_timeout(slots, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        slots.insert(folder.to_string(), Slot::Probing { token, shown: true });
        drop(slots);
        self.emit_current();
    }

    fn emit_current(&self) {
        self.inner.emit_current();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{FakeReachabilityNotifier, ReachabilityNotifier};
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

    const TICK: Duration = Duration::from_millis(30);

    fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..200 {
            if cond() {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        cond()
    }

    #[test]
    fn recheck_brings_back_an_unreachable_folder_that_no_poller_covers() {
        let (registry, probe, notifier) =
            registry_with(FakeFolderProbe::with_unreachable(&[LOCAL]));
        let fired = Arc::new(AtomicUsize::new(0));
        {
            let fired = fired.clone();
            registry.set_on_recovered(move || {
                fired.fetch_add(1, Ordering::SeqCst);
            });
        }
        assert!(!registry.is_reachable(LOCAL));
        let handle = registry.start_recheck(TICK);

        probe.set_reachable(LOCAL, true);

        // The hook is the last step of a recovery (after the state change and
        // the event), so waiting for it covers all three.
        assert!(wait_until(|| fired.load(Ordering::SeqCst) == 1));
        handle.stop_and_join();
        assert_eq!(fired.load(Ordering::SeqCst), 1);
        assert!(registry.unreachable_folders().is_empty());
        assert_eq!(
            events(&notifier),
            vec![vec![LOCAL.to_string()], Vec::<String>::new()]
        );
    }

    #[test]
    fn recheck_leaves_reachable_and_forgotten_folders_alone() {
        let (registry, probe, _) =
            registry_with(FakeFolderProbe::with_unreachable(&[NAS, "D:/gone"]));
        registry.is_reachable(LOCAL);
        registry.is_reachable(NAS);
        registry.is_reachable("D:/gone");
        registry.retain_folders(&[LOCAL.to_string(), NAS.to_string()]);
        let handle = registry.start_recheck(TICK);

        assert!(wait_until(|| probe.call_count_for(NAS) >= 3));
        handle.stop_and_join();

        assert_eq!(probe.call_count_for(LOCAL), 1, "reachable: never rechecked");
        assert_eq!(
            probe.call_count_for("D:/gone"),
            1,
            "forgotten: never rechecked"
        );
    }

    #[test]
    fn recheck_stops_when_asked() {
        let (registry, probe, _) = registry_with(FakeFolderProbe::with_unreachable(&[NAS]));
        registry.is_reachable(NAS);
        let handle = registry.start_recheck(TICK);
        assert!(wait_until(|| probe.call_count_for(NAS) >= 2));

        handle.stop_and_join();
        let after_exit = probe.call_count_for(NAS);
        thread::sleep(TICK * 6);

        assert_eq!(probe.call_count_for(NAS), after_exit);
    }

    /// Runs `on_probe` with the folder and its probe count every time a
    /// folder is probed, so a test can interleave another action at an exact
    /// point of a recheck pass. Always answers "unreachable".
    type ProbeHook = Box<dyn Fn(&str, usize) + Send + Sync>;

    struct HookedProbe {
        counts: Mutex<HashMap<String, usize>>,
        on_probe: ProbeHook,
    }

    impl HookedProbe {
        fn count(&self, folder: &str) -> usize {
            self.counts
                .lock()
                .unwrap()
                .get(folder)
                .copied()
                .unwrap_or(0)
        }
    }

    impl FolderProbe for HookedProbe {
        fn is_reachable(&self, folder: &str) -> bool {
            let n = {
                let mut counts = self.counts.lock().unwrap();
                let n = counts.entry(folder.to_string()).or_insert(0);
                *n += 1;
                *n
            };
            (self.on_probe)(folder, n);
            false
        }
    }

    #[test]
    fn recheck_does_not_resurrect_a_folder_forgotten_during_the_pass() {
        const A: &str = "C:/a";
        const B: &str = "C:/b";
        const C: &str = "C:/c";
        // While A is being rechecked (its second probe), A and B are removed
        // and C stays. The pass already holds B in its snapshot.
        let registry_slot: Arc<Mutex<Option<ReachabilityRegistry>>> = Arc::new(Mutex::new(None));
        let hook_slot = registry_slot.clone();
        let probe = Arc::new(HookedProbe {
            counts: Mutex::new(HashMap::new()),
            on_probe: Box::new(move |folder, n| {
                if folder == A && n == 2 {
                    let registry = hook_slot.lock().unwrap().clone().unwrap();
                    registry.retain_folders(&[C.to_string()]);
                }
            }),
        });
        let registry =
            ReachabilityRegistry::new(probe.clone(), Arc::new(FakeReachabilityNotifier::default()));
        *registry_slot.lock().unwrap() = Some(registry.clone());
        for folder in [A, B, C] {
            registry.is_reachable(folder);
        }

        let handle = registry.start_recheck(TICK);
        // C follows B in the pass, so C's second probe proves the pass went
        // past B.
        assert!(wait_until(|| probe.count(C) >= 2));
        handle.stop_and_join();

        assert_eq!(probe.count(B), 1, "B was forgotten and must not be probed");
        assert_eq!(registry.unreachable_folders(), vec![C.to_string()]);
    }

    struct PanicOnceNotifier {
        armed: std::sync::atomic::AtomicBool,
    }

    impl ReachabilityNotifier for PanicOnceNotifier {
        fn notify_changed(&self, _unreachable: &[String]) {
            if self.armed.swap(false, Ordering::SeqCst) {
                panic!("notifier failure injected by the test");
            }
        }
    }

    #[test]
    fn recheck_survives_a_panic_and_keeps_rechecking() {
        const THIRD: &str = "X:/x";
        let probe = Arc::new(FakeFolderProbe::with_unreachable(&[LOCAL, NAS, THIRD]));
        let notifier = Arc::new(PanicOnceNotifier {
            armed: std::sync::atomic::AtomicBool::new(false),
        });
        let registry = ReachabilityRegistry::new(probe.clone(), notifier.clone());
        let fired = Arc::new(AtomicUsize::new(0));
        {
            let fired = fired.clone();
            registry.set_on_recovered(move || {
                fired.fetch_add(1, Ordering::SeqCst);
            });
        }
        for folder in [LOCAL, NAS, THIRD] {
            registry.is_reachable(folder);
        }
        // Armed only now, so the setup above does not trip it.
        notifier.armed.store(true, Ordering::SeqCst);
        let handle = registry.start_recheck(TICK);

        // LOCAL sorts first, so its recovery hits the panic; NAS, next in the
        // same pass, must still be rechecked.
        probe.set_reachable(LOCAL, true);
        probe.set_reachable(NAS, true);
        assert!(wait_until(|| fired.load(Ordering::SeqCst) == 1));

        // And a later pass still runs.
        probe.set_reachable(THIRD, true);
        assert!(wait_until(|| fired.load(Ordering::SeqCst) == 2));
        handle.stop_and_join();
        assert!(registry.unreachable_folders().is_empty());
    }

    /// Blocks every probe until `release`, so a test decides exactly when the
    /// answer arrives instead of racing a sleep against the watcher.
    struct GatedProbe {
        released: Mutex<bool>,
        cond: Condvar,
        entered: AtomicUsize,
        reachable: std::sync::atomic::AtomicBool,
        panic_on_release: std::sync::atomic::AtomicBool,
    }

    impl GatedProbe {
        fn new(reachable: bool) -> Arc<Self> {
            Arc::new(Self {
                released: Mutex::new(false),
                cond: Condvar::new(),
                entered: AtomicUsize::new(0),
                reachable: std::sync::atomic::AtomicBool::new(reachable),
                panic_on_release: std::sync::atomic::AtomicBool::new(false),
            })
        }

        fn release(&self) {
            *self.released.lock().unwrap() = true;
            self.cond.notify_all();
        }
    }

    impl FolderProbe for GatedProbe {
        fn is_reachable(&self, _folder: &str) -> bool {
            self.entered.fetch_add(1, Ordering::SeqCst);
            // Not held across the panic below, which would poison the lock.
            drop(
                self.cond
                    .wait_while(self.released.lock().unwrap(), |r| !*r)
                    .unwrap(),
            );
            if self.panic_on_release.load(Ordering::SeqCst) {
                panic!("probe failure injected by the test");
            }
            self.reachable.load(Ordering::SeqCst)
        }
    }

    const SOON: Duration = Duration::from_millis(30);

    fn gated_registry(
        reachable: bool,
    ) -> (
        ReachabilityRegistry,
        Arc<GatedProbe>,
        Arc<FakeReachabilityNotifier>,
    ) {
        let probe = GatedProbe::new(reachable);
        let notifier = Arc::new(FakeReachabilityNotifier::default());
        let registry =
            ReachabilityRegistry::with_provisional_after(probe.clone(), notifier.clone(), SOON);
        (registry, probe, notifier)
    }

    fn watchers_alive(registry: &ReachabilityRegistry) -> usize {
        registry.inner.watchers_alive.load(Ordering::SeqCst)
    }

    /// Starts a probe of `NAS` that stays in flight and waits until the
    /// folder has been flagged unreachable.
    fn start_slow_probe_until_flagged(
        registry: &ReachabilityRegistry,
        notifier: &FakeReachabilityNotifier,
    ) -> thread::JoinHandle<bool> {
        let r = registry.clone();
        let t = thread::spawn(move || r.is_reachable(NAS));
        assert!(wait_until(|| events(notifier).len() == 1));
        t
    }

    #[test]
    fn a_slow_probe_that_ends_unreachable_is_flagged_once() {
        let (registry, probe, notifier) = gated_registry(false);

        let t = start_slow_probe_until_flagged(&registry, &notifier);

        assert_eq!(registry.unreachable_folders(), vec![NAS.to_string()]);
        assert_eq!(events(&notifier), vec![vec![NAS.to_string()]]);
        assert_eq!(probe.entered.load(Ordering::SeqCst), 1, "still probing");

        probe.release();
        assert!(!t.join().unwrap());

        assert_eq!(registry.unreachable_folders(), vec![NAS.to_string()]);
        assert_eq!(events(&notifier).len(), 1, "confirming adds no event");
        assert!(wait_until(|| watchers_alive(&registry) == 0));
    }

    #[test]
    fn a_slow_probe_that_ends_reachable_retracts_the_flag_without_a_recovery() {
        let (registry, probe, notifier) = gated_registry(true);
        let fired = Arc::new(AtomicUsize::new(0));
        {
            let fired = fired.clone();
            registry.set_on_recovered(move || {
                fired.fetch_add(1, Ordering::SeqCst);
            });
        }

        let t = start_slow_probe_until_flagged(&registry, &notifier);
        // A second caller waits for the real answer and never sees the flag.
        let r = registry.clone();
        let waiter = thread::spawn(move || r.is_reachable(NAS));

        probe.release();

        assert!(t.join().unwrap());
        assert!(waiter.join().unwrap());
        assert!(registry.unreachable_folders().is_empty());
        assert_eq!(
            events(&notifier),
            vec![vec![NAS.to_string()], Vec::<String>::new()]
        );
        assert_eq!(fired.load(Ordering::SeqCst), 0);
        assert_eq!(probe.entered.load(Ordering::SeqCst), 1, "one shared probe");
        assert!(wait_until(|| watchers_alive(&registry) == 0));
    }

    #[test]
    fn a_fast_probe_flags_nothing_and_its_watcher_exits_early() {
        // The default delay is seconds; the watcher ending within wait_until's
        // budget proves it was woken by the answer and did not run it out.
        let probe = Arc::new(FakeFolderProbe::with_unreachable(&[NAS]));
        let notifier = Arc::new(FakeReachabilityNotifier::default());
        let registry = ReachabilityRegistry::new(probe, notifier.clone());

        assert!(registry.is_reachable(LOCAL));
        assert!(wait_until(|| watchers_alive(&registry) == 0));
        assert!(events(&notifier).is_empty());
        assert!(registry.unreachable_folders().is_empty());

        // Even a fast unreachable answer is announced exactly once, as before.
        assert!(!registry.is_reachable(NAS));
        assert!(wait_until(|| watchers_alive(&registry) == 0));
        assert_eq!(events(&notifier), vec![vec![NAS.to_string()]]);
    }

    #[test]
    fn forgetting_a_flagged_folder_unflags_it_and_the_late_answer_is_not_recorded() {
        let (registry, probe, notifier) = gated_registry(false);
        let t = start_slow_probe_until_flagged(&registry, &notifier);

        registry.retain_folders(&[]);

        assert!(registry.unreachable_folders().is_empty());
        assert_eq!(
            events(&notifier),
            vec![vec![NAS.to_string()], Vec::<String>::new()]
        );

        probe.release();
        assert!(!t.join().unwrap());

        assert!(registry.unreachable_folders().is_empty());
        assert_eq!(events(&notifier).len(), 2, "no event from the late answer");
        // Not recorded, so the next question probes afresh.
        assert!(!registry.is_reachable(NAS));
        assert_eq!(probe.entered.load(Ordering::SeqCst), 2);
        assert!(wait_until(|| watchers_alive(&registry) == 0));
    }

    #[test]
    fn a_probe_that_panics_after_being_flagged_leaves_no_flag_behind() {
        let (registry, probe, notifier) = gated_registry(false);
        let t = start_slow_probe_until_flagged(&registry, &notifier);

        probe.panic_on_release.store(true, Ordering::SeqCst);
        probe.release();

        assert!(t.join().is_err(), "the probe's panic reaches its caller");
        assert!(registry.unreachable_folders().is_empty());
        assert_eq!(
            events(&notifier),
            vec![vec![NAS.to_string()], Vec::<String>::new()]
        );
        assert!(wait_until(|| watchers_alive(&registry) == 0));

        // The folder is unknown again, not stuck as "being probed".
        probe.panic_on_release.store(false, Ordering::SeqCst);
        assert!(!registry.is_reachable(NAS));
    }

    #[test]
    fn a_panicking_probe_that_was_never_flagged_emits_nothing() {
        let probe = GatedProbe::new(true);
        probe.panic_on_release.store(true, Ordering::SeqCst);
        probe.release();
        let notifier = Arc::new(FakeReachabilityNotifier::default());
        // Far longer than the test, so the flag cannot appear.
        let registry = ReachabilityRegistry::with_provisional_after(
            probe.clone(),
            notifier.clone(),
            Duration::from_secs(60),
        );
        let r = registry.clone();

        assert!(thread::spawn(move || r.is_reachable(NAS)).join().is_err());

        assert!(events(&notifier).is_empty());
        assert!(wait_until(|| watchers_alive(&registry) == 0));
    }

    #[test]
    fn a_recheck_that_runs_long_is_flagged_again_and_still_recovers_through_the_hook() {
        let (registry, probe, notifier) = gated_registry(false);
        probe.release();
        assert!(!registry.is_reachable(NAS));
        assert_eq!(events(&notifier), vec![vec![NAS.to_string()]]);
        let fired = Arc::new(AtomicUsize::new(0));
        {
            let fired = fired.clone();
            registry.set_on_recovered(move || {
                fired.fetch_add(1, Ordering::SeqCst);
            });
        }
        *probe.released.lock().unwrap() = false;
        probe.reachable.store(true, Ordering::SeqCst);

        let r = registry.clone();
        let t = thread::spawn(move || r.refresh(NAS));
        // While a recheck's marker is not yet flagged the folder is absent
        // from the set; the flag puts it back.
        assert!(wait_until(|| events(&notifier).len() == 2));
        assert_eq!(registry.unreachable_folders(), vec![NAS.to_string()]);

        probe.release();
        assert!(t.join().unwrap());

        assert!(registry.unreachable_folders().is_empty());
        assert_eq!(fired.load(Ordering::SeqCst), 1, "a real recovery");
        assert_eq!(events(&notifier).last(), Some(&Vec::<String>::new()));
        assert!(wait_until(|| watchers_alive(&registry) == 0));
    }
}
