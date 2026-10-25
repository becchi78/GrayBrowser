//! Folder reachability port: asks the OS whether a registered watch folder
//! can be listed right now. The real implementation (in
//! `src-tauri::adapters::folder_probe`) calls `read_dir`, which against an
//! unreachable network share can block for tens of seconds -- callers must
//! therefore run it on a background thread, never on the UI/setup thread.

pub trait FolderProbe: Send + Sync {
    /// `true` if `folder` could be opened for listing. `false` means "not
    /// reachable right now", which is deliberately not the same claim as
    /// "its files were deleted".
    fn is_reachable(&self, folder: &str) -> bool;
}
