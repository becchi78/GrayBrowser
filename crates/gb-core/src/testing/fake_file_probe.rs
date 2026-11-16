use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use crate::ports::file_probe::{FileCheckError, FileProbe};

/// Every path is present unless given an error, so a test only names the
/// paths that fail. Paths are compared as written.
#[derive(Default)]
pub struct FakeFileProbe {
    errors: Mutex<HashMap<String, FileCheckError>>,
}

impl FakeFileProbe {
    pub fn with_error(path: &str, error: FileCheckError) -> Self {
        let fake = Self::default();
        fake.errors.lock().unwrap().insert(path.to_string(), error);
        fake
    }
}

impl FileProbe for FakeFileProbe {
    fn check(&self, path: &Path) -> Result<(), FileCheckError> {
        match self
            .errors
            .lock()
            .unwrap()
            .get(path.to_string_lossy().as_ref())
        {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_listed_paths_fail() {
        let fake = FakeFileProbe::with_error("C:/a.mp4", FileCheckError::NotFound("gone".into()));
        assert_eq!(
            fake.check(Path::new("C:/a.mp4")),
            Err(FileCheckError::NotFound("gone".into()))
        );
        assert!(fake.check(Path::new("C:/b.mp4")).is_ok());
    }
}
