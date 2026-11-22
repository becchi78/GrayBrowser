//! Real `DiskSpace`: `GetDiskFreeSpaceExW`, which reports the space available
//! to the calling user (honouring quotas), not the volume's raw free space.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use gb_core::ports::disk_space::DiskSpace;
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

pub struct RealDiskSpace;

impl DiskSpace for RealDiskSpace {
    fn available_bytes(&self, path: &Path) -> Result<u64, String> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut available = 0u64;
        // SAFETY: `wide` is NUL-terminated and outlives the call; `available`
        // is a valid out-pointer, and the two optional outputs are null.
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(available)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_real_directory_reports_some_space() {
        let dir = tempfile::tempdir().unwrap();
        assert!(RealDiskSpace.available_bytes(dir.path()).unwrap() > 0);
    }

    #[test]
    fn a_missing_path_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(RealDiskSpace
            .available_bytes(&dir.path().join("nope").join("deeper"))
            .is_err());
    }
}
