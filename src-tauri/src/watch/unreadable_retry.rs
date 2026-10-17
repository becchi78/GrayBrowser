//! Remembers files that could not be read yet and re-evaluates them later.
//!
//! While Explorer copies a file, the destination is held with an exclusive
//! share mode, so hashing it fails with a sharing violation. The copy emits
//! its Created/Modified events during that window, and nothing guarantees
//! another event once the lock is released, so without a retry such a file
//! stays unregistered until the next restart. `defer` only records the path
//! under a mutex; a single worker thread re-evaluates the pending paths at a
//! fixed interval, so a bulk copy neither blocks the event callback nor piles
//! up threads.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// How often pending paths are re-evaluated.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(5);
/// A copy of a very large file can hold the lock for tens of minutes; past
/// this since the first deferral the path is dropped so a file that never
/// becomes readable cannot be retried forever.
pub const DEFAULT_GIVE_UP: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryOutcome {
    /// Handled (registered, or nothing left to do); stop tracking the path.
    Done,
    /// Still locked/unreadable; keep it pending until the give-up time.
    StillUnreadable,
    /// The file no longer exists; stop tracking the path.
    Gone,
}

#[derive(Default)]
struct State {
    /// Path -> when it was first deferred.
    pending: HashMap<PathBuf, Instant>,
    shutdown: bool,
}

struct Inner {
    state: Mutex<State>,
    cond: Condvar,
    interval: Duration,
    give_up: Duration,
    retry: Box<dyn Fn(&Path) -> RetryOutcome + Send + Sync>,
}

pub struct UnreadableRetry {
    inner: Option<Arc<Inner>>,
}

impl UnreadableRetry {
    /// An `UnreadableRetry` whose `defer` does nothing and owns no thread; used
    /// where no retry is wanted (tests).
    pub fn noop() -> Self {
        Self { inner: None }
    }

    pub fn new(retry: impl Fn(&Path) -> RetryOutcome + Send + Sync + 'static) -> Self {
        Self::with_timing(DEFAULT_INTERVAL, DEFAULT_GIVE_UP, retry)
    }

    /// Timings are injectable so tests can use millisecond values.
    pub fn with_timing(
        interval: Duration,
        give_up: Duration,
        retry: impl Fn(&Path) -> RetryOutcome + Send + Sync + 'static,
    ) -> Self {
        let inner = Arc::new(Inner {
            state: Mutex::new(State::default()),
            cond: Condvar::new(),
            interval,
            give_up,
            retry: Box::new(retry),
        });
        let worker = Arc::clone(&inner);
        thread::spawn(move || worker_loop(&worker));
        Self { inner: Some(inner) }
    }

    /// Returns immediately. Deferring a path that is already pending keeps its
    /// original time, so repeated events cannot extend the give-up deadline.
    pub fn defer(&self, path: &Path) {
        let Some(inner) = &self.inner else { return };
        let mut state = inner.state.lock().unwrap();
        state
            .pending
            .entry(path.to_path_buf())
            .or_insert_with(Instant::now);
        inner.cond.notify_all();
    }

    pub fn remove(&self, path: &Path) {
        let Some(inner) = &self.inner else { return };
        inner.state.lock().unwrap().pending.remove(path);
    }

    #[cfg(test)]
    pub(crate) fn pending_count(&self) -> usize {
        self.inner
            .as_ref()
            .map_or(0, |inner| inner.state.lock().unwrap().pending.len())
    }
}

impl Drop for UnreadableRetry {
    fn drop(&mut self) {
        if let Some(inner) = &self.inner {
            inner.state.lock().unwrap().shutdown = true;
            inner.cond.notify_all();
        }
    }
}

