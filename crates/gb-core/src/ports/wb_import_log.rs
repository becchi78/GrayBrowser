//! Port for the per-import log file.

/// Destination of one import's log. Writing must never fail an import, so the
/// methods return nothing: an implementation that cannot write reports the
/// problem on its own and carries on.
pub trait WbImportLog: Send + Sync {
    /// Appends `text` to the log; the caller supplies line breaks.
    fn append(&self, text: &str);

    /// Human-readable location (a file path), for the application log.
    fn location(&self) -> String;
}
