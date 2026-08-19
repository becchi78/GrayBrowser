use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::ports::ffmpeg::{FfmpegAdapter, FfmpegAvailability, FfmpegError, VideoMetadata};

/// Closure type backing `FakeFfmpegAdapter::convert_result`, factored out to
/// satisfy `clippy::type_complexity`.
type ConvertResultFn = Box<dyn Fn(&Path) -> Result<(), FfmpegError> + Send + Sync>;

#[derive(Debug, Clone, PartialEq)]
pub enum FakeCall {
    CheckAvailable,
    ProbeDuration(PathBuf),
    ExtractThumbnail {
        video_path: PathBuf,
        output_tmp_path: PathBuf,
        seek_secs: f64,
        width_px: u32,
        quality: u8,
    },
    ProbeMetadata(PathBuf),
    ConvertImageToWebp {
        src_path: PathBuf,
        output_tmp_path: PathBuf,
        quality: u8,
    },
}

pub struct FakeFfmpegAdapter {
    pub availability: Result<FfmpegAvailability, FfmpegError>,
    pub duration: Result<Option<f64>, FfmpegError>,
    /// Decides the result of `extract_thumbnail` from the `seek_secs` it was
    /// called with -- a closure rather than a fixed `Result` so tests can
    /// model "fails at the computed position, succeeds at the 0s fallback"
    /// (the runtime-retry path).
    pub extract_result: Box<dyn Fn(f64) -> Result<(), FfmpegError> + Send + Sync>,
    pub metadata: Result<VideoMetadata, FfmpegError>,
    /// Decides the result of `convert_image_to_webp` from the `src_path` it
    /// was called with -- a closure rather than a fixed `Result`, mirroring
    /// `extract_result`, so tests can model per-file success/failure (e.g.
    /// "this one legacy JPG is corrupt, the rest convert fine").
    pub convert_result: ConvertResultFn,
    pub calls: Mutex<Vec<FakeCall>>,
}

