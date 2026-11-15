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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_the_canned_folders() {
        let picked = vec![PathBuf::from("C:/Videos")];
        let fake = FakeFolderPicker {
            result: Ok(Some(picked.clone())),
        };
        assert_eq!(fake.pick_folders().unwrap(), Some(picked));
    }

    #[test]
    fn none_represents_user_cancelled() {
        let fake = FakeFolderPicker { result: Ok(None) };
        assert_eq!(fake.pick_folders().unwrap(), None);
    }
}
