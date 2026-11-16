use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::ports::file_revealer::{FileRevealer, RevealError};

pub struct FakeFileRevealer {
    pub result: Result<(), RevealError>,
    pub calls: Mutex<Vec<PathBuf>>,
}

impl Default for FakeFileRevealer {
    fn default() -> Self {
        Self {
            result: Err(RevealError::Reveal(
                "FakeFileRevealer: no canned value configured for this test".into(),
            )),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl FileRevealer for FakeFileRevealer {
    fn reveal(&self, path: &Path) -> Result<(), RevealError> {
        self.calls.lock().unwrap().push(path.to_path_buf());
        self.result.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_fails_safely() {
        let fake = FakeFileRevealer::default();
        assert!(fake.reveal(Path::new("x.mp4")).is_err());
    }
}
