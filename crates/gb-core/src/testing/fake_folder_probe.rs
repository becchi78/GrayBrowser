use std::collections::HashSet;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use crate::ports::folder_probe::FolderProbe;

/// Folders are reachable unless marked otherwise, so a test only has to name
/// the ones that are down. `set_reachable` can flip a folder mid-test to
/// model an outage starting or ending. `delay` makes each probe take
/// measurable time, which lets a test overlap concurrent callers.
#[derive(Default)]
pub struct FakeFolderProbe {
    unreachable: Mutex<HashSet<String>>,
    calls: Mutex<Vec<String>>,
    delay: Duration,
}

impl FakeFolderProbe {
    pub fn with_unreachable(folders: &[&str]) -> Self {
        Self {
            unreachable: Mutex::new(folders.iter().map(|f| f.to_string()).collect()),
            ..Self::default()
        }
    }

    /// Makes every probe take `delay`.
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn set_reachable(&self, folder: &str, reachable: bool) {
        let mut down = self.unreachable.lock().unwrap();
        if reachable {
            down.remove(folder);
        } else {
            down.insert(folder.to_string());
        }
    }

    /// Every folder passed to `is_reachable`, in call order.
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    pub fn call_count_for(&self, folder: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|f| f.as_str() == folder)
            .count()
    }
}

impl FolderProbe for FakeFolderProbe {
    fn is_reachable(&self, folder: &str) -> bool {
        self.calls.lock().unwrap().push(folder.to_string());
        if !self.delay.is_zero() {
            thread::sleep(self.delay);
        }
        !self.unreachable.lock().unwrap().contains(folder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_are_reachable_by_default_and_calls_are_recorded() {
        let fake = FakeFolderProbe::default();
        assert!(fake.is_reachable("C:/a"));
        assert_eq!(fake.calls(), vec!["C:/a".to_string()]);
    }

    #[test]
    fn marked_folders_are_unreachable_until_flipped_back() {
        let fake = FakeFolderProbe::with_unreachable(&["W:/v"]);
        assert!(!fake.is_reachable("W:/v"));
        assert!(fake.is_reachable("C:/a"));
        fake.set_reachable("W:/v", true);
        assert!(fake.is_reachable("W:/v"));
        assert_eq!(fake.call_count_for("W:/v"), 2);
    }
}
