use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::ports::trash::{FileTrasher, TrashError};

/// Records calls and returns a canned result. It never touches the
/// filesystem, so a test using it cannot remove a real file. The default
/// result is a failure, so a test that forgets to configure it cannot be
/// mistaken for a successful move (same convention as `FakePlayerLauncher`).
pub struct FakeFileTrasher {
    pub result: Result<(), TrashError>,
    pub calls: Mutex<Vec<PathBuf>>,
}

impl Default for FakeFileTrasher {
    fn default() -> Self {
        Self {
            result: Err(TrashError::Failed {
                path: String::new(),
                message: "FakeFileTrasher: no canned value configured for this test".into(),
            }),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl FileTrasher for FakeFileTrasher {
    fn move_to_trash(&self, path: &Path) -> Result<(), TrashError> {
        self.calls.lock().unwrap().push(path.to_path_buf());
        self.result.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_the_call_and_returns_the_canned_result() {
        let fake = FakeFileTrasher {
            result: Ok(()),
            ..Default::default()
        };
        let p = Path::new("C:/videos/movie.mp4");
        assert!(fake.move_to_trash(p).is_ok());
        assert_eq!(fake.calls.lock().unwrap().as_slice(), [p.to_path_buf()]);
    }

    #[test]
    fn default_fails_safely_and_still_records_the_call() {
        let fake = FakeFileTrasher::default();
        assert!(fake.move_to_trash(Path::new("x.mp4")).is_err());
        assert_eq!(fake.calls.lock().unwrap().len(), 1);
    }
}
