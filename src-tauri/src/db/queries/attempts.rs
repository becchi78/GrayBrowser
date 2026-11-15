//! Per-video thumbnail/metadata retry counters, `thumbnail_ready` flags and the exhausted-retry listings.

use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection};

// --- Generation retry-limit tracking (gb_core::retry's pure classification)
// ----------------------------------------------------------------------
//
// `thumbnail_attempts`/`metadata_attempts` (migration 0007) count *failed*
// attempts only -- a successful generation is recorded elsewhere (a
// `thumbnails/[id].webp` file existing, `probed_at` being non-NULL) and never
// increments these columns. Whether a given attempts count still permits an
// automatic retry is `gb_core::retry::is_eligible_for_automatic_retry`'s job,
// not this module's -- these functions only read/write the raw counter.

/// Increments `thumbnail_attempts` by 1 for `video_id`. Called by
/// `thumbnail::worker` immediately after a generation attempt is
/// determined to have failed.
pub fn increment_thumbnail_attempts(conn: &Connection, video_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET thumbnail_attempts = thumbnail_attempts + 1 WHERE id = ?1",
        params![video_id],
    )?;
    Ok(())
}

/// Gives back one `thumbnail_attempts` count (never below 0). For a failure
/// that turned out to be the folder being unreachable rather than the video
/// being bad, so it must not count against the video's retry budget.
pub fn refund_thumbnail_attempt(conn: &Connection, video_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET thumbnail_attempts = MAX(thumbnail_attempts - 1, 0) WHERE id = ?1",
        params![video_id],
    )?;
    Ok(())
}

/// `refund_thumbnail_attempt`'s counterpart for `metadata_attempts`.
pub fn refund_metadata_attempt(conn: &Connection, video_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET metadata_attempts = MAX(metadata_attempts - 1, 0) WHERE id = ?1",
        params![video_id],
    )?;
    Ok(())
}

/// Increments `metadata_attempts` by 1 for `video_id`. Called by
/// `metadata::worker` immediately after a probe attempt is
/// determined to have failed.
pub fn increment_metadata_attempts(conn: &Connection, video_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET metadata_attempts = metadata_attempts + 1 WHERE id = ?1",
        params![video_id],
    )?;
    Ok(())
}

/// Sets `thumbnail_ready = 1` for `video_id` (migration 0008).
/// Called by `thumbnail::worker::generate_thumbnail_for_video` right after a
/// successful generation, and by `list_videos_missing_thumbnails`'s resume
/// pass as a backfill for rows whose file already exists but whose flag
/// hadn't been set yet. The `AND thumbnail_ready = 0` guard makes the common
/// case (flag already set) a no-op write rather than an unconditional
/// `UPDATE` on every call.
pub fn mark_thumbnail_ready(conn: &Connection, video_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET thumbnail_ready = 1 WHERE id = ?1 AND thumbnail_ready = 0",
        params![video_id],
    )?;
    Ok(())
}

/// Clears the ready flag of a video whose thumbnail files were removed, so the
/// flag never claims files that are gone.
pub fn clear_thumbnail_ready(conn: &Connection, video_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET thumbnail_ready = 0 WHERE id = ?1",
        params![video_id],
    )?;
    Ok(())
}

/// `mark_thumbnail_ready` for every id in `ids` inside one transaction, so a
/// bulk link pays for one commit instead of one per row. A failing row is
/// logged and skipped rather than aborting the batch: the thumbnail files are
/// already in place, so the other rows must still become ready. Ids that match
/// no row (or are already ready) are no-ops; an empty slice does not touch the
/// database.
pub fn mark_thumbnails_ready(conn: &mut Connection, ids: &[String]) {
    if ids.is_empty() {
        return;
    }
    let tx = match conn.transaction() {
        Ok(tx) => tx,
        Err(e) => {
            log::warn!("failed to begin the thumbnail-ready batch: {e}");
            return;
        }
    };
    {
        let mut stmt = match tx
            .prepare("UPDATE videos SET thumbnail_ready = 1 WHERE id = ?1 AND thumbnail_ready = 0")
        {
            Ok(stmt) => stmt,
            Err(e) => {
                log::warn!("failed to prepare the thumbnail-ready update: {e}");
                return;
            }
        };
        for id in ids {
            if let Err(e) = stmt.execute(params![id]) {
                log::warn!("failed to mark thumbnail ready for video {id}: {e}");
            }
        }
    }
    if let Err(e) = tx.commit() {
        log::warn!("failed to commit the thumbnail-ready batch: {e}");
    }
}

