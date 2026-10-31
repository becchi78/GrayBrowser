//! Real `PathExistence`: a `metadata` call on the long-path form.

use std::io::ErrorKind;
use std::path::Path;

use gb_core::ports::path_existence::PathExistence;

use super::long_path::to_long_path;

pub struct RealPathExistence;

impl PathExistence for RealPathExistence {
    fn exists(&self, path: &Path) -> bool {
        match std::fs::metadata(to_long_path(path)) {
            Ok(_) => true,
            Err(e) => e.kind() != ErrorKind::NotFound,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_existing_and_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.mp4");
        std::fs::write(&file, b"x").unwrap();

        assert!(RealPathExistence.exists(&file));
        assert!(RealPathExistence.exists(dir.path()));
        assert!(!RealPathExistence.exists(&dir.path().join("nope.mp4")));
        assert!(!RealPathExistence.exists(&dir.path().join("nope").join("a.mp4")));
    }
}
