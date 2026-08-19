use std::path::PathBuf;
use std::sync::Mutex;

use crate::ports::dialog::{DialogError, FolderPicker};

pub struct FakeFolderPicker {
    pub result: Result<Option<Vec<PathBuf>>, DialogError>,
    pub call_count: Mutex<u32>,
}

impl Default for FakeFolderPicker {
    fn default() -> Self {
        Self {
            result: Err(DialogError::Failed(
                "FakeFolderPicker: no canned value configured for this test".into(),
            )),
            call_count: Mutex::new(0),
        }
    }
}

impl FolderPicker for FakeFolderPicker {
    fn pick_folders(&self) -> Result<Option<Vec<PathBuf>>, DialogError> {
        *self.call_count.lock().unwrap() += 1;
        self.result.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_the_canned_folders_and_counts_calls() {
        let picked = vec![PathBuf::from("C:/Videos")];
        let fake = FakeFolderPicker {
            result: Ok(Some(picked.clone())),
            ..Default::default()
        };
        assert_eq!(fake.pick_folders().unwrap(), Some(picked));
        assert_eq!(*fake.call_count.lock().unwrap(), 1);
    }

    #[test]
    fn none_represents_user_cancelled() {
        let fake = FakeFolderPicker {
            result: Ok(None),
            ..Default::default()
        };
        assert_eq!(fake.pick_folders().unwrap(), None);
    }
}
