//! Free-disk-space port: how many bytes can still be written on the volume
//! that holds a path. The trait is OS-independent; the real implementation
//! (in `src-tauri::adapters::disk_space`) asks Windows.

pub trait DiskSpace: Send + Sync {
    /// Bytes available to the current user on the volume holding `path`.
    /// `path` must exist (a directory, or a file inside the volume).
    fn available_bytes(&self, path: &std::path::Path) -> Result<u64, String>;
}
