//! Real `FileProbe`: a `metadata` call on the long-path form.

use std::io::ErrorKind;
use std::path::Path;

use gb_core::ports::file_probe::{FileCheckError, FileProbe};

use super::long_path::to_long_path;

pub struct RealFileProbe;

impl FileProbe for RealFileProbe {
    fn check(&self, path: &Path) -> Result<(), FileCheckError> {
        std::fs::metadata(to_long_path(path))
            .map(|_| ())
            .map_err(|e| match e.kind() {
                ErrorKind::NotFound => FileCheckError::NotFound(e.to_string()),
                _ => FileCheckError::Other(e.to_string()),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_present_from_missing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.mp4");
        std::fs::write(&file, b"x").unwrap();

        assert!(RealFileProbe.check(&file).is_ok());
        assert!(matches!(
            RealFileProbe.check(&dir.path().join("nope.mp4")),
            Err(FileCheckError::NotFound(_))
        ));
    }
}
