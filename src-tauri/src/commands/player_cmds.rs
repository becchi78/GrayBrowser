use std::path::Path;

use gb_core::ports::player::PlayerLauncher;

use crate::adapters;

/// `override_player` is always `None` (confirmed decision: OS
/// default file association only, no player-picker UI).
///
/// The existence check runs first so an unreachable path (e.g. a powered-off
/// NAS) never reaches `ShellExecuteExW`: that call would block for ~20s and
/// then pop up a Windows "network error" dialog the app cannot suppress.
fn launch_video(launcher: &impl PlayerLauncher, video_path: &Path) -> Result<(), String> {
    std::fs::metadata(video_path).map_err(|e| e.to_string())?;
    launcher.launch(video_path, None).map_err(|e| e.to_string())
}

/// Runs on a blocking worker so a slow shell launch can never freeze the UI
/// thread (a sync command would execute on the main thread).
#[tauri::command]
pub async fn play_video(app: tauri::AppHandle, file_path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
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
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use gb_core::ports::player::LaunchError;
    use gb_core::testing::fake_player::FakePlayerLauncher;

    fn existing_file() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("movie.mp4");
        std::fs::write(&path, b"x").unwrap();
        (dir, path)
    }

    #[test]
    fn success_is_propagated_and_launch_is_called() {
        let (_dir, path) = existing_file();
        let fake = FakePlayerLauncher {
            result: Ok(()),
            ..Default::default()
        };
        assert!(launch_video(&fake, &path).is_ok());
        let calls = fake.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].video_path, path);
        assert_eq!(calls[0].override_player, None);
    }

    #[test]
    fn failure_is_propagated_as_a_string_error() {
        let (_dir, path) = existing_file();
        let fake = FakePlayerLauncher {
            result: Err(LaunchError::Spawn("no association".into())),
            ..Default::default()
        };
        let err = launch_video(&fake, &path).unwrap_err();
        assert!(err.contains("no association"));
    }

    #[test]
    fn missing_path_fails_without_launching() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.mp4");
        let fake = FakePlayerLauncher {
            result: Ok(()),
            ..Default::default()
        };
        let err = launch_video(&fake, &path).unwrap_err();
        assert!(!err.is_empty());
        assert!(fake.calls.lock().unwrap().is_empty());
    }
}
