//! Real `FileWatcher`: wraps the `notify` crate's `RecommendedWatcher`,
//! normalizing its raw events into `gb_core::ports::watcher::WatchEvent` at
//! this adapter boundary. `notify` is a dependency of
//! `src-tauri` only -- never `gb-core` (gb-core stays OS-independent).

use std::path::Path;

use gb_core::ports::watcher::{FileWatcher, WatchEvent, WatchEventKind, WatchHandle, WatcherError};
use notify::event::{ModifyKind, RenameMode};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::adapters::long_path;

pub struct RealFileWatcher;

impl FileWatcher for RealFileWatcher {
    fn watch(
        &self,
        folder: &Path,
        on_event: Box<dyn Fn(WatchEvent) + Send + Sync>,
    ) -> Result<Box<dyn WatchHandle>, WatcherError> {
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| match res {
                Ok(event) => {
                    for watch_event in normalize_event(event) {
                        on_event(watch_event);
                    }
                }
                Err(e) => log::warn!("file watcher error: {e}"),
            })
            .map_err(|e| WatcherError::StartFailed {
                path: folder.display().to_string(),
                message: e.to_string(),
            })?;

        // Prefixed so registering the watch itself can succeed on a
        // watch folder deeper than MAX_PATH. `notify` then echoes back
        // paths built on top of whatever root it was given, so every path
        // it hands back is stripped to plain form in normalize_event below
        // before it ever reaches a `WatchEvent` -- gb_core's port only ever
        // sees plain paths.
        watcher
            .watch(&long_path::to_long_path(folder), RecursiveMode::Recursive)
            .map_err(|e| WatcherError::StartFailed {
                path: folder.display().to_string(),
                message: e.to_string(),
            })?;

        Ok(Box::new(RealWatchHandle {
            watcher: Some(watcher),
        }))
    }
}

struct RealWatchHandle {
    // `Option` so `stop()` can explicitly drop (and thus unregister) the
    // underlying `notify` watcher without consuming `self` (the trait's
    // `stop(&mut self)` can't take ownership). Simply letting this struct --
    // and therefore the watcher -- drop at process exit achieves the same
    // effect without ever calling `stop()` explicitly (process exit tears
    // down the OS-level watch with it).
    watcher: Option<RecommendedWatcher>,
}

impl WatchHandle for RealWatchHandle {
    fn stop(&mut self) {
        self.watcher.take(); // dropping notify's watcher unregisters it
    }
}

/// Normalizes one raw `notify::Event` into zero or more neutral
/// `WatchEvent`s. `RenameMode::Both` carries both the old and new path in
/// `event.paths` (in that order, per `notify`'s documented convention) and
/// is split into a `Removed{old}` followed by a `Created{new}` -- there is
/// no dedicated "moved" `WatchEventKind`, by design: the
/// quick_hash+file_size path-follow picks this up the same way it
/// would an unrelated delete-then-create, trading a brief offline blip for
/// zero special-casing here.
fn normalize_event(event: notify::Event) -> Vec<WatchEvent> {
    match event.kind {
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => match event.paths.as_slice() {
            [old, new] => vec![
                WatchEvent {
                    kind: WatchEventKind::Removed,
                    path: long_path::strip_long_path_prefix(old),
                },
                WatchEvent {
                    kind: WatchEventKind::Created,
                    path: long_path::strip_long_path_prefix(new),
                },
            ],
            other => {
                log::warn!(
                    "rename event with unexpected path count ({}), ignoring: {other:?}",
                    other.len()
                );
                vec![]
            }
        },
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
            single(event, WatchEventKind::Removed)
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
            single_filtering_dirs(event, WatchEventKind::Created)
        }
        EventKind::Create(_) => single_filtering_dirs(event, WatchEventKind::Created),
        EventKind::Modify(_) => single_filtering_dirs(event, WatchEventKind::Modified),
        EventKind::Remove(_) => single(event, WatchEventKind::Removed),
        // Access/Any/Other: no file content/existence change to act on.
        _ => vec![],
    }
}

/// Emits one `WatchEvent` per path in the raw event, unfiltered. Used for
/// `Removed` (and rename's `From` half): the path no longer exists by the
/// time the event arrives, so there's no way to check `is_dir()` against it.
/// Directory-vs-file disambiguation for removals is left to the caller's DB
/// lookup instead -- a removed directory was never a `videos` row, so
/// looking it up there is a harmless no-op.
fn single(event: notify::Event, kind: WatchEventKind) -> Vec<WatchEvent> {
    event
        .paths
        .into_iter()
        .map(|path| WatchEvent {
            kind,
            path: long_path::strip_long_path_prefix(&path),
        })
        .collect()
}

/// Like `single`, but for events where the path is still known to exist
/// (`Created`/`Modified`) -- directories are filtered out here so callers
/// never see a directory event for these two kinds.
fn single_filtering_dirs(event: notify::Event, kind: WatchEventKind) -> Vec<WatchEvent> {
    event
        .paths
        .into_iter()
        // Re-prefixed for the is_dir() OS call itself, independent of
        // whatever form `path` already happens to be in -- to_long_path is
        // idempotent, so this is correct whether or not notify already
        // echoed the prefixed root back.
        .filter(|path| !long_path::to_long_path(path).is_dir())
        .map(|path| WatchEvent {
            kind,
            path: long_path::strip_long_path_prefix(&path),
        })
        .collect()
}
