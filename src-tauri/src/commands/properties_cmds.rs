//! Video properties command. Fetched lazily, one video at a time,
//! only when the properties panel opens -- these columns are deliberately
//! excluded from `VideoDto`/`list_videos` to keep the hot grid payload lean.

use serde::Serialize;
use tauri::State;

use crate::db::{queries, Db};

#[derive(Serialize)]
pub struct VideoPropertiesDto {
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub bitrate: Option<i64>,
    pub fps: Option<f64>,
    /// `None` means "not yet probed" -- the frontend must render this as a
    /// distinct pending state, not blank fields indistinguishable from a
    /// probe that failed.
    pub probed_at: Option<String>,
}

impl From<queries::VideoPropertiesRow> for VideoPropertiesDto {
    fn from(row: queries::VideoPropertiesRow) -> Self {
        Self {
            width: row.width,
            height: row.height,
            video_codec: row.video_codec,
            audio_codec: row.audio_codec,
            bitrate: row.bitrate,
            fps: row.fps,
            probed_at: row.probed_at,
        }
    }
}

#[tauri::command]
pub fn get_video_properties(
    db: State<Db>,
    video_id: String,
) -> Result<Option<VideoPropertiesDto>, String> {
    queries::get_video_properties(&db.read_pool, &video_id)
        .map(|row| row.map(VideoPropertiesDto::from))
        .map_err(|e| e.to_string())
}
