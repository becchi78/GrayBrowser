//! Drive-type detection port: decides whether a registered watch folder is a
//! local/removable drive (realtime `notify` watching) or a network drive
//! (startup diff-scan + polling). The trait is OS-independent; only the real
//! implementation (in `src-tauri::adapters::drive_type`) calls
//! `GetDriveTypeW`.

use std::path::Path;

pub trait DriveTypeDetector: Send + Sync {
    fn detect(&self, folder: &Path) -> Result<DriveKind, DriveTypeError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveKind {
    Local,
    Network,
    Removable,
    Unknown,
}

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum DriveTypeError {
    #[error("failed to determine drive type for {path}: {message}")]
    DetectionFailed { path: String, message: String },
}
