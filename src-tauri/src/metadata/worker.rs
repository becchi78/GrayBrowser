//! Per-video metadata probing: pure passthrough to
//! `FfmpegAdapter::probe_metadata`, written back via
//! `queries::update_video_metadata`. No seek-position computation or
//! retry-with-fallback needed here -- unlike thumbnail extraction, there's
//! no "try again at a different position" recovery available for metadata.

use std::path::{Path, PathBuf};

use gb_core::ports::ffmpeg::FfmpegAdapter;

use crate::db::{queries, Db};

pub fn probe_metadata_for_video(
    ffmpeg: &impl FfmpegAdapter,
    db: &Db,
    video_id: &str,
    video_path: &Path,
) -> anyhow::Result<()> {
    let metadata = match ffmpeg.probe_metadata(video_path) {
        Ok(metadata) => metadata,
        Err(e) => {
            // Recorded only on failure (never on success) -- this is the one
            // and only place `metadata_attempts` is incremented, mirroring
            // `thumbnail::worker::generate_thumbnail_for_video`'s identical
            // reasoning, so a direct caller (e.g. the
            // retry_metadata_probe command) counts the same way the
            // automatic queue does, with no risk of double-counting a
            // single failed attempt.
            if let Err(inc_err) =
                queries::increment_metadata_attempts(&db.writer.lock().unwrap(), video_id)
            {
                log::warn!("failed to increment metadata_attempts for {video_id}: {inc_err}");
            }
            return Err(e.into());
        }
    };
    let conn = db.writer.lock().unwrap();
    queries::update_video_metadata(&conn, video_id, &metadata)?;
    Ok(())
}

