use std::path::Path;

use crate::ports::disk_space::DiskSpace;

/// A volume with a canned amount of free space, or one that cannot be asked.
pub struct FakeDiskSpace {
    pub available: Result<u64, String>,
}

impl FakeDiskSpace {
    pub fn with_free(bytes: u64) -> Self {
        Self {
            available: Ok(bytes),
        }
    }

    pub fn failing(message: &str) -> Self {
        Self {
            available: Err(message.to_string()),
        }
    }
}

impl DiskSpace for FakeDiskSpace {
    fn available_bytes(&self, _path: &Path) -> Result<u64, String> {
        self.available.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_the_canned_answer() {
        assert_eq!(
            FakeDiskSpace::with_free(5).available_bytes(Path::new("x")),
            Ok(5)
        );
        assert!(FakeDiskSpace::failing("no")
            .available_bytes(Path::new("x"))
            .is_err());
    }
}
