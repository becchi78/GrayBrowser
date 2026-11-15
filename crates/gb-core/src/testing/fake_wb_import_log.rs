use std::sync::Mutex;

use crate::ports::wb_import_log::WbImportLog;

/// Collects everything appended. With `fail_writes` it drops the text, like an
/// adapter whose file could not be written.
#[derive(Default)]
pub struct FakeWbImportLog {
    appended: Mutex<Vec<String>>,
    fail_writes: bool,
}

impl FakeWbImportLog {
    pub fn failing() -> Self {
        Self {
            fail_writes: true,
            ..Self::default()
        }
    }

    /// Every appended chunk joined in order.
    pub fn text(&self) -> String {
        self.appended.lock().unwrap().concat()
    }

    pub fn append_count(&self) -> usize {
        self.appended.lock().unwrap().len()
    }
}

impl WbImportLog for FakeWbImportLog {
    fn append(&self, text: &str) {
        if self.fail_writes {
            return;
        }
        self.appended.lock().unwrap().push(text.to_string());
    }

    fn location(&self) -> String {
        "fake://wb-import.log".to_string()
    }
}
