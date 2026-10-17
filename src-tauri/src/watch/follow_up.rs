//! Coalesces "something was registered" signals from the watchers/pollers
//! into one deferred follow-up run (thumbnail + metadata enqueueing).
//!
//! A bulk copy fires hundreds of registrations in quick succession; running
//! the follow-up per event would re-enumerate the whole catalog each time, and
//! spawning a thread per event would pile up threads. Instead `trigger` only
//! flips a pending flag under a mutex, and a single worker thread waits for
//! the burst to settle before running the action once.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Wait this long after the last trigger before running.
pub const DEFAULT_QUIET: Duration = Duration::from_secs(2);
/// Never defer longer than this since the first un-run trigger, so a copy
/// that keeps producing events still gets periodic thumbnails/metadata.
pub const DEFAULT_MAX_WAIT: Duration = Duration::from_secs(15);

#[derive(Default)]
struct State {
    /// When the oldest un-run trigger arrived; `None` means nothing pending.
    first: Option<Instant>,
    last: Option<Instant>,
    shutdown: bool,
}

struct Inner {
    state: Mutex<State>,
    cond: Condvar,
    quiet: Duration,
    max_wait: Duration,
    action: Box<dyn Fn() + Send + Sync>,
}

pub struct FollowUp {
    inner: Option<Arc<Inner>>,
}

impl FollowUp {
    /// A `FollowUp` whose `trigger` does nothing and owns no thread; used where
    /// no follow-up is wanted (tests, `RealtimeWatchManager::default`).
    pub fn noop() -> Self {
        Self { inner: None }
    }

    pub fn new(action: impl Fn() + Send + Sync + 'static) -> Self {
        Self::with_timing(DEFAULT_QUIET, DEFAULT_MAX_WAIT, action)
    }

    /// Timings are injectable so tests can use millisecond values.
    pub fn with_timing(
        quiet: Duration,
        max_wait: Duration,
        action: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let inner = Arc::new(Inner {
            state: Mutex::new(State::default()),
            cond: Condvar::new(),
            quiet,
            max_wait,
            action: Box::new(action),
        });
        let worker = Arc::clone(&inner);
        thread::spawn(move || worker_loop(&worker));
        Self { inner: Some(inner) }
    }

    /// Returns immediately. A trigger that arrives while the action is
    /// running is kept pending and causes one more run afterwards, so a file
    /// registered mid-run is never missed.
    pub fn trigger(&self) {
        let Some(inner) = &self.inner else { return };
        let now = Instant::now();
        let mut state = inner.state.lock().unwrap();
        state.first.get_or_insert(now);
        state.last = Some(now);
        inner.cond.notify_all();
    }

    #[cfg(test)]
    pub(crate) fn has_pending(&self) -> bool {
        self.inner
            .as_ref()
            .is_some_and(|inner| inner.state.lock().unwrap().first.is_some())
    }
}

impl Drop for FollowUp {
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
        // Sleep until something is pending.
        while state.first.is_none() {
            if state.shutdown {
                return;
            }
            state = inner.cond.wait(state).unwrap();
        }

        // Wait for the burst to settle, bounded by the max wait.
        loop {
            if state.shutdown {
                return;
            }
            let (Some(first), Some(last)) = (state.first, state.last) else {
                break;
            };
            let deadline = (last + inner.quiet).min(first + inner.max_wait);
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            state = inner.cond.wait_timeout(state, deadline - now).unwrap().0;
        }

        state.first = None;
        state.last = None;
        drop(state);
        // A panicking action must not kill the only worker, or every later
        // trigger would be silently dropped.
        if catch_unwind(AssertUnwindSafe(|| (inner.action)())).is_err() {
            log::error!("follow-up action panicked");
        }
        state = inner.state.lock().unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..200 {
            if cond() {
                return true;
            }
            thread::sleep(Duration::from_millis(25));
        }
        false
    }

    #[test]
    fn noop_trigger_does_nothing() {
        let follow_up = FollowUp::noop();
        follow_up.trigger();
        assert!(!follow_up.has_pending());
    }

    #[test]
    fn a_burst_of_triggers_runs_the_action_once() {
        let runs = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&runs);
        let follow_up = FollowUp::with_timing(
            Duration::from_millis(150),
            Duration::from_secs(10),
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
            },
        );

        for _ in 0..50 {
            follow_up.trigger();
        }

        assert!(wait_until(|| runs.load(Ordering::SeqCst) >= 1));
        // Give a spurious second run the chance to show up.
        thread::sleep(Duration::from_millis(500));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn continuous_triggers_are_forced_to_run_by_the_max_wait() {
        let runs = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&runs);
        // Quiet period is longer than the gap between triggers, so only the
        // max wait can make it run.
        let follow_up = FollowUp::with_timing(
            Duration::from_secs(30),
            Duration::from_millis(200),
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
            },
        );

        let mut ran = false;
        for _ in 0..200 {
            follow_up.trigger();
            if runs.load(Ordering::SeqCst) >= 1 {
                ran = true;
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        assert!(ran, "max wait must force a run despite constant triggers");
    }

    #[test]
    fn a_trigger_during_the_action_causes_one_more_run() {
        let runs = Arc::new(AtomicUsize::new(0));
        let in_action = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(AtomicUsize::new(0));
        let (runs_c, in_c, rel_c) = (
            Arc::clone(&runs),
            Arc::clone(&in_action),
            Arc::clone(&release),
        );
        let follow_up = FollowUp::with_timing(
            Duration::from_millis(50),
            Duration::from_secs(10),
            move || {
                let n = runs_c.fetch_add(1, Ordering::SeqCst) + 1;
                if n == 1 {
                    in_c.store(1, Ordering::SeqCst);
                    // Hold the first run open until the test has re-triggered.
                    for _ in 0..400 {
                        if rel_c.load(Ordering::SeqCst) == 1 {
                            break;
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                }
            },
        );

        follow_up.trigger();
        assert!(wait_until(|| in_action.load(Ordering::SeqCst) == 1));
        follow_up.trigger();
        release.store(1, Ordering::SeqCst);

        assert!(wait_until(|| runs.load(Ordering::SeqCst) == 2));
        thread::sleep(Duration::from_millis(300));
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }
}
