//! Per-video thumbnail generation: probe duration, compute 6 evenly spaced
//! seek positions, extract each via `FfmpegAdapter` with a one-shot 0s
//! fallback per slot on failure, then atomically rename each `.tmp` ->
//! final -- but only once all 6 slots have succeeded. If any slot fails
//! permanently, extraction of the remaining slots is skipped (fail-fast)
//! and every `.tmp` file produced so far for this video is cleaned up.

use std::path::{Path, PathBuf};

use gb_core::ports::ffmpeg::FfmpegAdapter;

use crate::db::{queries, Db};

const THUMBNAIL_WIDTH_PX: u32 = 320;
const THUMBNAIL_QUALITY: u8 = 55;

/// Number of thumbnails generated per video: 6 evenly spaced frames per
/// row.
pub const THUMBNAILS_PER_VIDEO: usize = 6;

pub fn generate_thumbnail_for_video(
    ffmpeg: &impl FfmpegAdapter,
    db: &Db,
    thumbnails_dir: &Path,
    video_id: &str,
    video_path: &Path,
) -> anyhow::Result<()> {
    // A probe failure just means "duration unknown" --
    // thumbnail_seek_positions already handles that gracefully, so it's not
    // treated as fatal here.
    let duration = ffmpeg.probe_duration(video_path).ok().flatten();

    // Written back as soon as we know it, independent of whether the frame
    // extraction below succeeds (an unsupported codec can still report a
    // valid duration even if we can't grab a frame from it).
    if let Some(secs) = duration {
        let conn = db.writer.lock().unwrap();
        if let Err(e) = queries::update_video_duration(&conn, video_id, secs) {
            log::warn!("failed to write back duration for {video_id}: {e}");
        }
    }

    let seeks = gb_core::thumbnail_policy::thumbnail_seek_positions(duration);

    let mut tmp_paths: Vec<PathBuf> = Vec::with_capacity(THUMBNAILS_PER_VIDEO);
    let mut extraction_err: Option<anyhow::Error> = None;

    for (i, seek) in seeks.into_iter().enumerate() {
        let tmp_path = thumbnails_dir.join(format!("{video_id}_{i}.webp.tmp"));

        let slot_result = match ffmpeg.extract_thumbnail(
            video_path,
            &tmp_path,
            seek,
            THUMBNAIL_WIDTH_PX,
            THUMBNAIL_QUALITY,
        ) {
            Ok(()) => Ok(()),
            Err(first_err) => match gb_core::thumbnail_policy::fallback_seek_seconds(seek) {
                Some(fallback) => ffmpeg
                    .extract_thumbnail(video_path, &tmp_path, fallback, THUMBNAIL_WIDTH_PX, THUMBNAIL_QUALITY)
                    .map_err(|second_err| anyhow::anyhow!("slot {i}: seek {seek}s failed ({first_err}); fallback to 0s also failed ({second_err})")),
                None => Err(anyhow::anyhow!("slot {i}: seek {seek}s failed: {first_err}")),
            },
        };

        match slot_result {
            Ok(()) => tmp_paths.push(tmp_path),
            Err(e) => {
                // Fail-fast: don't bother attempting the remaining slots
                // once one has permanently failed -- they'd just be
                // discarded anyway since a video's thumbnails are all-or-
                // nothing.
                extraction_err = Some(e);
                break;
            }
        }
    }

    match extraction_err {
        None => {
            debug_assert_eq!(tmp_paths.len(), THUMBNAILS_PER_VIDEO);
            for (i, tmp_path) in tmp_paths.iter().enumerate() {
                let final_path = thumbnails_dir.join(format!("{video_id}_{i}.webp"));
                std::fs::rename(tmp_path, &final_path)?;
            }
            // Keeps videos.thumbnail_ready (migration 0008) in
            // sync with the files that were just written, so list_videos's
            // hot path never has to stat() these files itself. Best-effort: a
            // failure to write this flag must not turn an otherwise-
            // successful generation into an error -- worst case, the files
            // exist but the flag lags behind until the next
            // list_videos_missing_thumbnails resume pass backfills it.
            if let Err(e) = queries::mark_thumbnail_ready(&db.writer.lock().unwrap(), video_id) {
                log::warn!("failed to mark thumbnail_ready for {video_id}: {e}");
            }
            Ok(())
        }
        Some(e) => {
            log::warn!(
                "thumbnail generation failed for video {video_id} ({}): {e}",
                video_path.display()
            );
            // best-effort cleanup of every tmp file produced before the
            // failing slot (the failing slot itself never wrote a tmp file
            // that needs cleaning up, since extract_thumbnail only writes
            // its output on success).
            for tmp_path in &tmp_paths {
                let _ = std::fs::remove_file(tmp_path);
            }
            // Recorded only on failure (never on success), and only once per
            // video regardless of which slot failed -- this is the one and
            // only place `thumbnail_attempts` is incremented, so a caller
            // invoking this function directly (e.g. the
            // retry_thumbnail_generation command) gets the same counting
            // behavior as the automatic queue, with no risk of double-
            // counting a single failed attempt.
            if let Err(inc_err) =
                queries::increment_thumbnail_attempts(&db.writer.lock().unwrap(), video_id)
            {
                log::warn!("failed to increment thumbnail_attempts for {video_id}: {inc_err}");
            }
            Err(e)
        }
    }
}

