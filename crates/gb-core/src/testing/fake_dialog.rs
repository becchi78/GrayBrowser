use std::path::PathBuf;

use crate::ports::dialog::{DialogError, FolderPicker};

pub struct FakeFolderPicker {
    pub result: Result<Option<Vec<PathBuf>>, DialogError>,
}

impl Default for FakeFolderPicker {
    fn default() -> Self {
        Self {
            result: Err(DialogError::Failed(
                "FakeFolderPicker: no canned value configured for this test".into(),
            )),
        }
    }
}

impl FolderPicker for FakeFolderPicker {
    fn pick_folders(&self) -> Result<Option<Vec<PathBuf>>, DialogError> {
        self.result.clone()
    }
}
