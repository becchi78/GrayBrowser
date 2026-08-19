//! Real `DriveTypeDetector`: calls `GetDriveTypeW` via `windows-sys` (a thin
//! raw-FFI binding -- no COM involved for this single call, so the heavier
//! `windows` crate isn't needed). `windows-sys` is a dependency of
//! `src-tauri` only -- never `gb-core` (gb-core stays OS-independent).

use std::path::Path;

use gb_core::ports::drive_type::{DriveKind, DriveTypeDetector, DriveTypeError};
use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
// The DRIVE_* result constants live in WindowsProgramming, not
// Storage::FileSystem (confirmed against the windows-sys 0.59 source) even
// though the function that returns them is in FileSystem.
use windows_sys::Win32::System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOTE, DRIVE_REMOVABLE};

pub struct RealDriveTypeDetector;

impl DriveTypeDetector for RealDriveTypeDetector {
    fn detect(&self, folder: &Path) -> Result<DriveKind, DriveTypeError> {
        let folder_str = folder.to_string_lossy();
        let root = gb_core::paths::extract_drive_root(&folder_str).ok_or_else(|| {
            DriveTypeError::DetectionFailed {
                path: folder_str.to_string(),
                message: "could not determine a drive root for this path".to_string(),
            }
        })?;

        let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();

        // SAFETY: `wide` is a valid null-terminated UTF-16 string, kept
        // alive for the duration of this call. `GetDriveTypeW` reads it
        // synchronously and does not retain the pointer afterward.
        let drive_type = unsafe { GetDriveTypeW(wide.as_ptr()) };

        Ok(match drive_type {
            DRIVE_FIXED => DriveKind::Local,
            DRIVE_REMOTE => DriveKind::Network,
            DRIVE_REMOVABLE => DriveKind::Removable,
            _ => DriveKind::Unknown,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn the_system_drive_is_local() {
        let detector = RealDriveTypeDetector;
        assert_eq!(
            detector.detect(Path::new("C:\\")).unwrap(),
            DriveKind::Local
        );
    }
}
