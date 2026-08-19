//! Native folder-picker dialog port.

use std::path::PathBuf;

pub trait FolderPicker: Send + Sync {
    /// Returns the folders the user picked, or `None` if they cancelled.
    fn pick_folders(&self) -> Result<Option<Vec<PathBuf>>, DialogError>;
}

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum DialogError {
    #[error("dialog failed: {0}")]
    Failed(String),
}
