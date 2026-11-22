//! OS-independent business logic for GrayBrowser.
//!
//! This crate must never depend on `tauri`, `rusqlite`, or any process-spawning
//! crate, and must never contain `#[cfg(windows)]`.

pub mod appearance;
pub mod backup;
pub mod dedup;
pub mod diagnostics;
pub mod file_delete_policy;
pub mod filename_validation;
pub mod hash;
pub mod image_size;
pub mod log_level;
pub mod log_view;
pub mod migrations;
pub mod nas_poll_interval;
pub mod path_restore;
pub mod paths;
pub mod ports;
pub mod presence;
pub mod rating;
pub mod reconcile;
pub mod rename_validation;
pub mod retry;
pub mod scan_pipeline;
pub mod search;
pub mod sort;
pub mod stderr_tail;
pub mod tag_layout;
pub mod tags;
pub mod thumbnail_paths;
pub mod thumbnail_policy;
pub mod thumbnail_progress;
pub mod update_channel;
pub mod view_state;
pub mod volume_label;
pub mod watch_folders;
pub mod wb_anonymize;
pub mod wb_import;
pub mod wb_import_log;
pub mod wb_sampling;
pub mod wb_skipped_payload;
pub mod wb_thumbnail_split;
pub mod window_state;

#[cfg(feature = "testing")]
pub mod testing;
