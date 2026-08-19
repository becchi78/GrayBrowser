//! Fake adapter implementations for unit-testing orchestration logic without
//! touching real processes, files, or OS UI. Only compiled when the
//! `testing` Cargo feature is enabled (never in a production build).

pub mod fake_dialog;
pub mod fake_drive_type;
pub mod fake_ffmpeg;
pub mod fake_player;
pub mod fake_watcher;
pub mod fake_wb_file;
pub mod fake_wb_source;
