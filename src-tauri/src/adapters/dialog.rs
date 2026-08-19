//! Real `FolderPicker`: native folder-selection dialog via
//! `tauri-plugin-dialog`.

use std::path::PathBuf;

use gb_core::ports::dialog::{DialogError, FolderPicker};
use tauri_plugin_dialog::DialogExt;

pub struct RealFolderPicker {
    app_handle: tauri::AppHandle,
}

impl RealFolderPicker {
    pub fn new(app_handle: tauri::AppHandle) -> Self {
        Self { app_handle }
    }
}

impl FolderPicker for RealFolderPicker {
    fn pick_folders(&self) -> Result<Option<Vec<PathBuf>>, DialogError> {
        let picked = self.app_handle.dialog().file().blocking_pick_folders();
        match picked {
            None => Ok(None),
            Some(paths) => {
                let paths = paths
                    .into_iter()
                    .map(|p| {
                        p.into_path()
                            .map_err(|e| DialogError::Failed(e.to_string()))
                    })
                    .collect::<Result<Vec<PathBuf>, DialogError>>()?;
                Ok(Some(paths))
            }
        }
    }
}
