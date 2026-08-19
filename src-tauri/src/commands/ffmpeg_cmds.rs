use gb_core::ports::ffmpeg::FfmpegAdapter;
use serde::Serialize;

use crate::adapters;

#[derive(Serialize)]
pub struct FfmpegStatusDto {
    pub available: bool,
    pub ffmpeg_version: Option<String>,
    pub ffprobe_version: Option<String>,
}

fn ffmpeg_status_from(adapter: &impl FfmpegAdapter) -> FfmpegStatusDto {
    match adapter.check_available() {
        Ok(a) => FfmpegStatusDto {
            available: a.ffmpeg_version.is_some() && a.ffprobe_version.is_some(),
            ffmpeg_version: a.ffmpeg_version,
            ffprobe_version: a.ffprobe_version,
        },
        Err(_) => FfmpegStatusDto {
            available: false,
            ffmpeg_version: None,
            ffprobe_version: None,
        },
    }
}

#[tauri::command]
pub fn get_ffmpeg_status() -> FfmpegStatusDto {
    ffmpeg_status_from(&adapters::ffmpeg::RealFfmpegAdapter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gb_core::ports::ffmpeg::{FfmpegAvailability, FfmpegError};
    use gb_core::testing::fake_ffmpeg::FakeFfmpegAdapter;

    #[test]
    fn unavailable_when_ffmpeg_check_fails() {
        let fake = FakeFfmpegAdapter {
            availability: Err(FfmpegError::Spawn("ffmpeg not found".into())),
            ..Default::default()
        };
        let status = ffmpeg_status_from(&fake);
        assert!(!status.available);
        assert!(status.ffmpeg_version.is_none());
        assert!(status.ffprobe_version.is_none());
    }

    #[test]
    fn available_when_both_binaries_found() {
        let fake = FakeFfmpegAdapter {
            availability: Ok(FfmpegAvailability {
                ffmpeg_version: Some("ffmpeg version 7.0".into()),
                ffprobe_version: Some("ffprobe version 7.0".into()),
            }),
            ..Default::default()
        };
        let status = ffmpeg_status_from(&fake);
        assert!(status.available);
    }

    #[test]
    fn unavailable_when_only_one_binary_found() {
        let fake = FakeFfmpegAdapter {
            availability: Ok(FfmpegAvailability {
                ffmpeg_version: Some("ffmpeg version 7.0".into()),
                ffprobe_version: None,
            }),
            ..Default::default()
        };
        let status = ffmpeg_status_from(&fake);
        assert!(!status.available);
    }
}
