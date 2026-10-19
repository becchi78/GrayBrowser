use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::ports::file_rename::{FileRenamer, RenameError};

/// An in-memory set of file paths. Like the real adapter it never replaces an
/// existing file, and it compares paths case-insensitively as NTFS does.
#[derive(Default)]
pub struct FakeFileRenamer {
    files: Mutex<BTreeSet<String>>,
    /// When set, every rename fails with this error before touching `files`.
    pub forced_error: Mutex<Option<RenameError>>,
    pub calls: Mutex<Vec<(PathBuf, PathBuf)>>,
}

fn key(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/").to_lowercase()
}

impl FakeFileRenamer {
    pub fn with_files(paths: &[&str]) -> Self {
        let fake = Self::default();
        for p in paths {
            fake.files.lock().unwrap().insert(key(Path::new(p)));
        }
        fake
    }

    pub fn exists(&self, p: &Path) -> bool {
        self.files.lock().unwrap().contains(&key(p))
    }
}

impl FileRenamer for FakeFileRenamer {
    fn rename_no_overwrite(&self, from: &Path, to: &Path) -> Result<(), RenameError> {
        self.calls
            .lock()
            .unwrap()
            .push((from.to_path_buf(), to.to_path_buf()));
        if let Some(e) = self.forced_error.lock().unwrap().clone() {
            return Err(e);
        }
        let mut files = self.files.lock().unwrap();
        if !files.contains(&key(from)) {
            return Err(RenameError::SourceMissing);
        }
        if files.contains(&key(to)) {
            return Err(RenameError::TargetExists);
        }
        files.remove(&key(from));
        files.insert(key(to));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_when_the_target_is_free() {
        let fake = FakeFileRenamer::with_files(&["C:/v/a.mp4"]);
        fake.rename_no_overwrite(Path::new("C:/v/a.mp4"), Path::new("C:/v/b.mp4"))
            .unwrap();
        assert!(!fake.exists(Path::new("C:/v/a.mp4")));
        assert!(fake.exists(Path::new("C:/v/b.mp4")));
    }

    #[test]
    fn refuses_to_overwrite_case_insensitively_and_keeps_both() {
        let fake = FakeFileRenamer::with_files(&["C:/v/a.mp4", "C:/v/B.mp4"]);
        let r = fake.rename_no_overwrite(Path::new("C:/v/a.mp4"), Path::new("C:/v/b.MP4"));
        assert_eq!(r, Err(RenameError::TargetExists));
        assert!(fake.exists(Path::new("C:/v/a.mp4")));
        assert!(fake.exists(Path::new("C:/v/B.mp4")));
    }

    #[test]
    fn reports_a_missing_source() {
        let fake = FakeFileRenamer::default();
        let r = fake.rename_no_overwrite(Path::new("C:/v/a.mp4"), Path::new("C:/v/b.mp4"));
        assert_eq!(r, Err(RenameError::SourceMissing));
    }
}
