//! Port for asking the OS whether a path exists.

use std::path::Path;

pub trait PathExistence: Send + Sync {
    /// `false` only when the OS positively reports the path as not found.
    /// Any other failure (access denied, I/O error, timeout) returns `true`:
    /// callers use a `false` as proof of deletion, so an indeterminate answer
    /// must never look like one.
    fn exists(&self, path: &Path) -> bool;
}
