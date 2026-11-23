//! `skipped_files` rows (files kept out of the catalog) and promoting one to a regular video.

use gb_core::scan_pipeline::{NewSkippedFile, NewVideo};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, OptionalExtension};

use super::videos::insert_video;
use super::wb_import::import_wb_video_in_tx;

pub struct SkippedFileRow {
    pub id: i64,
    pub file_path: String,
    pub file_name: String,
    pub reason: String,
    pub detected_char: Option<String>,
    pub detected_at: String,
    /// JSON of `gb_core::wb_skipped_payload::WbSkippedPayload`; `None` for
    /// records that did not come from a .wb import.
    pub wb_payload: Option<String>,
}

/// Records a skipped file. Re-detecting the same path only refreshes
/// `detected_at`; in particular a row the user dismissed stays dismissed, which
/// is what keeps a re-import of a .wb file from resurrecting it (the .wb
/// import records rows without checking the file exists).
pub fn upsert_skipped_file(conn: &Connection, skipped: &NewSkippedFile) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO skipped_files (file_path, file_name, reason, detected_char)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(file_path) DO UPDATE SET detected_at = CURRENT_TIMESTAMP",
        params![
            skipped.file_path,
            skipped.file_name,
            skipped.reason,
            skipped.detected_char.to_string()
        ],
    )?;
    Ok(())
}

/// `upsert_skipped_file` for a .wb import, which also keeps the row's .wb data
/// (`wb_payload` JSON). Re-importing refreshes the payload with the latest
/// .wb contents; `dismissed` is still left alone.
pub fn upsert_skipped_file_with_wb_payload(
    conn: &Connection,
    skipped: &NewSkippedFile,
    wb_payload: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO skipped_files (file_path, file_name, reason, detected_char, wb_payload)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(file_path) DO UPDATE
         SET detected_at = CURRENT_TIMESTAMP, wb_payload = excluded.wb_payload",
        params![
            skipped.file_path,
            skipped.file_name,
            skipped.reason,
            skipped.detected_char.to_string(),
            wb_payload
        ],
    )?;
    Ok(())
}

/// Like `upsert_skipped_file`, but for a file a scan has just seen on disk:
/// it also un-dismisses the row, so a file that exists again is never left
/// hidden from the list.
pub fn upsert_skipped_file_found_on_disk(
    conn: &Connection,
    skipped: &NewSkippedFile,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO skipped_files (file_path, file_name, reason, detected_char)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(file_path) DO UPDATE SET detected_at = CURRENT_TIMESTAMP, dismissed = 0",
        params![
            skipped.file_path,
            skipped.file_name,
            skipped.reason,
            skipped.detected_char.to_string()
        ],
    )?;
    Ok(())
}

/// Maps a row selected as
/// `id, file_path, file_name, reason, detected_char, detected_at, wb_payload`.
fn skipped_row_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SkippedFileRow> {
    Ok(SkippedFileRow {
        id: r.get(0)?,
        file_path: r.get(1)?,
        file_name: r.get(2)?,
        reason: r.get(3)?,
        detected_char: r.get(4)?,
        detected_at: r.get(5)?,
        wb_payload: r.get(6)?,
    })
}

pub fn list_skipped_files(
    pool: &r2d2::Pool<SqliteConnectionManager>,
) -> anyhow::Result<Vec<SkippedFileRow>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT id, file_path, file_name, reason, detected_char, detected_at, wb_payload
         FROM skipped_files WHERE dismissed = 0 ORDER BY detected_at DESC",
    )?;
    let rows = stmt
        .query_map([], skipped_row_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Looks a row up regardless of `dismissed`.
pub fn get_skipped_file(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    id: i64,
) -> anyhow::Result<Option<SkippedFileRow>> {
    let conn = pool.get()?;
    let row = conn
        .query_row(
            "SELECT id, file_path, file_name, reason, detected_char, detected_at, wb_payload
             FROM skipped_files WHERE id = ?1",
            params![id],
            skipped_row_from_row,
        )
        .optional()?;
    Ok(row)
}

/// Removes one skipped-file record. Returns whether a row was deleted.
pub fn delete_skipped_file(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    let n = conn.execute("DELETE FROM skipped_files WHERE id = ?1", params![id])?;
    Ok(n > 0)
}

/// Hides one skipped-file record from the list without deleting it (see
/// `upsert_skipped_file`). Returns whether the row exists.
pub fn dismiss_skipped_file(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "UPDATE skipped_files SET dismissed = 1 WHERE id = ?1",
        params![id],
    )?;
    Ok(n > 0)
}

