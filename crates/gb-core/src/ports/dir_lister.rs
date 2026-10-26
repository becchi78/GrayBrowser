//! Directory listing port: asks the OS for the entry names directly under a
//! folder. The real implementation (in `src-tauri::adapters::dir_lister`)
//! calls `read_dir`, which against an unreachable network share can block for
//! tens of seconds -- callers must therefore run it on a background thread
//! and avoid listing the same folder repeatedly.

pub trait DirLister: Send + Sync {
    /// The names (files and folders, not full paths) directly under `dir`.
    /// `None` means the folder could not be listed right now (unreachable or
    /// nonexistent), which is deliberately not the same claim as "empty".
    fn list_names(&self, dir: &str) -> Option<Vec<String>>;
}
