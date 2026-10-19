//! Real `FileRenamer`: `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`.
//! `std::fs::rename` is deliberately not used -- on Windows it replaces an
//! existing destination, which would destroy another file of the user's.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use gb_core::ports::file_rename::{FileRenamer, RenameError};
use windows_sys::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND,
};
use windows_sys::Win32::Storage::FileSystem::MoveFileExW;

use super::long_path::to_long_path;

pub struct RealFileRenamer;

fn to_wide(path: &Path) -> Vec<u16> {
    to_long_path(path)
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

impl FileRenamer for RealFileRenamer {
    fn rename_no_overwrite(&self, from: &Path, to: &Path) -> Result<(), RenameError> {
        let from_w = to_wide(from);
        let to_w = to_wide(to);
        // SAFETY: both buffers are valid null-terminated UTF-16 strings that
        // outlive the call; flags 0 means no replace, no copy fallback.
        let ok = unsafe { MoveFileExW(from_w.as_ptr(), to_w.as_ptr(), 0) };
        if ok != 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        match err.raw_os_error().map(|c| c as u32) {
            Some(ERROR_FILE_EXISTS | ERROR_ALREADY_EXISTS) => Err(RenameError::TargetExists),
            Some(ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) => Err(RenameError::SourceMissing),
            _ => Err(RenameError::Failed {
                message: err.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn renames_a_file_when_the_target_is_free() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("a.mp4");
        let to = dir.path().join("b.mp4");
        fs::write(&from, b"payload").unwrap();

        RealFileRenamer.rename_no_overwrite(&from, &to).unwrap();

        assert!(!from.exists());
        assert_eq!(fs::read(&to).unwrap(), b"payload");
    }

    #[test]
    fn never_overwrites_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("a.mp4");
        let to = dir.path().join("b.mp4");
        fs::write(&from, b"source").unwrap();
        fs::write(&to, b"precious").unwrap();

        let r = RealFileRenamer.rename_no_overwrite(&from, &to);

        assert_eq!(r, Err(RenameError::TargetExists));
        assert_eq!(fs::read(&to).unwrap(), b"precious");
        assert_eq!(fs::read(&from).unwrap(), b"source");
    }

    #[test]
    fn never_overwrites_when_only_the_case_of_the_target_differs() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("a.mp4");
        let existing = dir.path().join("B.mp4");
        fs::write(&from, b"source").unwrap();
        fs::write(&existing, b"precious").unwrap();

        let r = RealFileRenamer.rename_no_overwrite(&from, &dir.path().join("b.mp4"));

        assert_eq!(r, Err(RenameError::TargetExists));
        assert_eq!(fs::read(&existing).unwrap(), b"precious");
    }

    #[test]
    fn reports_a_missing_source() {
        let dir = tempfile::tempdir().unwrap();
        let r = RealFileRenamer
            .rename_no_overwrite(&dir.path().join("nope.mp4"), &dir.path().join("b.mp4"));
        assert_eq!(r, Err(RenameError::SourceMissing));
    }
}