/// Result of `register_missing_skipped_as_video`.
#[derive(Debug, PartialEq, Eq)]
pub enum RegisterMissingOutcome {
    Registered,
    /// A video already occupies the path (compared case-insensitively, like
    /// the Windows file system); nothing was changed.
    PathTaken,
    /// The skipped record no longer exists; nothing was changed.
    RecordGone,
}

/// Registers `video` and hides the skipped record `skipped_id` in one
/// transaction, so a failure never leaves a registered video whose record is
/// still listed (or the reverse). With `wb`, the rating, readings and tags go
/// through the same path as a .wb import.
pub fn register_missing_skipped_as_video(
    conn: &mut Connection,
    skipped_id: i64,
    video: &NewVideo,
    wb: Option<&gb_core::wb_skipped_payload::WbSkippedPayload>,
) -> rusqlite::Result<RegisterMissingOutcome> {
    let tx = conn.transaction()?;
    let record_exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM skipped_files WHERE id = ?1)",
        params![skipped_id],
        |r| r.get(0),
    )?;
    if !record_exists {
        return Ok(RegisterMissingOutcome::RecordGone);
    }
    let taken: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM videos WHERE lower(file_path) = lower(?1))",
        params![video.file_path],
        |r| r.get(0),
    )?;
    if taken {
        return Ok(RegisterMissingOutcome::PathTaken);
    }
    match wb {
        Some(p) => {
            import_wb_video_in_tx(&tx, video, p.rating, &p.kana, &p.roma, &p.tags)?;
        }
        None => insert_video(&tx, video)?,
    }
    dismiss_skipped_file(&tx, skipped_id)?;
    tx.commit()?;
    Ok(RegisterMissingOutcome::Registered)
}

#[cfg(test)]
mod skipped_file_tests {
    use super::*;
    use crate::db::test_support::init_temp_db;
    use crate::db::Db;

    fn skipped(path: &str) -> NewSkippedFile {
        NewSkippedFile {
            file_path: path.to_string(),
            file_name: "a\u{E000}.mp4".to_string(),
            reason: "machine_dependent_char",
            detected_char: '\u{E000}',
        }
    }

    fn id_of(db: &Db, path: &str) -> i64 {
        list_skipped_files(&db.read_pool)
            .unwrap()
            .into_iter()
            .find(|r| r.file_path == path)
            .unwrap()
            .id
    }

    #[test]
    fn get_returns_the_row_or_none() {
        let (_d, db) = init_temp_db();
        upsert_skipped_file(&db.writer.lock().unwrap(), &skipped("C:/v/a.mp4")).unwrap();
        let id = id_of(&db, "C:/v/a.mp4");

        let row = get_skipped_file(&db.read_pool, id).unwrap().unwrap();
        assert_eq!(row.file_path, "C:/v/a.mp4");
        assert!(get_skipped_file(&db.read_pool, id + 1000)
            .unwrap()
            .is_none());
    }

