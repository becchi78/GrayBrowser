//! Commands for listing videos whose automatic thumbnail/metadata
//! generation has exhausted its retry budget
//! (`gb_core::retry::MAX_GENERATION_ATTEMPTS`), plus a manual "retry this
//! one" action for each pipeline that resets the counter and immediately
//! attempts generation once more.
//!
//! Both retry commands deliberately reuse `thumbnail::worker::
//! generate_thumbnail_for_video`/`metadata::worker::probe_metadata_for_video`
//! directly rather than re-implementing the attempt -- those functions
//! already own the "increment the counter on failure, never on success"
//! rule, so calling them here means a manual retry counts exactly
//! like an automatic one, with no risk of double-incrementing.

use tauri::State;

use crate::adapters;
use crate::db::{queries, Db};
use crate::events::{CatalogNotifier, TauriCatalogNotifier};
use crate::{metadata, thumbnail};

/// One row of `list_generation_failures`'s thumbnail half -- a thin
/// `serde::Serialize` mirror of `queries::ExhaustedThumbnailRow`, matching
/// this codebase's usual `*Row` (DB layer) -> `*Dto` (IPC layer) split (see
/// `scan_cmds::VideoDto`/`SkippedFileDto`).
#[derive(serde::Serialize)]
pub struct ExhaustedThumbnailDto {
    pub id: String,
    pub file_path: String,
    pub file_name: String,
    pub thumbnail_attempts: i64,
}

impl From<queries::ExhaustedThumbnailRow> for ExhaustedThumbnailDto {
    fn from(row: queries::ExhaustedThumbnailRow) -> Self {
        Self {
            id: row.id,
            file_path: row.file_path,
            file_name: row.file_name,
            thumbnail_attempts: row.thumbnail_attempts,
        }
    }
}

/// Metadata-pipeline counterpart of `ExhaustedThumbnailDto`.
#[derive(serde::Serialize)]
pub struct ExhaustedMetadataDto {
    pub id: String,
    pub file_path: String,
    pub file_name: String,
    pub metadata_attempts: i64,
}

impl From<queries::ExhaustedMetadataRow> for ExhaustedMetadataDto {
    fn from(row: queries::ExhaustedMetadataRow) -> Self {
        Self {
            id: row.id,
            file_path: row.file_path,
            file_name: row.file_name,
            metadata_attempts: row.metadata_attempts,
        }
    }
}

#[derive(serde::Serialize)]
pub struct GenerationFailuresDto {
    pub thumbnail_failures: Vec<ExhaustedThumbnailDto>,
    pub metadata_failures: Vec<ExhaustedMetadataDto>,
}

/// Returns every online video whose thumbnail and/or metadata generation has
/// exhausted its automatic-retry budget, for a notification panel.
#[tauri::command]
pub fn list_generation_failures(db: State<Db>) -> Result<GenerationFailuresDto, String> {
    let thumbnail_failures = queries::list_videos_with_exhausted_thumbnail_attempts(&db.read_pool)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(ExhaustedThumbnailDto::from)
        .collect();
    let metadata_failures = queries::list_videos_with_exhausted_metadata_attempts(&db.read_pool)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(ExhaustedMetadataDto::from)
        .collect();
    Ok(GenerationFailuresDto {
        thumbnail_failures,
        metadata_failures,
    })
}