/// `(id, file_path)` for every online video for which any of the 6
/// `thumbnails/[id]_0.webp`..`[id]_5.webp` files doesn't exist yet *and*
/// still has automatic-retry budget left
/// (`gb_core::retry::is_eligible_for_automatic_retry`). This --
/// not a persisted job table -- is the entire "resume after restart"
/// mechanism: whatever's still missing on disk (and hasn't
/// exhausted its attempts) is, by definition, still pending.
///
/// This is deliberately the one place in the app (besides the generation
/// worker's own write) that still treats the filesystem, not
/// `videos.thumbnail_ready`, as the source of truth -- it runs once per
/// scan/startup, not on every `list_videos` call, so its per-row `.exists()`
/// check doesn't recreate the hot-path cost the `thumbnail_ready` flag was
/// added to avoid. As a
/// side effect, it also backfills `thumbnail_ready` for any row where the
/// file already exists on disk but the DB flag is still stale (e.g. every
/// row that existed before migration 0008 ever ran, or any other reason the
/// flag write was missed) -- reusing this loop's existing `.exists()` result
/// rather than adding a second filesystem check anywhere.
///
/// The backfill write (`mark_thumbnail_ready`, which takes
/// `db.writer`'s lock) only runs for rows whose DB flag isn't already `1`.
/// Without this guard, every resume pass over a library that's already fully
/// backfilled would still take `db.writer`'s lock and issue an `UPDATE` once
/// per online video, for no effect (`mark_thumbnail_ready`'s own
/// `AND thumbnail_ready = 0` guard already made those writes no-ops, but the
/// lock acquisition + statement execution cost was still paid every time).
pub fn list_videos_missing_thumbnails(
    db: &Db,
    thumbnails_dir: &Path,
) -> anyhow::Result<Vec<(String, PathBuf)>> {
    let all_online = queries::list_online_video_paths_with_thumbnail_attempts(&db.read_pool)?;
    let mut missing = Vec::new();
    for (id, path, attempts, thumbnail_ready) in all_online {
        let file_exists = (0..THUMBNAILS_PER_VIDEO)
            .all(|i| thumbnails_dir.join(format!("{id}_{i}.webp")).exists());
        if file_exists {
            // Self-healing backfill (see doc comment above): the file is
            // already there, so this video is never "missing" regardless of
            // what the DB flag currently says -- but make sure the flag
            // catches up so the hot path (list_videos) sees it too. Only
            // needed when the DB doesn't already agree the flag is set.
            if !thumbnail_ready {
                if let Err(e) = queries::mark_thumbnail_ready(&db.writer.lock().unwrap(), &id) {
                    log::warn!("failed to backfill thumbnail_ready for {id}: {e}");
                }
            }
        } else if gb_core::retry::is_eligible_for_automatic_retry(attempts as u32) {
            missing.push((id, PathBuf::from(path)));
        }
    }
    Ok(missing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{init_temp_db, insert_test_video};
    use gb_core::ports::ffmpeg::FfmpegError;
    use gb_core::testing::fake_ffmpeg::FakeCall;
    use gb_core::testing::fake_ffmpeg::FakeFfmpegAdapter;

    /// Writes all 6 final `{id}_{i}.webp` files for `id` directly (bypassing
    /// generation), simulating "this video's thumbnails were already
    /// generated in a previous run".
    fn write_all_six_final_thumbnails(thumbs_dir: &Path, id: &str) {
        for i in 0..THUMBNAILS_PER_VIDEO {
            std::fs::write(
                thumbs_dir.join(format!("{id}_{i}.webp")),
                b"already generated",
            )
            .unwrap();
        }
    }

    fn all_six_final_files_exist(thumbs_dir: &Path, id: &str) -> bool {
        (0..THUMBNAILS_PER_VIDEO).all(|i| thumbs_dir.join(format!("{id}_{i}.webp")).exists())
    }

    fn any_tmp_file_exists(thumbs_dir: &Path, id: &str) -> bool {
        (0..THUMBNAILS_PER_VIDEO).any(|i| thumbs_dir.join(format!("{id}_{i}.webp.tmp")).exists())
    }

    #[test]
    fn success_writes_all_six_final_files_and_duration_with_no_leftover_tmp() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_dir = tempfile::tempdir().unwrap();
        insert_test_video(&db, "vid-1", "C:/videos/movie.mp4");

        let ffmpeg = FakeFfmpegAdapter {
            duration: Ok(Some(120.0)),
            extract_result: Box::new(|_seek| Ok(())),
            ..Default::default()
        };

        generate_thumbnail_for_video(
            &ffmpeg,
            &db,
            thumbs_dir.path(),
            "vid-1",
            Path::new("C:/videos/movie.mp4"),
        )
        .expect("generation should succeed");

        assert!(all_six_final_files_exist(thumbs_dir.path(), "vid-1"));
        assert!(!any_tmp_file_exists(thumbs_dir.path(), "vid-1"));

        let conn = db.writer.lock().unwrap();
        let duration: Option<i64> = conn
            .query_row("SELECT duration FROM videos WHERE id = 'vid-1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(duration, Some(120));
    }

    #[test]
    fn every_slot_that_fails_its_initial_seek_falls_back_to_zero_and_all_six_succeed() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_dir = tempfile::tempdir().unwrap();
        insert_test_video(&db, "vid-2", "C:/videos/movie.mp4");

        // duration=120.0 -> all 6 computed seeks (120*1/7 .. 120*6/7) are
        // non-zero, so this fails every slot's initial seek and rescues all
        // 6 via the 0s fallback.
        let ffmpeg = FakeFfmpegAdapter {
            duration: Ok(Some(120.0)),
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

        generate_thumbnail_for_video(
            &ffmpeg,
            &db,
            thumbs_dir.path(),
            "vid-2",
            Path::new("C:/videos/movie.mp4"),
        )
        .expect("fallback should rescue every slot");

        assert!(all_six_final_files_exist(thumbs_dir.path(), "vid-2"));
        let calls = ffmpeg.calls.lock().unwrap();
        let extract_calls: Vec<f64> = calls
            .iter()
            .filter_map(|c| match c {
                FakeCall::ExtractThumbnail { seek_secs, .. } => Some(*seek_secs),
                _ => None,
            })
            .collect();
        // 6 slots, each: (non-zero primary seek, 0.0 fallback).
        assert_eq!(extract_calls.len(), 12);
        for pair in extract_calls.chunks(2) {
            assert_ne!(
                pair[0], 0.0,
                "primary seek for each slot should be non-zero"
            );
            assert_eq!(pair[1], 0.0, "fallback seek for each slot should be 0.0");
        }
    }

    #[test]
    fn permanent_failure_at_the_first_slot_leaves_no_files_and_returns_err() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_dir = tempfile::tempdir().unwrap();
        insert_test_video(&db, "vid-3", "C:/videos/movie.mp4");

        let ffmpeg = FakeFfmpegAdapter {
            duration: Ok(Some(120.0)),
            extract_result: Box::new(|_seek| {
                Err(FfmpegError::NonZeroExit {
                    status: 1,
                    stderr: "corrupt file".into(),
                })
            }),
            ..Default::default()
        };

        let result = generate_thumbnail_for_video(
            &ffmpeg,
            &db,
            thumbs_dir.path(),
            "vid-3",
            Path::new("C:/videos/movie.mp4"),
        );

        assert!(result.is_err());
        assert!(!all_six_final_files_exist(thumbs_dir.path(), "vid-3"));
        for i in 0..THUMBNAILS_PER_VIDEO {
            assert!(!thumbs_dir.path().join(format!("vid-3_{i}.webp")).exists());
            assert!(!thumbs_dir
                .path()
                .join(format!("vid-3_{i}.webp.tmp"))
                .exists());
        }
    }

    /// Fail-fast guard: once slot 0's primary and fallback seeks have both
    /// failed, slots 1..5 must never be attempted at all -- not just that
    /// their output happens to be discarded. Achieved by having the fake
    /// fail only the first 2 extract_thumbnail calls (slot 0's primary and
    /// fallback) and succeed on any later call, so if fail-fast were broken
    /// and slot 1 were attempted anyway, its calls would show up as extra,
    /// successful `FakeCall::ExtractThumbnail` entries beyond the first 2.
    #[test]
    fn fail_fast_skips_the_remaining_slots_after_a_permanent_failure() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_dir = tempfile::tempdir().unwrap();
        insert_test_video(&db, "vid-fastfail", "C:/videos/movie.mp4");

        let call_count = std::sync::atomic::AtomicUsize::new(0);
        let ffmpeg = FakeFfmpegAdapter {
            duration: Ok(Some(120.0)),
            extract_result: Box::new(move |_seek| {
                let n = call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n < 2 {
                    Err(FfmpegError::NonZeroExit {
                        status: 1,
                        stderr: "corrupt file".into(),
                    })
                } else {
                    Ok(())
                }
            }),
            ..Default::default()
        };

        let result = generate_thumbnail_for_video(
            &ffmpeg,
            &db,
            thumbs_dir.path(),
            "vid-fastfail",
            Path::new("C:/videos/movie.mp4"),
        );

        assert!(result.is_err());
        let calls = ffmpeg.calls.lock().unwrap();
        let extract_paths: Vec<PathBuf> = calls
            .iter()
            .filter_map(|c| match c {
                FakeCall::ExtractThumbnail {
                    output_tmp_path, ..
                } => Some(output_tmp_path.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            extract_paths.len(),
            2,
            "only slot 0's primary + fallback attempts should have run"
        );
        for path in &extract_paths {
            assert_eq!(
                path,
                &thumbs_dir.path().join("vid-fastfail_0.webp.tmp"),
                "no slot other than 0 should ever have been attempted"
            );
        }
    }

    #[test]
    fn duration_is_written_back_even_when_extraction_fails() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_dir = tempfile::tempdir().unwrap();
        insert_test_video(&db, "vid-4", "C:/videos/movie.mp4");

        let ffmpeg = FakeFfmpegAdapter {
            duration: Ok(Some(42.0)),
            extract_result: Box::new(|_seek| {
                Err(FfmpegError::NonZeroExit {
                    status: 1,
                    stderr: "unsupported codec".into(),
                })
            }),
            ..Default::default()
        };

        let _ = generate_thumbnail_for_video(
            &ffmpeg,
            &db,
            thumbs_dir.path(),
            "vid-4",
            Path::new("C:/videos/movie.mp4"),
        );

        let conn = db.writer.lock().unwrap();
        let duration: Option<i64> = conn
            .query_row("SELECT duration FROM videos WHERE id = 'vid-4'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(duration, Some(42));
    }

    #[test]
    fn list_videos_missing_thumbnails_returns_only_videos_without_all_six_thumbnail_files() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_dir = tempfile::tempdir().unwrap();
        insert_test_video(&db, "has-thumb", "C:/videos/a.mp4");
        insert_test_video(&db, "missing-thumb", "C:/videos/b.mp4");
        insert_test_video(&db, "partial-thumb", "C:/videos/c.mp4");
        write_all_six_final_thumbnails(thumbs_dir.path(), "has-thumb");
        // Only 5 of the 6 slots present -- must still count as missing.
        for i in 0..(THUMBNAILS_PER_VIDEO - 1) {
            std::fs::write(
                thumbs_dir.path().join(format!("partial-thumb_{i}.webp")),
                b"already generated",
            )
            .unwrap();
        }

        let missing = list_videos_missing_thumbnails(&db, thumbs_dir.path()).unwrap();
        let mut ids: Vec<&str> = missing.iter().map(|(id, _)| id.as_str()).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec!["missing-thumb", "partial-thumb"]);
    }

    #[test]
    fn success_does_not_increment_thumbnail_attempts() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_dir = tempfile::tempdir().unwrap();
        insert_test_video(&db, "vid-6", "C:/videos/movie.mp4");

        let ffmpeg = FakeFfmpegAdapter {
            duration: Ok(Some(120.0)),
            extract_result: Box::new(|_seek| Ok(())),
            ..Default::default()
        };

        generate_thumbnail_for_video(
            &ffmpeg,
            &db,
            thumbs_dir.path(),
            "vid-6",
            Path::new("C:/videos/movie.mp4"),
        )
        .expect("generation should succeed");

        let conn = db.writer.lock().unwrap();
        let attempts: i64 = conn
            .query_row(
                "SELECT thumbnail_attempts FROM videos WHERE id = 'vid-6'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            attempts, 0,
            "a successful generation must never increment thumbnail_attempts"
        );
    }

    #[test]
    fn a_single_failure_increments_thumbnail_attempts_by_exactly_one() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_dir = tempfile::tempdir().unwrap();
        insert_test_video(&db, "vid-7", "C:/videos/movie.mp4");

        let ffmpeg = FakeFfmpegAdapter {
            duration: Ok(Some(120.0)),
            extract_result: Box::new(|_seek| {
                Err(FfmpegError::NonZeroExit {
                    status: 1,
                    stderr: "corrupt file".into(),
                })
            }),
            ..Default::default()
        };

        let result = generate_thumbnail_for_video(
            &ffmpeg,
            &db,
            thumbs_dir.path(),
            "vid-7",
            Path::new("C:/videos/movie.mp4"),
        );
        assert!(result.is_err());

        let conn = db.writer.lock().unwrap();
        let attempts: i64 = conn
            .query_row(
                "SELECT thumbnail_attempts FROM videos WHERE id = 'vid-7'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            attempts, 1,
            "exactly one attempt must be recorded per generate_thumbnail_for_video call, \
             regardless of how many of the 6 slots were attempted before the fail-fast break"
        );
    }

    /// The off-by-one guard: simulates the automatic queue's real usage
    /// pattern -- "list what's still eligible, attempt every one of
    /// those, repeat" -- across 4 enqueue rounds against a video whose
    /// generation always fails. With `MAX_GENERATION_ATTEMPTS == 3`, the
    /// video must be attempted on exactly the first 3 rounds and be absent
    /// from the 4th round's listing (attempts never incremented before an
    /// attempt, and never incremented on success, so there is no way to
    /// reach this state after fewer than 3 real attempts either).
    #[test]
    fn a_permanently_failing_video_is_attempted_exactly_three_times_then_excluded() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_dir = tempfile::tempdir().unwrap();
        insert_test_video(&db, "vid-doomed", "C:/videos/doomed.mp4");

        let ffmpeg = FakeFfmpegAdapter {
            duration: Ok(Some(120.0)),
            extract_result: Box::new(|_seek| {
                Err(FfmpegError::NonZeroExit {
                    status: 1,
                    stderr: "always fails".into(),
                })
            }),
            ..Default::default()
        };

        let mut was_eligible_by_round = Vec::new();
        for _ in 0..4 {
            let missing = list_videos_missing_thumbnails(&db, thumbs_dir.path()).unwrap();
            let eligible = missing.iter().any(|(id, _)| id == "vid-doomed");
            was_eligible_by_round.push(eligible);
            if eligible {
                let _ = generate_thumbnail_for_video(
                    &ffmpeg,
                    &db,
                    thumbs_dir.path(),
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
                "SELECT thumbnail_attempts FROM videos WHERE id = 'vid-doomed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(attempts, 3, "exactly 3 failed attempts, no more, no less");

        // Each of the 3 real attempts tries slot 0's computed seek and then
        // its 0s fallback (both fail here), then fail-fasts without
        // attempting slots 1-5 -- 3 attempts * 2 extract_thumbnail calls
        // each is an independent cross-check that exactly 3 (not 2 or 4)
        // generation attempts actually ran, and that fail-fast held across
        // every one of them.
        let calls = ffmpeg.calls.lock().unwrap();
        let extract_call_count = calls
            .iter()
            .filter(|c| matches!(c, FakeCall::ExtractThumbnail { .. }))
            .count();
        assert_eq!(extract_call_count, 6);
    }

    fn thumbnail_ready_flag(db: &Db, id: &str) -> i64 {
        let conn = db.writer.lock().unwrap();
        conn.query_row(
            "SELECT thumbnail_ready FROM videos WHERE id = ?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn success_marks_thumbnail_ready_in_the_db() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_dir = tempfile::tempdir().unwrap();
        insert_test_video(&db, "vid-ready", "C:/videos/movie.mp4");
        assert_eq!(
            thumbnail_ready_flag(&db, "vid-ready"),
            0,
            "must start out not-ready"
        );

        let ffmpeg = FakeFfmpegAdapter {
            duration: Ok(Some(120.0)),
            extract_result: Box::new(|_seek| Ok(())),
            ..Default::default()
        };

        generate_thumbnail_for_video(
            &ffmpeg,
            &db,
            thumbs_dir.path(),
            "vid-ready",
            Path::new("C:/videos/movie.mp4"),
        )
        .expect("generation should succeed");

        assert_eq!(
            thumbnail_ready_flag(&db, "vid-ready"),
            1,
            "a successful generation must set thumbnail_ready"
        );
    }

    #[test]
    fn failure_leaves_thumbnail_ready_at_zero() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_dir = tempfile::tempdir().unwrap();
        insert_test_video(&db, "vid-not-ready", "C:/videos/movie.mp4");

        let ffmpeg = FakeFfmpegAdapter {
            duration: Ok(Some(120.0)),
            extract_result: Box::new(|_seek| {
                Err(FfmpegError::NonZeroExit {
                    status: 1,
                    stderr: "corrupt file".into(),
                })
            }),
            ..Default::default()
        };

        let result = generate_thumbnail_for_video(
            &ffmpeg,
            &db,
            thumbs_dir.path(),
            "vid-not-ready",
            Path::new("C:/videos/movie.mp4"),
        );
        assert!(result.is_err());

        assert_eq!(
            thumbnail_ready_flag(&db, "vid-not-ready"),
            0,
            "a failed generation must not set thumbnail_ready"
        );
    }

    /// Set by the `trace_v2` callback installed in the regression test below
    /// whenever SQLite begins executing any statement whose text contains
    /// `UPDATE`. A plain top-level `static` because `Connection::trace_v2`'s
    /// callback type is a bare `fn(TraceEvent<'_>)` (no closure captures
    /// allowed).
    static SAW_UPDATE_STATEMENT: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    fn record_if_update(event: rusqlite::trace::TraceEvent<'_>) {
        if let rusqlite::trace::TraceEvent::Stmt(_, sql) = event {
            if sql.to_uppercase().contains("UPDATE") {
                SAW_UPDATE_STATEMENT.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }

    /// The performance regression guard: when every
    /// online video's file already exists on disk *and* its DB
    /// `thumbnail_ready` flag already agrees (`= 1`), this resume pass must
    /// not execute `mark_thumbnail_ready`'s `UPDATE` at all -- not even as a
    /// no-op write. Before this fix, that `UPDATE` (and the `db.writer` lock
    /// acquisition around it) ran unconditionally for every row whose file
    /// existed, regardless of what the DB already believed, which is exactly
    /// the "definitely-a-no-op, but still paid for" cost this test rules out.
    #[test]
    fn list_videos_missing_thumbnails_issues_no_update_when_the_ready_flag_is_already_current() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_dir = tempfile::tempdir().unwrap();
        insert_test_video(&db, "already-ready", "C:/videos/a.mp4");
        write_all_six_final_thumbnails(thumbs_dir.path(), "already-ready");
        {
            let conn = db.writer.lock().unwrap();
            queries::mark_thumbnail_ready(&conn, "already-ready").unwrap();
        }
        assert_eq!(thumbnail_ready_flag(&db, "already-ready"), 1);

        SAW_UPDATE_STATEMENT.store(false, std::sync::atomic::Ordering::SeqCst);
        {
            let conn = db.writer.lock().unwrap();
            conn.trace_v2(
                rusqlite::trace::TraceEventCodes::SQLITE_TRACE_STMT,
                Some(record_if_update),
            );
        }

        let missing = list_videos_missing_thumbnails(&db, thumbs_dir.path()).unwrap();

        assert!(missing.is_empty());
        assert!(
            !SAW_UPDATE_STATEMENT.load(std::sync::atomic::Ordering::SeqCst),
            "no UPDATE statement (i.e. no mark_thumbnail_ready call) should run \
             when the DB's thumbnail_ready flag already matches reality"
        );

        // Disarm the trace before the connection is reused by later tests
        // in the same process (a `fn` pointer trace callback has no way to
        // know which test installed it).
        let conn = db.writer.lock().unwrap();
        conn.trace_v2(rusqlite::trace::TraceEventCodes::empty(), None);
    }

    /// The backfill guard: a row whose thumbnail files are
    /// already on disk but whose `thumbnail_ready` flag is still 0 (e.g. a
    /// pre-migration-0008 row) must (a) be excluded from the "missing"
    /// result -- it already has all 6 files, so it never needed
    /// (re)generating -- and (b) come out of this call with its DB flag
    /// corrected to 1, without this function performing any extra
    /// filesystem stat beyond the ones it already does per row.
    #[test]
    fn list_videos_missing_thumbnails_backfills_a_stale_ready_flag_for_existing_files() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_dir = tempfile::tempdir().unwrap();
        insert_test_video(&db, "stale-flag", "C:/videos/a.mp4");
        assert_eq!(thumbnail_ready_flag(&db, "stale-flag"), 0);
        write_all_six_final_thumbnails(thumbs_dir.path(), "stale-flag");

        let missing = list_videos_missing_thumbnails(&db, thumbs_dir.path()).unwrap();

        assert!(
            missing.is_empty(),
            "a video whose files already all exist must not be reported as missing"
        );
        assert_eq!(
            thumbnail_ready_flag(&db, "stale-flag"),
            1,
            "the stale DB flag must be backfilled to 1 now that the files were observed to exist"
        );
    }

    #[test]
    fn probe_failure_is_not_fatal_and_falls_back_to_the_unknown_duration_seek_policy() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_dir = tempfile::tempdir().unwrap();
        insert_test_video(&db, "vid-5", "C:/videos/movie.mp4");

        let ffmpeg = FakeFfmpegAdapter {
            duration: Err(FfmpegError::Spawn("ffprobe crashed".into())),
            extract_result: Box::new(|seek_secs| {
                // thumbnail_seek_positions(None) == [1.0; 6]
                assert_eq!(seek_secs, 1.0);
                Ok(())
            }),
            ..Default::default()
        };

        generate_thumbnail_for_video(
            &ffmpeg,
            &db,
            thumbs_dir.path(),
            "vid-5",
            Path::new("C:/videos/movie.mp4"),
        )
        .expect("a probe failure alone should not fail generation");
        assert!(all_six_final_files_exist(thumbs_dir.path(), "vid-5"));
    }
}