fn worker_loop(inner: &Inner) {
    let mut state = inner.state.lock().unwrap();
    loop {
        // Sleep (no polling) until something is pending.
        while state.pending.is_empty() {
            if state.shutdown {
                return;
            }
            state = inner.cond.wait(state).unwrap();
        }

        // `defer` also wakes this wait, so loop until the interval has
        // really elapsed.
        let deadline = Instant::now() + inner.interval;
        loop {
            if state.shutdown {
                return;
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            state = inner.cond.wait_timeout(state, deadline - now).unwrap().0;
        }

        let paths: Vec<PathBuf> = state.pending.keys().cloned().collect();
        drop(state);

        for path in paths {
            if inner.state.lock().unwrap().shutdown {
                return;
            }
            // A panicking retry must not kill the only worker; the path stays
            // pending and is bounded by the give-up time.
            let outcome =
                catch_unwind(AssertUnwindSafe(|| (inner.retry)(&path))).unwrap_or_else(|_| {
                    log::error!("unreadable-file retry panicked for {}", path.display());
                    RetryOutcome::StillUnreadable
                });

            let mut guard = inner.state.lock().unwrap();
            // Read the first-deferral time from the map, not a snapshot: the
            // path may have been removed (and deferred anew) meanwhile.
            let Some(&first) = guard.pending.get(&path) else {
                continue;
            };
            match outcome {
                RetryOutcome::Done | RetryOutcome::Gone => {
                    guard.pending.remove(&path);
                }
                RetryOutcome::StillUnreadable => {
                    if first.elapsed() > inner.give_up {
                        guard.pending.remove(&path);
                        log::warn!(
                            "giving up on {}: still unreadable after {:?}",
                            path.display(),
                            inner.give_up
                        );
                    }
                }
            }
        }

        state = inner.state.lock().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const TICK: Duration = Duration::from_millis(20);

    fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..200 {
            if cond() {
                return true;
            }
            thread::sleep(Duration::from_millis(25));
        }
        false
    }

    fn counting(
        outcome: impl Fn(usize) -> RetryOutcome + Send + Sync + 'static,
    ) -> (UnreadableRetry, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let retry = UnreadableRetry::with_timing(TICK, Duration::from_secs(60), move |_| {
            outcome(counter.fetch_add(1, Ordering::SeqCst) + 1)
        });
        (retry, calls)
    }

    #[test]
    fn noop_defer_does_nothing() {
        let retry = UnreadableRetry::noop();
        retry.defer(Path::new("a.mp4"));
        retry.remove(Path::new("a.mp4"));
        assert_eq!(retry.pending_count(), 0);
    }

    #[test]
    fn keeps_retrying_while_unreadable_then_drops_on_done() {
        let (retry, calls) = counting(|n| {
            if n >= 3 {
                RetryOutcome::Done
            } else {
                RetryOutcome::StillUnreadable
            }
        });

        retry.defer(Path::new("a.mp4"));

        assert!(wait_until(|| retry.pending_count() == 0));
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        // Once dropped it is not evaluated again.
        thread::sleep(TICK * 5);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn deferring_the_same_path_repeatedly_keeps_one_entry() {
        let (retry, _calls) = counting(|_| RetryOutcome::StillUnreadable);

        for _ in 0..10 {
            retry.defer(Path::new("a.mp4"));
        }

        assert_eq!(retry.pending_count(), 1);
    }

    #[test]
    fn gives_up_once_the_give_up_time_has_passed() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let retry = UnreadableRetry::with_timing(TICK, Duration::from_millis(150), move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            RetryOutcome::StillUnreadable
        });

        retry.defer(Path::new("a.mp4"));

        assert!(wait_until(|| retry.pending_count() == 0));
        let after_give_up = calls.load(Ordering::SeqCst);
        assert!(after_give_up >= 2, "it must retry before giving up");
        thread::sleep(TICK * 5);
        assert_eq!(calls.load(Ordering::SeqCst), after_give_up);
    }

    #[test]
    fn gone_drops_the_path() {
        let (retry, calls) = counting(|_| RetryOutcome::Gone);

        retry.defer(Path::new("a.mp4"));

        assert!(wait_until(|| retry.pending_count() == 0));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn remove_drops_the_path_without_evaluating_it() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let retry = UnreadableRetry::with_timing(
            Duration::from_secs(60),
            Duration::from_secs(60),
            move |_| {
                counter.fetch_add(1, Ordering::SeqCst);
                RetryOutcome::StillUnreadable
            },
        );

        retry.defer(Path::new("a.mp4"));
        assert_eq!(retry.pending_count(), 1);
        retry.remove(Path::new("a.mp4"));

        assert_eq!(retry.pending_count(), 0);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn idle_worker_does_not_evaluate_anything() {
        let (_retry, calls) = counting(|_| RetryOutcome::Done);

        thread::sleep(TICK * 10);

        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_panicking_retry_does_not_kill_the_worker() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let retry = UnreadableRetry::with_timing(TICK, Duration::from_secs(60), move |_| {
            if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                panic!("boom");
            }
            RetryOutcome::Done
        });

        retry.defer(Path::new("a.mp4"));

        assert!(wait_until(|| retry.pending_count() == 0));
        assert!(calls.load(Ordering::SeqCst) >= 2);
    }

    #[test]
    fn dropping_stops_the_worker() {
        let (retry, calls) = counting(|_| RetryOutcome::StillUnreadable);
        retry.defer(Path::new("a.mp4"));
        assert!(wait_until(|| calls.load(Ordering::SeqCst) >= 1));

        drop(retry);
        thread::sleep(TICK * 3);
        let after_drop = calls.load(Ordering::SeqCst);
        thread::sleep(TICK * 10);

        assert_eq!(calls.load(Ordering::SeqCst), after_drop);
    }
}
