//! Real `FileTrasher`: moves a file to the Windows recycle bin through the
//! `trash` crate (`IFileOperation`), which handles long paths and COM
//! lifetime. `trash::delete` never falls back to a permanent delete; on
//! failure it returns an error and leaves the file in place.

use std::path::Path;

use gb_core::ports::trash::{FileTrasher, TrashError};

pub struct RealFileTrasher;

impl FileTrasher for RealFileTrasher {
    fn move_to_trash(&self, path: &Path) -> Result<(), TrashError> {
        trash::delete(path).map_err(|e| TrashError::Failed {
            path: path.display().to_string(),
            message: e.to_string(),
        })
    }
}
