//! Port for reading the application log incrementally.

pub trait LogSource: Send + Sync {
    /// Reads log text the caller has not seen yet.
    ///
    /// `offset` is the `next_offset` of the previous chunk, or `None` for the
    /// first read, which returns only the last `max_bytes` bytes. Returned
    /// text always ends on a line boundary: a trailing line that is still
    /// being written is left for the next call. `reset` tells the caller to
    /// discard what it already holds (the log was replaced or shrank, or too
    /// much was written since `offset` to return it all).
    fn read_from(&self, offset: Option<u64>, max_bytes: u64) -> Result<LogChunk, LogReadError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogChunk {
    pub text: String,
    pub next_offset: u64,
    pub reset: bool,
}

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum LogReadError {
    #[error("the log file does not exist")]
    NotFound,
    #[error("failed to read the log file: {message}")]
    Failed { message: String },
}
