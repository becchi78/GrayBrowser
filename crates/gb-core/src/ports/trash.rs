//! Port for moving a file to the OS recycle bin.

use std::path::Path;

pub trait FileTrasher: Send + Sync {
    /// Moves `path` to the OS recycle bin. Must never permanently delete: an
    /// implementation that cannot recycle the file has to return an error
    /// instead of falling back to a permanent removal.
    fn move_to_trash(&self, path: &Path) -> Result<(), TrashError>;
}

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum TrashError {
    #[error("failed to move {path} to the recycle bin: {message}")]
    Failed { path: String, message: String },
}
