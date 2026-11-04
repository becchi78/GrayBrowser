use std::collections::HashMap;
use std::sync::Mutex;

use crate::ports::volume_label::{LabelResolution, VolumeLabelResolver};

/// Canned per-label results, matched case-insensitively like the real
/// resolver. A label with no canned result is "not connected", the state of a
/// drive that is simply absent. Every lookup is recorded so tests can assert
/// each label is asked about once.
#[derive(Default)]
pub struct FakeVolumeLabelResolver {
    results: HashMap<String, LabelResolution>,
    pub calls: Mutex<Vec<String>>,
}

impl FakeVolumeLabelResolver {
    pub fn with(mut self, label: &str, result: LabelResolution) -> Self {
        self.results.insert(label.to_lowercase(), result);
        self
    }
}

impl VolumeLabelResolver for FakeVolumeLabelResolver {
    fn resolve(&self, label: &str) -> LabelResolution {
        self.calls.lock().unwrap().push(label.to_string());
        self.results
            .get(&label.to_lowercase())
            .cloned()
            .unwrap_or(LabelResolution::NotConnected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unconfigured_label_is_not_connected() {
        let fake = FakeVolumeLabelResolver::default();
        assert_eq!(fake.resolve("X"), LabelResolution::NotConnected);
    }

    #[test]
    fn matches_case_insensitively_and_records_calls() {
        let fake = FakeVolumeLabelResolver::default()
            .with("ESD-USB", LabelResolution::Resolved("E:\\".into()));
        assert_eq!(
            fake.resolve("esd-usb"),
            LabelResolution::Resolved("E:\\".into())
        );
        assert_eq!(fake.calls.lock().unwrap().as_slice(), ["esd-usb"]);
    }
}
