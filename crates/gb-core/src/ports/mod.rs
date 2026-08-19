//! Trait definitions for OS/process-dependent adapters. Only the trait
//! shapes live here; real implementations live in `src-tauri::adapters`.

pub mod dialog;
pub mod drive_type;
pub mod ffmpeg;
pub mod player;
pub mod watcher;
pub mod wb_file;
pub mod wb_source;