    #[test]
    fn delete_removes_only_the_target_row() {
        let (_d, db) = init_temp_db();
        {
            let conn = db.writer.lock().unwrap();
            upsert_skipped_file(&conn, &skipped("C:/v/a.mp4")).unwrap();
            upsert_skipped_file(&conn, &skipped("C:/v/b.mp4")).unwrap();
        }
        let id = id_of(&db, "C:/v/a.mp4");

        assert!(delete_skipped_file(&db.writer.lock().unwrap(), id).unwrap());

        let left = list_skipped_files(&db.read_pool).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].file_path, "C:/v/b.mp4");
    }

    #[test]
    fn delete_of_a_missing_id_reports_false() {
        let (_d, db) = init_temp_db();
        assert!(!delete_skipped_file(&db.writer.lock().unwrap(), 12345).unwrap());
    }

    fn dismissed_flag(db: &Db, path: &str) -> i64 {
        db.read_pool
            .get()
            .unwrap()
            .query_row(
                "SELECT dismissed FROM skipped_files WHERE file_path = ?1",
                params![path],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn wb_payload_is_stored_and_refreshed_by_a_reimport_without_undismissing() {
        let (_d, db) = init_temp_db();
        let conn = db.writer.lock().unwrap();
        upsert_skipped_file_with_wb_payload(&conn, &skipped("C:/v/a.mp4"), "{\"v\":1}").unwrap();
        drop(conn);
        let id = id_of(&db, "C:/v/a.mp4");
        dismiss_skipped_file(&db.writer.lock().unwrap(), id).unwrap();

        upsert_skipped_file_with_wb_payload(
            &db.writer.lock().unwrap(),
            &skipped("C:/v/a.mp4"),
            "{\"v\":2}",
        )
        .unwrap();

        let row = get_skipped_file(&db.read_pool, id).unwrap().unwrap();
        assert_eq!(row.wb_payload.as_deref(), Some("{\"v\":2}"));
        assert_eq!(dismissed_flag(&db, "C:/v/a.mp4"), 1);
    }

    #[test]
    fn a_plain_upsert_leaves_the_payload_and_scan_rows_have_none() {
        let (_d, db) = init_temp_db();
        upsert_skipped_file(&db.writer.lock().unwrap(), &skipped("C:/v/b.mp4")).unwrap();
        let id = id_of(&db, "C:/v/b.mp4");
        assert_eq!(
            get_skipped_file(&db.read_pool, id)
                .unwrap()
                .unwrap()
                .wb_payload,
            None
        );

        upsert_skipped_file_with_wb_payload(
            &db.writer.lock().unwrap(),
            &skipped("C:/v/b.mp4"),
            "{\"v\":1}",
        )
        .unwrap();
        upsert_skipped_file(&db.writer.lock().unwrap(), &skipped("C:/v/b.mp4")).unwrap();

        let row = get_skipped_file(&db.read_pool, id).unwrap().unwrap();
        assert_eq!(row.wb_payload.as_deref(), Some("{\"v\":1}"));
    }

    #[test]
    fn new_rows_are_not_dismissed() {
        let (_d, db) = init_temp_db();
        upsert_skipped_file(&db.writer.lock().unwrap(), &skipped("C:/v/a.mp4")).unwrap();
        assert_eq!(dismissed_flag(&db, "C:/v/a.mp4"), 0);
    }

    #[test]
    fn dismiss_hides_only_the_target_row_but_keeps_it_gettable() {
        let (_d, db) = init_temp_db();
        {
            let conn = db.writer.lock().unwrap();
            upsert_skipped_file(&conn, &skipped("C:/v/a.mp4")).unwrap();
            upsert_skipped_file(&conn, &skipped("C:/v/b.mp4")).unwrap();
        }
        let id = id_of(&db, "C:/v/a.mp4");

        assert!(dismiss_skipped_file(&db.writer.lock().unwrap(), id).unwrap());

        let left = list_skipped_files(&db.read_pool).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].file_path, "C:/v/b.mp4");
        assert!(get_skipped_file(&db.read_pool, id).unwrap().is_some());
    }

    #[test]
    fn dismiss_of_an_unknown_id_reports_false() {
        let (_d, db) = init_temp_db();
        assert!(!dismiss_skipped_file(&db.writer.lock().unwrap(), 12345).unwrap());
    }

    #[test]
    fn a_plain_upsert_does_not_resurrect_a_dismissed_row() {
        let (_d, db) = init_temp_db();
        upsert_skipped_file(&db.writer.lock().unwrap(), &skipped("C:/v/a.mp4")).unwrap();
        let id = id_of(&db, "C:/v/a.mp4");
        dismiss_skipped_file(&db.writer.lock().unwrap(), id).unwrap();

        upsert_skipped_file(&db.writer.lock().unwrap(), &skipped("C:/v/a.mp4")).unwrap();

        assert!(list_skipped_files(&db.read_pool).unwrap().is_empty());
        assert_eq!(dismissed_flag(&db, "C:/v/a.mp4"), 1);
    }

    #[test]
    fn scan_style_upsert_brings_a_dismissed_row_back() {
        let (_d, db) = init_temp_db();
        upsert_skipped_file(&db.writer.lock().unwrap(), &skipped("C:/v/a.mp4")).unwrap();
        let id = id_of(&db, "C:/v/a.mp4");
        dismiss_skipped_file(&db.writer.lock().unwrap(), id).unwrap();

        upsert_skipped_file_found_on_disk(&db.writer.lock().unwrap(), &skipped("C:/v/a.mp4"))
            .unwrap();

        let rows = list_skipped_files(&db.read_pool).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
    }

    #[test]
    fn dismissal_survives_reopening_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        {
            let db = crate::db::init(&path).unwrap();
            upsert_skipped_file(&db.writer.lock().unwrap(), &skipped("C:/v/a.mp4")).unwrap();
            let id = id_of(&db, "C:/v/a.mp4");
            dismiss_skipped_file(&db.writer.lock().unwrap(), id).unwrap();
        }

        let db = crate::db::init(&path).unwrap();
        assert!(list_skipped_files(&db.read_pool).unwrap().is_empty());
    }
}
