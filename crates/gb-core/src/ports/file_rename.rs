//! Port for renaming a file without ever replacing an existing one.

use std::path::Path;

pub trait FileRenamer: Send + Sync {
    /// Renames `from` to `to`. If something already exists at `to` this must
    /// return `TargetExists` and leave both files untouched: the OS-level
    /// rename of some platforms replaces silently, and losing a user's file
    /// that way is not recoverable.
    fn rename_no_overwrite(&self, from: &Path, to: &Path) -> Result<(), RenameError>;
}

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum RenameError {
    #[error("a file already exists at the target path")]
    TargetExists,
    #[error("the source file does not exist")]
    SourceMissing,
    #[error("rename failed: {message}")]
    Failed { message: String },
}