/// Returns everything derived from the file's content (thumbnail state,
/// duration, probed properties and their retry counters) to the "not yet
/// generated/probed" state, for when the content itself changed. The probed
/// properties are cleared too, rather than left to be overwritten, so a
/// re-probe that fails does not leave the previous content's values next
/// to a NULL `probed_at`.
pub fn reset_video_derived_data(conn: &Connection, video_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos
         SET thumbnail_ready = 0, thumbnail_attempts = 0,
             duration = NULL, probed_at = NULL, metadata_attempts = 0,
             width = NULL, height = NULL, video_codec = NULL,
             audio_codec = NULL, bitrate = NULL, fps = NULL
         WHERE id = ?1",
        params![video_id],
    )?;
    Ok(())
}

/// Resets `thumbnail_attempts` back to 0 for `video_id`. Intended for a
/// manual "retry" action (surfaced in the UI) that gives an exhausted video
/// another chance at automatic generation.
pub fn reset_thumbnail_attempts(conn: &Connection, video_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET thumbnail_attempts = 0 WHERE id = ?1",
        params![video_id],
    )?;
    Ok(())
}

/// Resets `thumbnail_attempts` to 0 for every id in `ids` in one transaction,
/// so a bulk regeneration either clears all of them or none. Ids that match no
/// row are ignored; an empty slice does not touch the database.
pub fn reset_thumbnail_attempts_for(conn: &mut Connection, ids: &[String]) -> rusqlite::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare("UPDATE videos SET thumbnail_attempts = 0 WHERE id = ?1")?;
        for id in ids {
            stmt.execute(params![id])?;
        }
    }
    tx.commit()
}

/// Resets `metadata_attempts` back to 0 for `video_id`. Intended for a manual
/// "retry" action (surfaced in the UI) that gives an exhausted video another
/// chance at automatic probing.
pub fn reset_metadata_attempts(conn: &Connection, video_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE videos SET metadata_attempts = 0 WHERE id = ?1",
        params![video_id],
    )?;
    Ok(())
}

/// Resets `metadata_attempts` to 0 for every id in `ids` in one transaction,
/// so a bulk re-probe either clears all of them or none. Ids that match no
/// row are ignored; an empty slice does not touch the database.
pub fn reset_metadata_attempts_for(conn: &mut Connection, ids: &[String]) -> rusqlite::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare("UPDATE videos SET metadata_attempts = 0 WHERE id = ?1")?;
        for id in ids {
            stmt.execute(params![id])?;
        }
    }
    tx.commit()
}

/// `(id, file_path, thumbnail_attempts, thumbnail_ready)` for every online
/// video -- lets `thumbnail::worker::list_videos_missing_thumbnails`
/// skip rows `gb_core::retry::is_eligible_for_automatic_retry`
/// deems exhausted without a second query per row. `thumbnail_ready`
/// additionally lets that caller skip its backfill write
/// (`mark_thumbnail_ready`) for rows the DB already knows are ready, instead
/// of calling it unconditionally on every row whose file happens to exist.
pub fn list_online_video_paths_with_thumbnail_attempts(
    pool: &r2d2::Pool<SqliteConnectionManager>,
) -> anyhow::Result<Vec<(String, String, i64, bool)>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, file_path, thumbnail_attempts, thumbnail_ready FROM videos WHERE status = 'online'",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// `(id, file_path, metadata_attempts)` for every online video that hasn't
/// been probed for metadata yet -- mirrors
/// `list_online_video_paths_with_thumbnail_attempts` above, consumed by
/// `metadata::worker::list_videos_missing_metadata` the same way.
pub fn list_videos_missing_metadata_with_attempts(
    pool: &r2d2::Pool<SqliteConnectionManager>,
) -> anyhow::Result<Vec<(String, String, i64)>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, file_path, metadata_attempts FROM videos
         WHERE status = 'online' AND probed_at IS NULL",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// One row of `list_videos_with_exhausted_thumbnail_attempts`'s result --
/// enough for a UI list (which file, how many attempts) without
/// pulling in the full `VideoRow`/`VIDEO_COLUMNS` payload, same lightweight
/// intent as `SkippedFileRow`.
pub struct ExhaustedThumbnailRow {
    pub id: String,
    pub file_path: String,
    pub file_name: String,
    pub thumbnail_attempts: i64,
}

