//! "Show this file in the OS file manager" port.

use std::path::Path;

pub trait FileRevealer: Send + Sync {
    /// Opens the file manager on the folder containing `path` with the file
    /// selected. Fails if the file does not exist or the shell call fails.
    fn reveal(&self, path: &Path) -> Result<(), RevealError>;
}

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum RevealError {
    #[error("failed to reveal file in explorer: {0}")]
    Reveal(String),
}
