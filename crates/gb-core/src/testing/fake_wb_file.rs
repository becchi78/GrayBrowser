use std::path::PathBuf;
use std::sync::Mutex;

use crate::ports::dialog::DialogError;
use crate::ports::wb_file::WbFilePicker;

pub struct FakeWbFilePicker {
    pub result: Result<Option<PathBuf>, DialogError>,
    pub call_count: Mutex<u32>,
}

impl Default for FakeWbFilePicker {
    fn default() -> Self {
        Self {
            result: Err(DialogError::Failed(
                "FakeWbFilePicker: no canned value configured for this test".into(),
            )),
            call_count: Mutex::new(0),
        }
    }
}

impl WbFilePicker for FakeWbFilePicker {
    fn pick_wb_file(&self) -> Result<Option<PathBuf>, DialogError> {
        *self.call_count.lock().unwrap() += 1;
        self.result.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_the_canned_file_and_counts_calls() {
        let picked = PathBuf::from("C:/Videos/library.wb");
        let fake = FakeWbFilePicker {
            result: Ok(Some(picked.clone())),
            ..Default::default()
        };
        assert_eq!(fake.pick_wb_file().unwrap(), Some(picked));
        assert_eq!(*fake.call_count.lock().unwrap(), 1);
    }

    #[test]
    fn none_represents_user_cancelled() {
        let fake = FakeWbFilePicker {
            result: Ok(None),
            ..Default::default()
        };
        assert_eq!(fake.pick_wb_file().unwrap(), None);
    }

    #[test]
    fn default_is_a_safe_failure() {
        let fake = FakeWbFilePicker::default();
        assert!(fake.pick_wb_file().is_err());
    }
}
