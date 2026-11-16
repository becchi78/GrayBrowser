//! Real `FileRevealer`: `tauri-plugin-opener`'s `reveal_item_in_dir`
//! (`SHOpenFolderAndSelectItems` on Windows).
//!
//! The path is passed as-is, without `long_path::to_long_path`: the plugin
//! canonicalizes it itself (`dunce::simplified` + an existence check), and the
//! shell item-ID-list APIs it uses do not take the `\\?\` form anyway.

use std::path::Path;

use gb_core::ports::file_revealer::{FileRevealer, RevealError};
use tauri_plugin_opener::OpenerExt;

use super::player::init_com_for_shell;

pub struct RealFileRevealer {
    app_handle: tauri::AppHandle,
}

impl RealFileRevealer {
    pub fn new(app_handle: tauri::AppHandle) -> Self {
        Self { app_handle }
    }
}

impl FileRevealer for RealFileRevealer {
    fn reveal(&self, path: &Path) -> Result<(), RevealError> {
        // Called from a `spawn_blocking` worker, which has no COM apartment.
        init_com_for_shell();
        self.app_handle
            .opener()
            .reveal_item_in_dir(path)
            .map_err(|e| RevealError::Reveal(e.to_string()))
    }
}
