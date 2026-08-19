//! External player launch port.

use std::path::Path;

pub trait PlayerLauncher: Send + Sync {
    /// Launches `video_path` for playback. If `override_player` is set, that
    /// executable is spawned with the video path as an argument; otherwise
    /// the OS default file association is used.
    fn launch(&self, video_path: &Path, override_player: Option<&Path>) -> Result<(), LaunchError>;
}

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    #[error("failed to launch player: {0}")]
    Spawn(String),
}