/// Resets `thumbnail_attempts` to 0 for `video_id` and immediately attempts
/// generation once more. Whether that immediate attempt succeeds or fails,
/// this always emits `catalog:changed` so the frontend picks up either the
/// new thumbnail or the (re-)incremented attempts count -- and always
/// returns `Ok(())`: a failed retry is not this command's error, it's
/// exactly the outcome `list_generation_failures` (re-queried by the
/// frontend after the event) is there to report.
#[tauri::command]
pub fn retry_thumbnail_generation(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    video_id: String,
) -> Result<(), String> {
    {
        let conn = db.writer.lock().unwrap();
        queries::reset_thumbnail_attempts(&conn, &video_id).map_err(|e| e.to_string())?;
    }

    let video = queries::find_video_by_id(&db.read_pool, &video_id).map_err(|e| e.to_string())?;
    match video {
        Some(video) => {
            let thumbnails_dir = crate::paths::app_data_dir()
                .map(|dir| dir.join("thumbnails"))
                .map_err(|e| e.to_string())?;
            let _ = std::fs::create_dir_all(&thumbnails_dir);

            let ffmpeg = adapters::ffmpeg::RealFfmpegAdapter;
            if let Err(e) = thumbnail::worker::generate_thumbnail_for_video(
                &ffmpeg,
                db.inner(),
                &thumbnails_dir,
                &video_id,
                std::path::Path::new(&video.file_path),
            ) {
                log::warn!("manual thumbnail retry failed for {video_id}: {e}");
            }
        }
        None => log::warn!("retry_thumbnail_generation: video {video_id} not found"),
    }

    TauriCatalogNotifier::new(app).notify_changed();
    Ok(())
}