/// `status='online'` videos whose `thumbnail_attempts` has reached (or
/// somehow exceeded) `gb_core::retry::MAX_GENERATION_ATTEMPTS` -- i.e. every
/// video `gb_core::retry::classify_retry_status` would call `Exhausted` for
/// the thumbnail pipeline. For the "these files could not get a
/// thumbnail" notification panel.
pub fn list_videos_with_exhausted_thumbnail_attempts(
    pool: &r2d2::Pool<SqliteConnectionManager>,
) -> anyhow::Result<Vec<ExhaustedThumbnailRow>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, file_path, file_name, thumbnail_attempts FROM videos
         WHERE status = 'online' AND thumbnail_attempts >= ?1
         ORDER BY file_path",
    )?;
    let rows = stmt
        .query_map(
            params![gb_core::retry::MAX_GENERATION_ATTEMPTS as i64],
            |r| {
                Ok(ExhaustedThumbnailRow {
                    id: r.get(0)?,
                    file_path: r.get(1)?,
                    file_name: r.get(2)?,
                    thumbnail_attempts: r.get(3)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// One row of `list_videos_with_exhausted_metadata_attempts`'s result,
/// mirroring `ExhaustedThumbnailRow` above for the metadata pipeline.
pub struct ExhaustedMetadataRow {
    pub id: String,
    pub file_path: String,
    pub file_name: String,
    pub metadata_attempts: i64,
}

/// `status='online'` videos whose `metadata_attempts` has reached (or somehow
/// exceeded) `gb_core::retry::MAX_GENERATION_ATTEMPTS` -- the metadata
/// pipeline's counterpart to `list_videos_with_exhausted_thumbnail_attempts`.
pub fn list_videos_with_exhausted_metadata_attempts(
    pool: &r2d2::Pool<SqliteConnectionManager>,
) -> anyhow::Result<Vec<ExhaustedMetadataRow>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, file_path, file_name, metadata_attempts FROM videos
         WHERE status = 'online' AND metadata_attempts >= ?1
         ORDER BY file_path",
    )?;
    let rows = stmt
        .query_map(
            params![gb_core::retry::MAX_GENERATION_ATTEMPTS as i64],
            |r| {
                Ok(ExhaustedMetadataRow {
                    id: r.get(0)?,
                    file_path: r.get(1)?,
                    file_name: r.get(2)?,
                    metadata_attempts: r.get(3)?,
                })
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod mark_thumbnails_ready_tests {
    use super::*;
    use crate::db::test_support::{init_temp_db, insert_test_video};
    use crate::db::Db;

    fn ready_flag(db: &Db, id: &str) -> i64 {
        db.writer
            .lock()
            .unwrap()
            .query_row(
                "SELECT thumbnail_ready FROM videos WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn every_listed_row_becomes_ready_and_others_are_untouched() {
        let (_d, db) = init_temp_db();
        for (id, path) in [("a", "C:/a.mp4"), ("b", "C:/b.mp4"), ("c", "C:/c.mp4")] {
            insert_test_video(&db, id, path);
        }

        mark_thumbnails_ready(&mut db.writer.lock().unwrap(), &ids(&["a", "b"]));

        assert_eq!(ready_flag(&db, "a"), 1);
        assert_eq!(ready_flag(&db, "b"), 1);
        assert_eq!(ready_flag(&db, "c"), 0);
    }

    #[test]
    fn an_already_ready_row_is_not_rewritten() {
        let (_d, db) = init_temp_db();
        insert_test_video(&db, "a", "C:/a.mp4");
        insert_test_video(&db, "b", "C:/b.mp4");
        mark_thumbnail_ready(&db.writer.lock().unwrap(), "a").unwrap();

        let before: i64 = {
            let conn = db.writer.lock().unwrap();
            conn.query_row("SELECT total_changes()", [], |r| r.get(0))
                .unwrap()
        };
        mark_thumbnails_ready(&mut db.writer.lock().unwrap(), &ids(&["a"]));
        let after: i64 = {
            let conn = db.writer.lock().unwrap();
            conn.query_row("SELECT total_changes()", [], |r| r.get(0))
                .unwrap()
        };

        assert_eq!(before, after, "the thumbnail_ready = 0 guard must hold");
        assert_eq!(ready_flag(&db, "a"), 1);
        assert_eq!(ready_flag(&db, "b"), 0);
    }

    #[test]
    fn an_unknown_id_does_not_stop_the_others() {
        let (_d, db) = init_temp_db();
        insert_test_video(&db, "a", "C:/a.mp4");
        insert_test_video(&db, "b", "C:/b.mp4");

        mark_thumbnails_ready(
            &mut db.writer.lock().unwrap(),
            &ids(&["a", "no-such-id", "b"]),
        );

        assert_eq!(ready_flag(&db, "a"), 1);
        assert_eq!(ready_flag(&db, "b"), 1);
    }

    #[test]
    fn an_empty_batch_is_a_no_op() {
        let (_d, db) = init_temp_db();
        insert_test_video(&db, "a", "C:/a.mp4");

        mark_thumbnails_ready(&mut db.writer.lock().unwrap(), &[]);

        assert_eq!(ready_flag(&db, "a"), 0);
    }
}
