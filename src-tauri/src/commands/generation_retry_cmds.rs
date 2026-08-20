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

use std::path::{Path, PathBuf};

use gb_core::ports::ffmpeg::FfmpegAdapter;
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
///
/// This "always `Ok(())`, always notify" promise covers *every* failure mode
/// downstream of the `reset_thumbnail_attempts` write above, including
/// `crate::paths::app_data_dir()` itself failing (see `attempt_thumbnail_retry`,
/// which is where that's handled) -- unlike `remove_watch_folder`/
/// `rename_watch_folder`, whose own DB mutations *are* the command's main
/// point and therefore must never proceed if `app_data_dir()` can't be
/// resolved, this command's DB write (`reset_thumbnail_attempts`) is
/// unconditionally safe to keep regardless of whether the thumbnail retry
/// that follows can actually run, so there is nothing to protect by moving
/// the `app_data_dir()` call earlier here.
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
            let ffmpeg = adapters::ffmpeg::RealFfmpegAdapter;
            attempt_thumbnail_retry(
                db.inner(),
                &ffmpeg,
                &video_id,
                &video,
                crate::paths::app_data_dir().map_err(|e| e.to_string()),
            );
        }
        None => log::warn!("retry_thumbnail_generation: video {video_id} not found"),
    }

    TauriCatalogNotifier::new(app).notify_changed();
    Ok(())
}

/// The actual thumbnail-retry attempt, factored out of the `#[tauri::command]`
/// wrapper so its best-effort handling of an `app_dir` resolution failure is
/// unit-testable without depending on `crate::paths::app_data_dir()`'s real
/// `std::env::current_exe()` call (which cannot be made to fail
/// deterministically in a test).
///
/// This function itself never returns an error and is never allowed to --
/// `retry_thumbnail_generation`'s own doc comment promises "always returns
/// `Ok(())`, a failed retry is not this command's error", and that promise
/// must hold for *every* failure mode here, not just `generate_thumbnail_for_video`
/// itself failing. In particular, `app_dir` being an `Err` (`crate::paths::
/// app_data_dir()` failed) is treated the same way as any other best-effort
/// thumbnail failure: logged and skipped, never propagated, never panicking
/// -- regardless of whether it's called before or after some other DB write,
/// since nothing downstream of it is allowed to fail this command either.
fn attempt_thumbnail_retry(
    db: &Db,
    ffmpeg: &impl FfmpegAdapter,
    video_id: &str,
    video: &queries::VideoRow,
    app_dir: Result<PathBuf, String>,
) {
    let app_dir = match app_dir {
        Ok(dir) => dir,
        Err(e) => {
            log::warn!(
                "manual thumbnail retry skipped for {video_id}: failed to resolve the app data \
                 directory: {e}"
            );
            return;
        }
    };
    let thumbnails_root = thumbnail::paths::thumbnails_root(&app_dir);
    let watch_folders = match db.read_pool.get() {
        Ok(conn) => queries::get_watch_folders(&conn).unwrap_or_default(),
        Err(e) => {
            log::warn!(
                "failed to acquire a DB connection to read watch_folders for thumbnail retry \
                 of {video_id}: {e}"
            );
            Vec::new()
        }
    };
    let video_dir =
        thumbnail::paths::video_thumbnail_dir(&thumbnails_root, &watch_folders, &video.file_path);
    // `generate_thumbnail_for_video` itself `create_dir_all`s `video_dir` if
    // it doesn't exist yet, so no separate call is needed here.
    if let Err(e) = thumbnail::worker::generate_thumbnail_for_video(
        ffmpeg,
        db,
        &video_dir,
        video_id,
        Path::new(&video.file_path),
    ) {
        log::warn!("manual thumbnail retry failed for {video_id}: {e}");
    }
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

    /// Regression test for the best-effort contract `attempt_thumbnail_retry`'s
    /// doc comment describes: an `app_dir` resolution failure (simulated here
    /// via a plain `Err(...)`, standing in for `crate::paths::app_data_dir()`
    /// itself failing -- not reproducible deterministically in a test, since
    /// it wraps a real `std::env::current_exe()` call) must be a silent skip,
    /// never propagated as an error and never allowed to reach
    /// `generate_thumbnail_for_video` at all.
    #[test]
    fn attempt_thumbnail_retry_is_a_best_effort_skip_when_app_dir_resolution_fails() {
        use gb_core::testing::fake_ffmpeg::FakeFfmpegAdapter;

        let (_db_dir, db) = init_temp_db();
        insert_test_video(&db, "vid-appdir-fail", "C:/videos/movie.mp4");
        let video = queries::find_video_by_id(&db.read_pool, "vid-appdir-fail")
            .unwrap()
            .unwrap();

        let ffmpeg = FakeFfmpegAdapter {
            duration: Ok(Some(120.0)),
            extract_result: Box::new(|_seek| Ok(())),
            ..Default::default()
        };

        attempt_thumbnail_retry(
            &db,
            &ffmpeg,
            "vid-appdir-fail",
            &video,
            Err("simulated app_data_dir failure".to_string()),
        );

        assert!(
            ffmpeg.calls.lock().unwrap().is_empty(),
            "generate_thumbnail_for_video (and therefore the ffmpeg adapter) must never be \
             reached when app_dir resolution failed"
        );

        let conn = db.writer.lock().unwrap();
        let attempts: i64 = conn
            .query_row(
                "SELECT thumbnail_attempts FROM videos WHERE id = 'vid-appdir-fail'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            attempts, 0,
            "a best-effort skip must not be counted as a failed generation attempt"
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
