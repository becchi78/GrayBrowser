//! Real `PlayerLauncher`: OS default file association via
//! `tauri-plugin-opener`, or a configured player exe via argument-array
//! `std::process::Command`.
//!
//! `tauri-plugin-shell`'s `open()` was considered first, but it has been
//! deprecated since 2.1.0 in favor of `tauri-plugin-opener` (confirmed in
//! that crate's source: `#[deprecated(since = "2.1.0", note = "Use
//! tauri-plugin-opener instead.")]`). `tauri-plugin-shell` isn't used for
//! anything else in this app, so it's not a dependency at all -- only
//! `tauri-plugin-opener` is.

use std::path::Path;
use std::process::Command;

use gb_core::ports::player::{LaunchError, PlayerLauncher};
use tauri_plugin_opener::OpenerExt;

pub struct RealPlayerLauncher {
    app_handle: tauri::AppHandle,
}

impl RealPlayerLauncher {
    pub fn new(app_handle: tauri::AppHandle) -> Self {
        Self { app_handle }
    }
}

impl PlayerLauncher for RealPlayerLauncher {
    fn launch(&self, video_path: &Path, override_player: Option<&Path>) -> Result<(), LaunchError> {
        match override_player {
            Some(player) => {
                Command::new(player)
                    .arg(video_path)
                    .spawn()
                    .map_err(|e| LaunchError::Spawn(e.to_string()))?;
                Ok(())
            }
            None => {
                let path_str = video_path.to_str().ok_or_else(|| {
                    LaunchError::Spawn(format!("path is not valid UTF-8: {}", video_path.display()))
                })?;
                self.app_handle
                    .opener()
                    .open_path(path_str, None::<&str>)
                    .map_err(|e| LaunchError::Spawn(e.to_string()))
            }
        }
    }
}