/// `(id, file_path)` for every online video still missing metadata *and*
/// still has automatic-retry budget left
/// (`gb_core::retry::is_eligible_for_automatic_retry`).
/// Delegates to `queries::list_videos_missing_metadata_with_attempts`,
/// matching `thumbnail::worker::list_videos_missing_thumbnails`'s shape (a
/// worker-module wrapper around the DB read, not called directly from the
/// orchestration function in `mod.rs`).
pub fn list_videos_missing_metadata(db: &Db) -> anyhow::Result<Vec<(String, PathBuf)>> {
    let rows = queries::list_videos_missing_metadata_with_attempts(&db.read_pool)?;
    Ok(rows
        .into_iter()
        .filter(|(_, _, attempts)| {
            gb_core::retry::is_eligible_for_automatic_retry(*attempts as u32)
        })
        .map(|(id, path, _)| (id, PathBuf::from(path)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{init_temp_db, insert_test_video};
    use gb_core::ports::ffmpeg::{FfmpegError, VideoMetadata};
    use gb_core::testing::fake_ffmpeg::FakeFfmpegAdapter;

    #[test]
    fn success_writes_metadata_and_stamps_probed_at() {
        let (_db_dir, db) = init_temp_db();
        insert_test_video(&db, "vid-1", "C:/videos/movie.mp4");

        let ffmpeg = FakeFfmpegAdapter {
            metadata: Ok(VideoMetadata {
                width: Some(1920),
                height: Some(1080),
                video_codec: Some("h264".into()),
                audio_codec: Some("aac".into()),
                bitrate: Some(5_000_000),
                fps: Some(29.97),
            }),
            ..Default::default()
        };

        probe_metadata_for_video(&ffmpeg, &db, "vid-1", Path::new("C:/videos/movie.mp4"))
            .expect("probing should succeed");

        let conn = db.writer.lock().unwrap();
        let (width, height, codec, probed_at): (
            Option<i64>,
            Option<i64>,
            Option<String>,
            Option<String>,
        ) = conn
            .query_row(
                "SELECT width, height, video_codec, probed_at FROM videos WHERE id = 'vid-1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(width, Some(1920));
        assert_eq!(height, Some(1080));
        assert_eq!(codec.as_deref(), Some("h264"));
        assert!(
            probed_at.is_some(),
            "probed_at should be stamped on success"
        );
    }

    #[test]
    fn partial_metadata_still_stamps_probed_at() {
        let (_db_dir, db) = init_temp_db();
        insert_test_video(&db, "vid-2", "C:/videos/movie.mkv");

        let ffmpeg = FakeFfmpegAdapter {
            metadata: Ok(VideoMetadata {
                video_codec: Some("hevc".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        probe_metadata_for_video(&ffmpeg, &db, "vid-2", Path::new("C:/videos/movie.mkv"))
            .expect("a partial-but-successful probe should not be an error");

        let conn = db.writer.lock().unwrap();
        let probed_at: Option<String> = conn
            .query_row("SELECT probed_at FROM videos WHERE id = 'vid-2'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            probed_at.is_some(),
            "a partial result is still a completed probe -- must not be retried"
        );
    }

    #[test]
    fn probe_failure_leaves_probed_at_null_and_returns_err() {
        let (_db_dir, db) = init_temp_db();
        insert_test_video(&db, "vid-3", "C:/videos/broken.mp4");

        let ffmpeg = FakeFfmpegAdapter {
            metadata: Err(FfmpegError::NonZeroExit {
                status: 1,
                stderr: "corrupt file".into(),
            }),
            ..Default::default()
        };

        let result =
            probe_metadata_for_video(&ffmpeg, &db, "vid-3", Path::new("C:/videos/broken.mp4"));
        assert!(result.is_err());

        let conn = db.writer.lock().unwrap();
        let probed_at: Option<String> = conn
            .query_row("SELECT probed_at FROM videos WHERE id = 'vid-3'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            probed_at.is_none(),
            "a failed probe must leave probed_at NULL so it's retried next enqueue"
        );
    }

    #[test]
    fn success_does_not_increment_metadata_attempts() {
        let (_db_dir, db) = init_temp_db();
        insert_test_video(&db, "vid-4", "C:/videos/movie.mp4");

        let ffmpeg = FakeFfmpegAdapter {
            metadata: Ok(VideoMetadata {
                video_codec: Some("h264".into()),
                ..Default::default()
            }),
            ..Default::default()
        };

        probe_metadata_for_video(&ffmpeg, &db, "vid-4", Path::new("C:/videos/movie.mp4"))
            .expect("probing should succeed");

        let conn = db.writer.lock().unwrap();
        let attempts: i64 = conn
            .query_row(
                "SELECT metadata_attempts FROM videos WHERE id = 'vid-4'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            attempts, 0,
            "a successful probe must never increment metadata_attempts"
        );
    }

    #[test]
    fn a_single_failure_increments_metadata_attempts_by_exactly_one() {
        let (_db_dir, db) = init_temp_db();
        insert_test_video(&db, "vid-5", "C:/videos/broken.mp4");

        let ffmpeg = FakeFfmpegAdapter {
            metadata: Err(FfmpegError::NonZeroExit {
                status: 1,
                stderr: "corrupt file".into(),
            }),
            ..Default::default()
        };

        let result =
            probe_metadata_for_video(&ffmpeg, &db, "vid-5", Path::new("C:/videos/broken.mp4"));
        assert!(result.is_err());

        let conn = db.writer.lock().unwrap();
        let attempts: i64 = conn
            .query_row(
                "SELECT metadata_attempts FROM videos WHERE id = 'vid-5'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(attempts, 1);
    }

    /// The off-by-one guard, mirroring
    /// `thumbnail::worker`'s identical test: simulates the automatic
    /// queue's real usage pattern -- "list what's still eligible, attempt
    /// every one of those, repeat" -- across 4 enqueue rounds against a
    /// video whose probe always fails. With `MAX_GENERATION_ATTEMPTS == 3`,
    /// the video must be attempted on exactly the first 3 rounds and be
    /// absent from the 4th round's listing.
    #[test]
    fn a_permanently_failing_video_is_probed_exactly_three_times_then_excluded() {
        let (_db_dir, db) = init_temp_db();
        insert_test_video(&db, "vid-doomed", "C:/videos/doomed.mp4");

        let ffmpeg = FakeFfmpegAdapter {
            metadata: Err(FfmpegError::NonZeroExit {
                status: 1,
                stderr: "always fails".into(),
            }),
            ..Default::default()
        };

        let mut was_eligible_by_round = Vec::new();
        for _ in 0..4 {
            let missing = list_videos_missing_metadata(&db).unwrap();
            let eligible = missing.iter().any(|(id, _)| id == "vid-doomed");
            was_eligible_by_round.push(eligible);
            if eligible {
                let _ = probe_metadata_for_video(
                    &ffmpeg,
                    &db,
                    "vid-doomed",
                    Path::new("C:/videos/doomed.mp4"),
                );
            }
        }

        assert_eq!(
            was_eligible_by_round,
            vec![true, true, true, false],
            "must be eligible (and thus attempted) on rounds 1-3, and excluded on round 4"
        );

        let conn = db.writer.lock().unwrap();
        let attempts: i64 = conn
            .query_row(
                "SELECT metadata_attempts FROM videos WHERE id = 'vid-doomed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(attempts, 3, "exactly 3 failed attempts, no more, no less");

        let calls = ffmpeg.calls.lock().unwrap();
        let probe_call_count = calls
            .iter()
            .filter(|c| matches!(c, gb_core::testing::fake_ffmpeg::FakeCall::ProbeMetadata(_)))
            .count();
        assert_eq!(probe_call_count, 3);
    }

    #[test]
    fn list_videos_missing_metadata_returns_only_unprobed_online_videos() {
        let (_db_dir, db) = init_temp_db();
        insert_test_video(&db, "probed", "C:/videos/a.mp4");
        insert_test_video(&db, "unprobed", "C:/videos/b.mp4");
        {
            let conn = db.writer.lock().unwrap();
            conn.execute(
                "UPDATE videos SET probed_at = CURRENT_TIMESTAMP WHERE id = 'probed'",
                [],
            )
            .unwrap();
        }

        let missing = list_videos_missing_metadata(&db).unwrap();
        let ids: Vec<&str> = missing.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["unprobed"]);
    }
}
