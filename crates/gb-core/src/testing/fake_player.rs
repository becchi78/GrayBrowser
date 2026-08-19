use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::ports::player::{LaunchError, PlayerLauncher};

#[derive(Debug, Clone, PartialEq)]
pub struct FakeLaunchCall {
    pub video_path: PathBuf,
    pub override_player: Option<PathBuf>,
}

pub struct FakePlayerLauncher {
    pub result: Result<(), LaunchError>,
    pub calls: Mutex<Vec<FakeLaunchCall>>,
}

impl Default for FakePlayerLauncher {
    fn default() -> Self {
        Self {
            result: Err(LaunchError::Spawn(
                "FakePlayerLauncher: no canned value configured for this test".into(),
            )),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl PlayerLauncher for FakePlayerLauncher {
    fn launch(&self, video_path: &Path, override_player: Option<&Path>) -> Result<(), LaunchError> {
        self.calls.lock().unwrap().push(FakeLaunchCall {
            video_path: video_path.to_path_buf(),
            override_player: override_player.map(|p| p.to_path_buf()),
        });
        self.result.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_the_call_and_returns_the_canned_result() {
        let fake = FakePlayerLauncher {
            result: Ok(()),
            ..Default::default()
        };
        let video = Path::new("C:/videos/movie.mp4");
        assert!(fake.launch(video, None).is_ok());
        assert_eq!(
            fake.calls.lock().unwrap().as_slice(),
            [FakeLaunchCall {
                video_path: video.to_path_buf(),
                override_player: None
            }]
        );
    }

    #[test]
    fn default_fails_safely() {
        let fake = FakePlayerLauncher::default();
        assert!(fake.launch(Path::new("x.mp4"), None).is_err());
    }
}
