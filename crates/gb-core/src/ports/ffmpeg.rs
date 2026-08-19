//! FFmpeg/FFprobe adapter port. The trait is OS-independent; only the real
//! implementation (in `src-tauri::adapters::ffmpeg`) spawns processes.

use std::path::Path;

pub trait FfmpegAdapter: Send + Sync {
    /// Checks whether `ffmpeg` and `ffprobe` are on PATH, returning their
    /// version strings if found.
    fn check_available(&self) -> Result<FfmpegAvailability, FfmpegError>;

    /// Returns the video's duration in seconds via ffprobe, or `None` if it
    /// could not be determined (e.g. missing duration field).
    fn probe_duration(&self, video_path: &Path) -> Result<Option<f64>, FfmpegError>;

    /// Extracts a single frame at `seek_secs` into `output_tmp_path` as WebP.
    /// The caller is responsible for the `.tmp` -> final-name rename.
    fn extract_thumbnail(
        &self,
        video_path: &Path,
        output_tmp_path: &Path,
        seek_secs: f64,
        width_px: u32,
        quality: u8,
    ) -> Result<(), FfmpegError>;

    /// Probes stream/format-level metadata via ffprobe. Every field
    /// is independently `Option` -- ffprobe's output can omit any of them
    /// depending on container/codec (e.g. no audio stream), and a partial
    /// result is still `Ok(...)`, mirroring `probe_duration`'s existing
    /// "`None` is a valid terminal answer" pattern. `Err` only means ffprobe
    /// itself failed to run or its output couldn't be parsed at all.
    fn probe_metadata(&self, video_path: &Path) -> Result<VideoMetadata, FfmpegError>;

    /// Converts a legacy JPG thumbnail at `src_path` into WebP, writing to
    /// `output_tmp_path` (`.wb` legacy thumbnail migration). As with
    /// `extract_thumbnail`, this does not rename -- the caller owns the
    /// `.tmp` -> final-name atomic rename step.
    fn convert_image_to_webp(
        &self,
        src_path: &Path,
        output_tmp_path: &Path,
        quality: u8,
    ) -> Result<(), FfmpegError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegAvailability {
    pub ffmpeg_version: Option<String>,
    pub ffprobe_version: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VideoMetadata {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    /// Bits per second. Falls back to the container-level (`format.bit_rate`)
    /// figure when the video stream doesn't report its own -- common for
    /// some containers/muxers.
    pub bitrate: Option<i64>,
    pub fps: Option<f64>,
}

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum FfmpegError {
    #[error("failed to spawn process: {0}")]
    Spawn(String),
    #[error("process exited with status {status}: {stderr}")]
    NonZeroExit { status: i32, stderr: String },
    #[error("could not parse ffprobe output: {0}")]
    ParseError(String),
}