/// Metadata-pipeline counterpart of `retry_thumbnail_generation`: resets
/// `metadata_attempts` to 0 and immediately probes once more.
#[tauri::command]
pub fn retry_metadata_probe(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    video_id: String,
) -> Result<(), String> {
    {
        let conn = db.writer.lock().unwrap();
        queries::reset_metadata_attempts(&conn, &video_id).map_err(|e| e.to_string())?;
    }

    let video = queries::find_video_by_id(&db.read_pool, &video_id).map_err(|e| e.to_string())?;
    match video {
        Some(video) => {
            let ffmpeg = adapters::ffmpeg::RealFfmpegAdapter;
            if let Err(e) = metadata::worker::probe_metadata_for_video(
                &ffmpeg,
                db.inner(),
                &video_id,
                std::path::Path::new(&video.file_path),
            ) {
                log::warn!("manual metadata retry failed for {video_id}: {e}");
            }
        }
        None => log::warn!("retry_metadata_probe: video {video_id} not found"),
    }

    TauriCatalogNotifier::new(app).notify_changed();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{init_temp_db, insert_test_video};

    /// `retry_thumbnail_generation`/`retry_metadata_probe` need a real
    /// `tauri::AppHandle` to notify with, which this codebase's own
    /// `events` module documents as unsafe to construct inside `cargo test`
    /// on Windows (`STATUS_ENTRYPOINT_NOT_FOUND`, see `events.rs`'s top doc
    /// comment). So instead of exercising the `#[tauri::command]` fns
    /// themselves, these tests cover the exact same reset-then-retry
    /// sequence they perform, directly against `queries`/`thumbnail::
    /// worker`/`metadata::worker` -- the parts that are actually
    /// interesting (attempts hits 0, then exactly one more attempt runs)
    /// and are fully AppHandle-independent.
    #[test]
    fn resetting_thumbnail_attempts_then_retrying_zeroes_and_then_recounts_a_failure() {
        use gb_core::ports::ffmpeg::FfmpegError;
        use gb_core::testing::fake_ffmpeg::FakeFfmpegAdapter;

        let (_db_dir, db) = init_temp_db();
        let thumbs_dir = tempfile::tempdir().unwrap();
        insert_test_video(&db, "vid-1", "C:/videos/movie.mp4");

        {
            let conn = db.writer.lock().unwrap();
            for _ in 0..gb_core::retry::MAX_GENERATION_ATTEMPTS {
                queries::increment_thumbnail_attempts(&conn, "vid-1").unwrap();
            }
        }
        {
            let conn = db.writer.lock().unwrap();
            let attempts: i64 = conn
                .query_row(
                    "SELECT thumbnail_attempts FROM videos WHERE id = 'vid-1'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(attempts, i64::from(gb_core::retry::MAX_GENERATION_ATTEMPTS));
        }

        {
            let conn = db.writer.lock().unwrap();
            queries::reset_thumbnail_attempts(&conn, "vid-1").unwrap();
        }
        {
            let conn = db.writer.lock().unwrap();
            let attempts: i64 = conn
                .query_row(
                    "SELECT thumbnail_attempts FROM videos WHERE id = 'vid-1'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(attempts, 0, "reset must zero the counter");
        }

        let ffmpeg = FakeFfmpegAdapter {
            duration: Ok(Some(120.0)),
            extract_result: Box::new(|_seek| {
                Err(FfmpegError::NonZeroExit {
                    status: 1,
                    stderr: "still broken".into(),
                })
            }),
            ..Default::default()
        };
        let result = thumbnail::worker::generate_thumbnail_for_video(
            &ffmpeg,
            &db,
            thumbs_dir.path(),
            "vid-1",
            std::path::Path::new("C:/videos/movie.mp4"),
        );
        assert!(result.is_err());

        let conn = db.writer.lock().unwrap();
        let attempts: i64 = conn
            .query_row(
                "SELECT thumbnail_attempts FROM videos WHERE id = 'vid-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            attempts, 1,
            "the immediate retry attempt is exactly one attempt, not zero and not two"
        );
    }

    #[test]
    fn resetting_metadata_attempts_then_retrying_zeroes_and_then_recounts_a_failure() {
        use gb_core::ports::ffmpeg::FfmpegError;
        use gb_core::testing::fake_ffmpeg::FakeFfmpegAdapter;

        let (_db_dir, db) = init_temp_db();
        insert_test_video(&db, "vid-2", "C:/videos/movie.mp4");

        {
            let conn = db.writer.lock().unwrap();
            for _ in 0..gb_core::retry::MAX_GENERATION_ATTEMPTS {
                queries::increment_metadata_attempts(&conn, "vid-2").unwrap();
            }
        }

        {
            let conn = db.writer.lock().unwrap();
            queries::reset_metadata_attempts(&conn, "vid-2").unwrap();
        }
        {
            let conn = db.writer.lock().unwrap();
            let attempts: i64 = conn
                .query_row(
                    "SELECT metadata_attempts FROM videos WHERE id = 'vid-2'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(attempts, 0, "reset must zero the counter");
        }

        let ffmpeg = FakeFfmpegAdapter {
            metadata: Err(FfmpegError::NonZeroExit {
                status: 1,
                stderr: "still broken".into(),
            }),
            ..Default::default()
        };
        let result = metadata::worker::probe_metadata_for_video(
            &ffmpeg,
            &db,
            "vid-2",
            std::path::Path::new("C:/videos/movie.mp4"),
        );
        assert!(result.is_err());

        let conn = db.writer.lock().unwrap();
        let attempts: i64 = conn
            .query_row(
                "SELECT metadata_attempts FROM videos WHERE id = 'vid-2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            attempts, 1,
            "the immediate retry attempt is exactly one attempt, not zero and not two"
        );
    }

    #[test]
    fn exhausted_dto_conversions_preserve_every_field() {
        let thumb_row = queries::ExhaustedThumbnailRow {
            id: "vid-1".into(),
            file_path: "C:/videos/movie.mp4".into(),
            file_name: "movie.mp4".into(),
            thumbnail_attempts: 3,
        };
        let dto = ExhaustedThumbnailDto::from(thumb_row);
        assert_eq!(dto.id, "vid-1");
        assert_eq!(dto.file_path, "C:/videos/movie.mp4");
        assert_eq!(dto.file_name, "movie.mp4");
        assert_eq!(dto.thumbnail_attempts, 3);

        let meta_row = queries::ExhaustedMetadataRow {
            id: "vid-2".into(),
            file_path: "C:/videos/movie2.mp4".into(),
            file_name: "movie2.mp4".into(),
            metadata_attempts: 4,
        };
        let dto = ExhaustedMetadataDto::from(meta_row);
        assert_eq!(dto.id, "vid-2");
        assert_eq!(dto.file_path, "C:/videos/movie2.mp4");
        assert_eq!(dto.file_name, "movie2.mp4");
        assert_eq!(dto.metadata_attempts, 4);
    }
}
