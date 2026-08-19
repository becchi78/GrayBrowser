//! Local realtime file-watching port. Unlike the other adapters in this
//! module, `watch()` doesn't return a single result --
//! it starts a background subscription that keeps invoking `on_event` until
//! the returned handle is stopped or dropped. The real implementation
//! (`src-tauri::adapters::watcher`) wraps the `notify` crate and normalizes
//! its raw events into `WatchEvent` at the adapter boundary, so nothing OS-
//! specific ever crosses into this trait's shape.

use std::path::{Path, PathBuf};

pub trait FileWatcher: Send + Sync {
    /// Starts watching `folder` recursively. `on_event` is invoked (from a
    /// background thread) for each normalized event until the returned
    /// handle is stopped or dropped.
    fn watch(
        &self,
        folder: &Path,
        on_event: Box<dyn Fn(WatchEvent) + Send + Sync>,
    ) -> Result<Box<dyn WatchHandle>, WatcherError>;
}

pub trait WatchHandle: Send {
    fn stop(&mut self);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchEvent {
    pub kind: WatchEventKind,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchEventKind {
    Created,
    Modified,
    Removed,
}

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum WatcherError {
    #[error("failed to start watching {path}: {message}")]
    StartFailed { path: String, message: String },
}
