//! Real `DirLister`: entry names straight from `read_dir`. Against a network
//! share with nothing listening this blocks for roughly 20 seconds inside the
//! OS, so it must only be called from background threads.

use std::path::Path;

use gb_core::ports::dir_lister::DirLister;

use super::long_path;

pub struct RealDirLister;

impl DirLister for RealDirLister {
    fn list_names(&self, dir: &str) -> Option<Vec<String>> {
        let entries = std::fs::read_dir(long_path::to_long_path(Path::new(dir))).ok()?;
        // A name that is not valid UTF-8 cannot be represented in the `&str`
        // paths the rest of the app uses, so it is left out rather than
        // lossily converted into a name that does not exist on disk.
        Some(
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_files_and_folders_directly_under_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.mp4"), b"x").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub").join("nested.mp4"), b"x").unwrap();

        let mut names = RealDirLister
            .list_names(&dir.path().to_string_lossy())
            .unwrap();
        names.sort();
        assert_eq!(names, vec!["a.mp4".to_string(), "sub".to_string()]);
    }

    #[test]
    fn a_missing_directory_cannot_be_listed() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone");
        assert_eq!(RealDirLister.list_names(&missing.to_string_lossy()), None);
    }
}
