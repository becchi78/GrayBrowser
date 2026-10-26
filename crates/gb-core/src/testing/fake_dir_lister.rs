use std::collections::HashMap;
use std::sync::Mutex;

use crate::ports::dir_lister::DirLister;

/// Folders not registered via `with_dir` cannot be listed (`None`), so a test
/// only has to name the folders that exist. Keys are matched exactly as
/// passed to `list_names`.
#[derive(Default)]
pub struct FakeDirLister {
    dirs: HashMap<String, Vec<String>>,
    calls: Mutex<Vec<String>>,
}

impl FakeDirLister {
    pub fn with_dir(mut self, dir: &str, names: &[&str]) -> Self {
        self.dirs.insert(
            dir.to_string(),
            names.iter().map(|n| n.to_string()).collect(),
        );
        self
    }

    /// Every folder passed to `list_names`, in call order.
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    pub fn call_count_for(&self, dir: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|d| d.as_str() == dir)
            .count()
    }
}

impl DirLister for FakeDirLister {
    fn list_names(&self, dir: &str) -> Option<Vec<String>> {
        self.calls.lock().unwrap().push(dir.to_string());
        self.dirs.get(dir).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_folders_list_and_others_do_not() {
        let fake = FakeDirLister::default().with_dir("D:/v", &["a.mp4"]);
        assert_eq!(fake.list_names("D:/v"), Some(vec!["a.mp4".to_string()]));
        assert_eq!(fake.list_names("D:/gone"), None);
        assert_eq!(fake.call_count_for("D:/v"), 1);
        assert_eq!(fake.calls().len(), 2);
    }
}
