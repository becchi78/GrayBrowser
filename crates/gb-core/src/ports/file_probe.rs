//! Port for checking that a file can be stat-ed, keeping "not found" apart
//! from every other failure.

use std::path::Path;

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum FileCheckError {
    /// The OS positively reported the path as not found. This is also what a
    /// share that cannot be reached reports, so on its own it is no proof
    /// that the file is gone.
    #[error("{0}")]
    NotFound(String),
    /// Access denied, I/O error and the like: nothing can be concluded.
    #[error("{0}")]
    Other(String),
}

pub trait FileProbe: Send + Sync {
    fn check(&self, path: &Path) -> Result<(), FileCheckError>;
}
