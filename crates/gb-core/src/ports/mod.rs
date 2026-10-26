//! Trait definitions for OS/process-dependent adapters. Only the trait
//! shapes live here; real implementations live in `src-tauri::adapters`.

pub mod dialog;
pub mod dir_lister;
pub mod drive_type;
pub mod ffmpeg;
pub mod file_rename;
pub mod folder_probe;
pub mod log_source;
pub mod player;
pub mod trash;
pub mod watcher;
pub mod wb_file;
pub mod wb_source;
