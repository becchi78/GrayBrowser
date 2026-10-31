use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Mutex;

use crate::ports::path_existence::PathExistence;

/// An in-memory set of existing paths, compared case-insensitively with
/// `\` and `/` treated alike, as on NTFS.
#[derive(Default)]
pub struct FakePathExistence {
    paths: Mutex<BTreeSet<String>>,
}

fn key(p: &Path) -> String {
    p.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

impl FakePathExistence {
    pub fn with_paths(paths: &[&str]) -> Self {
        let fake = Self::default();
        for p in paths {
            fake.paths.lock().unwrap().insert(key(Path::new(p)));
        }
        fake
    }
}

impl PathExistence for FakePathExistence {
    fn exists(&self, path: &Path) -> bool {
        self.paths.lock().unwrap().contains(&key(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_listed_paths_exist_ignoring_case_and_separator() {
        let fake = FakePathExistence::with_paths(&["C:/v/A.mp4", "C:/v"]);
        assert!(fake.exists(Path::new("c:\\v\\a.MP4")));
        assert!(fake.exists(Path::new("C:/v/")));
        assert!(!fake.exists(Path::new("C:/v/b.mp4")));
    }
}
