//! Native single-file-picker dialog port, for selecting the legacy `.wb`
//! (WhiteBrowser SQLite database) file to import. Separate from
//! `ports::dialog::FolderPicker` because a `.wb` import picks exactly one
//! *file*, not one-or-more *folders* -- a distinct
//! native dialog shape (`blocking_pick_file` vs. `blocking_pick_folders`),
//! so it gets its own trait rather than overloading `FolderPicker`.
//!
//! Reuses `ports::dialog::DialogError` rather than defining a parallel
//! error type: both traits wrap the same underlying failure mode (the
//! native OS dialog interaction itself failing), so a second, differently
//! named error enum with identical semantics would just be duplication.

use std::path::PathBuf;

use super::dialog::DialogError;

pub trait WbFilePicker: Send + Sync {
    /// Returns the `.wb` file the user picked, or `None` if they cancelled.
    fn pick_wb_file(&self) -> Result<Option<PathBuf>, DialogError>;
}
