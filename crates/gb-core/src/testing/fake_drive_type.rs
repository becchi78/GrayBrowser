use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ports::drive_type::{DriveKind, DriveTypeDetector, DriveTypeError};

/// Canned per-path results (not a single fixed result) since tests
/// exercising `reconfigure`'s Local/Network routing need different folders
/// to classify differently in the same test. Unconfigured paths fail loud
/// (same "no canned value" convention as `FakeFfmpegAdapter`).
#[derive(Default)]
pub struct FakeDriveTypeDetector {
    pub results: HashMap<PathBuf, Result<DriveKind, DriveTypeError>>,
}

impl DriveTypeDetector for FakeDriveTypeDetector {
    fn detect(&self, folder: &Path) -> Result<DriveKind, DriveTypeError> {
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
