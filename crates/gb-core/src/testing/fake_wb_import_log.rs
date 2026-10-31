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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_in_order_and_can_be_made_to_drop_writes() {
        let fake = FakeWbImportLog::default();
        fake.append("a\n");
        fake.append("b\n");
        assert_eq!(fake.text(), "a\nb\n");
        assert_eq!(fake.append_count(), 2);

        let failing = FakeWbImportLog::failing();
        failing.append("a\n");
        assert_eq!(failing.text(), "");
    }
}
