//! Real `FolderProbe`: succeeds iff the folder can be opened for listing,
//! the same test the NAS diff scan has always used to decide whether a root
//! is reachable. Against a network share with nothing listening this blocks
//! for roughly 20 seconds inside the OS, so it must only be called from
//! background threads.

use std::path::Path;

use gb_core::ports::folder_probe::FolderProbe;

use super::long_path;

pub struct RealFolderProbe;

impl FolderProbe for RealFolderProbe {
    fn is_reachable(&self, folder: &str) -> bool {
        std::fs::read_dir(long_path::to_long_path(Path::new(folder))).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_existing_directory_is_reachable() {
        let dir = tempfile::tempdir().unwrap();
        assert!(RealFolderProbe.is_reachable(&dir.path().to_string_lossy()));
    }

    #[test]
    fn a_missing_directory_is_not_reachable() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone");
        assert!(!RealFolderProbe.is_reachable(&missing.to_string_lossy()));
    }
}
