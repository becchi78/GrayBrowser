use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::ports::drive_type::{DriveKind, DriveTypeDetector, DriveTypeError};

/// Canned per-path results (not a single fixed result) since tests
/// exercising `reconfigure`'s Local/Network routing need different folders
/// to classify differently in the same test. Unconfigured paths fail loud
/// (same "no canned value" convention as `FakeFfmpegAdapter`).
#[derive(Default)]
pub struct FakeDriveTypeDetector {
    pub results: HashMap<PathBuf, Result<DriveKind, DriveTypeError>>,
    pub calls: Mutex<Vec<PathBuf>>,
}

impl DriveTypeDetector for FakeDriveTypeDetector {
    fn detect(&self, folder: &Path) -> Result<DriveKind, DriveTypeError> {
        self.calls.lock().unwrap().push(folder.to_path_buf());
        self.results.get(folder).cloned().unwrap_or_else(|| {
            Err(DriveTypeError::DetectionFailed {
                path: folder.display().to_string(),
                message: "FakeDriveTypeDetector: no canned result configured for this path".into(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unconfigured_path_fails_safely() {
        let fake = FakeDriveTypeDetector::default();
        assert!(fake.detect(Path::new("D:\\")).is_err());
    }

    #[test]
    fn returns_the_canned_result_and_records_the_call() {
        let mut fake = FakeDriveTypeDetector::default();
        fake.results
            .insert(PathBuf::from("D:\\"), Ok(DriveKind::Local));
        assert_eq!(fake.detect(Path::new("D:\\")), Ok(DriveKind::Local));
        assert_eq!(
            fake.calls.lock().unwrap().as_slice(),
            [PathBuf::from("D:\\")]
        );
    }

    #[test]
    fn different_paths_can_return_different_canned_results() {
        let mut fake = FakeDriveTypeDetector::default();
        fake.results
            .insert(PathBuf::from("D:\\"), Ok(DriveKind::Local));
        fake.results
            .insert(PathBuf::from("\\\\nas\\share"), Ok(DriveKind::Network));
        assert_eq!(fake.detect(Path::new("D:\\")), Ok(DriveKind::Local));
        assert_eq!(
            fake.detect(Path::new("\\\\nas\\share")),
            Ok(DriveKind::Network)
        );
    }
}