impl Default for FakeFfmpegAdapter {
    fn default() -> Self {
        let not_configured = || {
            FfmpegError::Spawn("FakeFfmpegAdapter: no canned value configured for this test".into())
        };
        Self {
            availability: Err(not_configured()),
            duration: Err(not_configured()),
            extract_result: Box::new(move |_seek_secs| {
                Err(FfmpegError::Spawn(
                    "FakeFfmpegAdapter: no canned extract_result configured for this test".into(),
                ))
            }),
            metadata: Err(not_configured()),
            convert_result: Box::new(move |_src_path| {
                Err(FfmpegError::Spawn(
                    "FakeFfmpegAdapter: no canned convert_result configured for this test".into(),
                ))
            }),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl FfmpegAdapter for FakeFfmpegAdapter {
    fn check_available(&self) -> Result<FfmpegAvailability, FfmpegError> {
        self.calls.lock().unwrap().push(FakeCall::CheckAvailable);
        self.availability.clone()
    }

    fn probe_duration(&self, video_path: &Path) -> Result<Option<f64>, FfmpegError> {
        self.calls
            .lock()
            .unwrap()
            .push(FakeCall::ProbeDuration(video_path.to_path_buf()));
        self.duration.clone()
    }

    fn extract_thumbnail(
        &self,
        video_path: &Path,
        output_tmp_path: &Path,
        seek_secs: f64,
        width_px: u32,
        quality: u8,
    ) -> Result<(), FfmpegError> {
        self.calls.lock().unwrap().push(FakeCall::ExtractThumbnail {
            video_path: video_path.to_path_buf(),
            output_tmp_path: output_tmp_path.to_path_buf(),
            seek_secs,
            width_px,
            quality,
        });
        let result = (self.extract_result)(seek_secs);
        if result.is_ok() {
            // Simulate ffmpeg having written the output file, so callers
            // testing the atomic tmp -> final rename step have a real file
            // to rename (a fake that returns Ok without producing a file
            // would make that step untestable).
            let _ = std::fs::write(output_tmp_path, b"fake webp bytes");
        }
        result
    }

    fn probe_metadata(&self, video_path: &Path) -> Result<VideoMetadata, FfmpegError> {
        self.calls
            .lock()
            .unwrap()
            .push(FakeCall::ProbeMetadata(video_path.to_path_buf()));
        self.metadata.clone()
    }

    fn convert_image_to_webp(
        &self,
        src_path: &Path,
        output_tmp_path: &Path,
        quality: u8,
    ) -> Result<(), FfmpegError> {
        self.calls
            .lock()
            .unwrap()
            .push(FakeCall::ConvertImageToWebp {
                src_path: src_path.to_path_buf(),
                output_tmp_path: output_tmp_path.to_path_buf(),
                quality,
            });
        let result = (self.convert_result)(src_path);
        if result.is_ok() {
            // Simulate ffmpeg having written the output file, so callers
            // testing the atomic tmp -> final rename step have a real file
            // to rename (see extract_thumbnail's identical rationale above).
            let _ = std::fs::write(output_tmp_path, b"fake webp bytes");
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_the_canned_availability_and_records_the_call() {
        let fake = FakeFfmpegAdapter {
            availability: Ok(FfmpegAvailability {
                ffmpeg_version: Some("7.0".into()),
                ffprobe_version: Some("7.0".into()),
            }),
            ..Default::default()
        };

        let result = fake.check_available().unwrap();
        assert_eq!(result.ffmpeg_version.as_deref(), Some("7.0"));
        assert_eq!(
            fake.calls.lock().unwrap().as_slice(),
            [FakeCall::CheckAvailable]
        );
    }

    #[test]
    fn default_is_a_safe_failure_for_every_method() {
        let fake = FakeFfmpegAdapter::default();
        assert!(fake.check_available().is_err());
        assert!(fake.probe_duration(Path::new("x.mp4")).is_err());
        assert!(fake
            .extract_thumbnail(Path::new("x.mp4"), Path::new("x.webp.tmp"), 1.0, 320, 50)
            .is_err());
        assert!(fake.probe_metadata(Path::new("x.mp4")).is_err());
        assert!(fake
            .convert_image_to_webp(Path::new("x.jpg"), Path::new("x.webp.tmp"), 50)
            .is_err());
    }

    #[test]
    fn records_probe_duration_call_with_the_given_path() {
        let fake = FakeFfmpegAdapter {
            duration: Ok(Some(42.0)),
            ..Default::default()
        };
        let path = Path::new("C:/videos/movie.mp4");
        assert_eq!(fake.probe_duration(path).unwrap(), Some(42.0));
        assert_eq!(
            fake.calls.lock().unwrap().as_slice(),
            [FakeCall::ProbeDuration(path.to_path_buf())]
        );
    }

    #[test]
    fn extract_result_can_branch_on_the_seek_position() {
        // Simulates "fails at the computed position, succeeds at the 0s fallback".
        let fake = FakeFfmpegAdapter {
            extract_result: Box::new(|seek_secs| {
                if seek_secs == 0.0 {
                    Ok(())
                } else {
                    Err(FfmpegError::NonZeroExit {
                        status: 1,
                        stderr: "seek out of range".into(),
                    })
                }
            }),
            ..Default::default()
        };
        let tmp = std::env::temp_dir().join(format!(
            "gb-fake-ffmpeg-test-{}.webp.tmp",
            std::process::id()
        ));

        assert!(fake
            .extract_thumbnail(Path::new("x.mp4"), &tmp, 12.0, 320, 55)
            .is_err());
        assert!(fake
            .extract_thumbnail(Path::new("x.mp4"), &tmp, 0.0, 320, 55)
            .is_ok());
        assert!(
            tmp.exists(),
            "a successful extract_thumbnail should write the output file"
        );

        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn convert_result_can_branch_on_the_src_path_and_records_the_call() {
        // Simulates "this one legacy JPG is corrupt, the rest convert fine".
        let fake = FakeFfmpegAdapter {
            convert_result: Box::new(|src_path| {
                if src_path == Path::new("corrupt.jpg") {
                    Err(FfmpegError::NonZeroExit {
                        status: 1,
                        stderr: "invalid JPEG".into(),
                    })
                } else {
                    Ok(())
                }
            }),
            ..Default::default()
        };
        let tmp = std::env::temp_dir().join(format!(
            "gb-fake-ffmpeg-convert-test-{}.webp.tmp",
            std::process::id()
        ));

        assert!(fake
            .convert_image_to_webp(Path::new("corrupt.jpg"), &tmp, 80)
            .is_err());
        assert!(fake
            .convert_image_to_webp(Path::new("ok.jpg"), &tmp, 80)
            .is_ok());
        assert!(
            tmp.exists(),
            "a successful convert_image_to_webp should write the output file"
        );
        assert_eq!(
            fake.calls.lock().unwrap().as_slice(),
            [
                FakeCall::ConvertImageToWebp {
                    src_path: PathBuf::from("corrupt.jpg"),
                    output_tmp_path: tmp.clone(),
                    quality: 80,
                },
                FakeCall::ConvertImageToWebp {
                    src_path: PathBuf::from("ok.jpg"),
                    output_tmp_path: tmp.clone(),
                    quality: 80,
                },
            ]
        );

        let _ = std::fs::remove_file(&tmp);
    }
}
