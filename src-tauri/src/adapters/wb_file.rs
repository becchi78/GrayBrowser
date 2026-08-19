//! Real `WbFilePicker`: native single-file-selection dialog via
//! `tauri-plugin-dialog`, filtered to `.wb` files.

use std::path::PathBuf;

use gb_core::ports::dialog::DialogError;
use gb_core::ports::wb_file::WbFilePicker;
use tauri_plugin_dialog::DialogExt;

pub struct RealWbFilePicker {
    app_handle: tauri::AppHandle,
}

impl RealWbFilePicker {
    pub fn new(app_handle: tauri::AppHandle) -> Self {
        Self { app_handle }
    }
}

impl WbFilePicker for RealWbFilePicker {
    fn pick_wb_file(&self) -> Result<Option<PathBuf>, DialogError> {
        let picked = self
            .app_handle
            .dialog()
            .file()
            .add_filter("WhiteBrowser database", &["wb"])
            .blocking_pick_file();
        match picked {
            None => Ok(None),
            Some(path) => path
                .into_path()
                .map(Some)
                .map_err(|e| DialogError::Failed(e.to_string())),
        }
    }
}
