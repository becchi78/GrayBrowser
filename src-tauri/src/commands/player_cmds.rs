use std::path::Path;

use gb_core::ports::player::PlayerLauncher;

use crate::adapters;

/// `override_player` is always `None` (confirmed decision: OS
/// default file association only, no player-picker UI).
fn launch_video(launcher: &impl PlayerLauncher, video_path: &Path) -> Result<(), String> {
    launcher.launch(video_path, None).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn play_video(app: tauri::AppHandle, file_path: String) -> Result<(), String> {
    let launcher = adapters::player::RealPlayerLauncher::new(app);
    match launch_video(&launcher, Path::new(&file_path)) {
        Ok(()) => {
            log::info!("launched external player for {file_path}");
            Ok(())
        }
        Err(e) => {
            log::warn!("failed to launch external player for {file_path}: {e}");
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gb_core::ports::player::LaunchError;
    use gb_core::testing::fake_player::FakePlayerLauncher;

    #[test]
    fn success_is_propagated() {
        let fake = FakePlayerLauncher {
            result: Ok(()),
            ..Default::default()
        };
        assert!(launch_video(&fake, Path::new("C:/videos/movie.mp4")).is_ok());
    }

    #[test]
    fn failure_is_propagated_as_a_string_error() {
        let fake = FakePlayerLauncher {
            result: Err(LaunchError::Spawn("no association".into())),
            ..Default::default()
        };
        assert!(launch_video(&fake, Path::new("C:/videos/movie.mp4")).is_err());
    }
}
