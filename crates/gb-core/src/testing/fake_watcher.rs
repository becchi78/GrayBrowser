use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::ports::watcher::{FileWatcher, WatchEvent, WatchHandle, WatcherError};

/// Records which folders `watch()` was called on and captures each
/// registered callback so tests can synthesize events (`emit`) without any
/// real filesystem activity -- the file-watching equivalent of
/// `FakeFfmpegAdapter`'s canned-`Result` + call log, adapted for a "start a
/// subscription, then receive events later" API shape instead of a plain
/// call/response one.
pub struct FakeFileWatcher {
    pub watch_result: Result<(), WatcherError>,
    // `pub` (not just accessed via `watched_folders()`/`emit()`) so
    // `..Default::default()` struct-update syntax works from other crates'
    // tests too (Rust requires every field to be visible at the
    // construction site for that syntax, matching `FakeFfmpegAdapter`'s
    // `pub calls` precedent).
    pub calls: Mutex<Vec<PathBuf>>,
    #[allow(clippy::type_complexity)]
    pub callbacks: Mutex<Vec<(PathBuf, Box<dyn Fn(WatchEvent) + Send + Sync>)>>,
}

impl Default for FakeFileWatcher {
    fn default() -> Self {
        Self {
            watch_result: Err(WatcherError::StartFailed {
                path: String::new(),
                message: "FakeFileWatcher: no canned watch_result configured for this test".into(),
            }),
            calls: Mutex::new(Vec::new()),
            callbacks: Mutex::new(Vec::new()),
        }
    }
}

impl FileWatcher for FakeFileWatcher {
    fn watch(
        &self,
        folder: &Path,
        on_event: Box<dyn Fn(WatchEvent) + Send + Sync>,
    ) -> Result<Box<dyn WatchHandle>, WatcherError> {
        self.calls.lock().unwrap().push(folder.to_path_buf());
        self.watch_result.clone()?;
        self.callbacks
            .lock()
            .unwrap()
            .push((folder.to_path_buf(), on_event));
        Ok(Box::new(FakeWatchHandle))
    }
}

pub struct FakeWatchHandle;

impl WatchHandle for FakeWatchHandle {
    fn stop(&mut self) {}
}

impl FakeFileWatcher {
    /// Invokes the callback registered for `folder` (from a prior successful
    /// `watch()` call) with a synthetic event.
    pub fn emit(&self, folder: &Path, event: WatchEvent) {
        for (watched_folder, callback) in self.callbacks.lock().unwrap().iter() {
            if watched_folder == folder {
                callback(event.clone());
            }
        }
    }

    pub fn watched_folders(&self) -> Vec<PathBuf> {
        self.calls.lock().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::watcher::WatchEventKind;
    use std::sync::Arc;

    #[test]
    fn default_is_a_safe_failure_for_every_method() {
        let fake = FakeFileWatcher::default();
        assert!(fake
            .watch(Path::new("C:/videos"), Box::new(|_| {}))
            .is_err());
    }

    #[test]
    fn records_the_call_and_returns_the_canned_result() {
        let fake = FakeFileWatcher {
            watch_result: Ok(()),
            ..Default::default()
        };
        let folder = Path::new("C:/videos");
        assert!(fake.watch(folder, Box::new(|_| {})).is_ok());
        assert_eq!(fake.watched_folders(), vec![folder.to_path_buf()]);
    }

    #[test]
    fn emit_invokes_the_registered_callback_for_that_folder() {
        let fake = FakeFileWatcher {
            watch_result: Ok(()),
            ..Default::default()
        };
        let folder = Path::new("C:/videos");
        let received: Arc<Mutex<Vec<WatchEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let received_clone = Arc::clone(&received);
        fake.watch(
            folder,
            Box::new(move |event| received_clone.lock().unwrap().push(event)),
        )
        .unwrap();

        let event = WatchEvent {
            kind: WatchEventKind::Created,
            path: folder.join("movie.mp4"),
        };
        fake.emit(folder, event.clone());

        assert_eq!(received.lock().unwrap().as_slice(), [event]);
    }

    #[test]
    fn emit_does_not_invoke_callbacks_registered_for_a_different_folder() {
        let fake = FakeFileWatcher {
            watch_result: Ok(()),
            ..Default::default()
        };
        let watched = Path::new("C:/videos");
        let other = Path::new("D:/other");
        let received: Arc<Mutex<Vec<WatchEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let received_clone = Arc::clone(&received);
        fake.watch(
            watched,
            Box::new(move |event| received_clone.lock().unwrap().push(event)),
        )
        .unwrap();

        fake.emit(
            other,
            WatchEvent {
                kind: WatchEventKind::Created,
                path: other.join("movie.mp4"),
            },
        );

        assert!(received.lock().unwrap().is_empty());
    }
}
