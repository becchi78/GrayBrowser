//! Thumbnails in the thumbnail folder that match no cataloged video.

use rusqlite::{params, Connection};

/// One legacy thumbnail file a .wb import left unlinked, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmatchedThumbnailRow {
    pub folder_path: String,
    pub file_name: String,
    /// `gb_core::wb_import::UnmatchedReason::as_db_str` text; kept as text
    /// here so an unrecognised value in the database cannot fail a listing.
    pub kind: String,
    /// The `thum_size` text the .wb stated (see `ThumbLayout::to_thum_size`),
    /// `None` when it stated none unambiguously.
    pub thum_layout: Option<String>,
}

/// A file to record for [`replace_unmatched_thumbnails`]; the folder is the
/// function's own argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewUnmatchedThumbnail {
    pub file_name: String,
    pub kind: String,
    pub thum_layout: Option<String>,
}

/// Makes `rows` the complete record for `folder`: the folder's existing rows
/// are deleted and the new ones inserted in one transaction, so a failure
/// leaves the previous record intact and a repeat import never duplicates.
/// Other folders' rows are untouched. A file name repeated in `rows` keeps its
/// last entry rather than failing the whole replacement on the primary key.
pub fn replace_unmatched_thumbnails(
    conn: &mut Connection,
    folder: &str,
    rows: &[NewUnmatchedThumbnail],
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM wb_unmatched_thumbnails WHERE folder_path = ?1",
        params![folder],
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT OR REPLACE INTO wb_unmatched_thumbnails (folder_path, file_name, kind, thum_layout) \
             VALUES (?1, ?2, ?3, ?4)",
        )?;
        for row in rows {
            stmt.execute(params![folder, row.file_name, row.kind, row.thum_layout])?;
        }
    }
    tx.commit()
}

/// Every recorded unmatched thumbnail, by folder then file name.
pub fn list_unmatched_thumbnails(
    conn: &Connection,
) -> rusqlite::Result<Vec<UnmatchedThumbnailRow>> {
    let mut stmt = conn.prepare(
        "SELECT folder_path, file_name, kind, thum_layout FROM wb_unmatched_thumbnails \
         ORDER BY folder_path, file_name",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(UnmatchedThumbnailRow {
            folder_path: r.get(0)?,
            file_name: r.get(1)?,
            kind: r.get(2)?,
            thum_layout: r.get(3)?,
        })
    })?;
    rows.collect()
}

/// Removes one record; `false` when there was none.
pub fn delete_unmatched_thumbnail(
    conn: &Connection,
    folder: &str,
    file_name: &str,
) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "DELETE FROM wb_unmatched_thumbnails WHERE folder_path = ?1 AND file_name = ?2",
        params![folder, file_name],
    )?;
    Ok(n > 0)
}

#[cfg(test)]
mod unmatched_thumbnail_tests {
    use super::*;
    use crate::db::test_support::init_temp_db;

    fn new_row(name: &str, kind: &str, layout: Option<&str>) -> NewUnmatchedThumbnail {
        NewUnmatchedThumbnail {
            file_name: name.to_string(),
            kind: kind.to_string(),
            thum_layout: layout.map(str::to_string),
        }
    }

    fn row(folder: &str, name: &str, kind: &str, layout: Option<&str>) -> UnmatchedThumbnailRow {
        UnmatchedThumbnailRow {
            folder_path: folder.to_string(),
            file_name: name.to_string(),
            kind: kind.to_string(),
            thum_layout: layout.map(str::to_string),
        }
    }

    #[test]
    fn replace_inserts_and_lists_ordered_by_folder_then_name() {
        let (_d, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();
        replace_unmatched_thumbnails(
            &mut conn,
            "B",
            &[new_row("z.#.jpg", "empty_hash", Some("200x150x6x1"))],
        )
        .unwrap();
        replace_unmatched_thumbnails(
            &mut conn,
            "A",
            &[
                new_row("b.jpg", "no_matching_video", None),
                new_row("a.jpg", "no_matching_video", None),
            ],
        )
        .unwrap();

        assert_eq!(
            list_unmatched_thumbnails(&conn).unwrap(),
            vec![
                row("A", "a.jpg", "no_matching_video", None),
                row("A", "b.jpg", "no_matching_video", None),
                row("B", "z.#.jpg", "empty_hash", Some("200x150x6x1")),
            ]
        );
    }

    #[test]
    fn replace_rebuilds_only_its_own_folder() {
        let (_d, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();
        replace_unmatched_thumbnails(&mut conn, "A", &[new_row("old.jpg", "empty_hash", None)])
            .unwrap();
        replace_unmatched_thumbnails(&mut conn, "B", &[new_row("keep.jpg", "empty_hash", None)])
            .unwrap();

        replace_unmatched_thumbnails(&mut conn, "A", &[new_row("new.jpg", "empty_hash", None)])
            .unwrap();
        assert_eq!(
            list_unmatched_thumbnails(&conn).unwrap(),
            vec![
                row("A", "new.jpg", "empty_hash", None),
                row("B", "keep.jpg", "empty_hash", None),
            ]
        );

        replace_unmatched_thumbnails(&mut conn, "A", &[]).unwrap();
        assert_eq!(
            list_unmatched_thumbnails(&conn).unwrap(),
            vec![row("B", "keep.jpg", "empty_hash", None)]
        );
    }

    #[test]
    fn replace_with_the_same_rows_twice_does_not_duplicate() {
        let (_d, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();
        let rows = [new_row("a.jpg", "empty_hash", Some("1x1x6x1"))];
        replace_unmatched_thumbnails(&mut conn, "A", &rows).unwrap();
        replace_unmatched_thumbnails(&mut conn, "A", &rows).unwrap();
        assert_eq!(list_unmatched_thumbnails(&conn).unwrap().len(), 1);
    }

    #[test]
    fn replace_with_a_repeated_file_name_keeps_the_last_entry() {
        let (_d, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();
        replace_unmatched_thumbnails(
            &mut conn,
            "A",
            &[
                new_row("a.jpg", "empty_hash", None),
                new_row("a.jpg", "no_matching_video", None),
            ],
        )
        .unwrap();
        assert_eq!(
            list_unmatched_thumbnails(&conn).unwrap(),
            vec![row("A", "a.jpg", "no_matching_video", None)]
        );
    }

    #[test]
    fn a_failed_replace_keeps_the_previous_rows() {
        let (_d, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();
        replace_unmatched_thumbnails(&mut conn, "A", &[new_row("old.jpg", "empty_hash", None)])
            .unwrap();
        // A trigger that rejects one file name makes the second insert fail
        // after the delete and the first insert already ran.
        conn.execute_batch(
            "CREATE TEMP TRIGGER reject BEFORE INSERT ON wb_unmatched_thumbnails \
             WHEN NEW.file_name = 'bad.jpg' BEGIN SELECT RAISE(ABORT, 'rejected'); END;",
        )
        .unwrap();

        let result = replace_unmatched_thumbnails(
            &mut conn,
            "A",
            &[
                new_row("good.jpg", "empty_hash", None),
                new_row("bad.jpg", "empty_hash", None),
            ],
        );
        assert!(result.is_err());
        assert_eq!(
            list_unmatched_thumbnails(&conn).unwrap(),
            vec![row("A", "old.jpg", "empty_hash", None)],
            "the delete must roll back with the failed insert"
        );
    }

    #[test]
    fn list_is_empty_when_nothing_is_recorded() {
        let (_d, db) = init_temp_db();
        let conn = db.writer.lock().unwrap();
        assert!(list_unmatched_thumbnails(&conn).unwrap().is_empty());
    }

    #[test]
    fn delete_removes_one_row_and_reports_whether_it_existed() {
        let (_d, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();
        replace_unmatched_thumbnails(
            &mut conn,
            "A",
            &[
                new_row("a.jpg", "empty_hash", None),
                new_row("b.jpg", "empty_hash", None),
            ],
        )
        .unwrap();
        replace_unmatched_thumbnails(&mut conn, "B", &[new_row("a.jpg", "empty_hash", None)])
            .unwrap();

        assert!(delete_unmatched_thumbnail(&conn, "A", "a.jpg").unwrap());
        assert!(!delete_unmatched_thumbnail(&conn, "A", "a.jpg").unwrap());
        assert!(!delete_unmatched_thumbnail(&conn, "C", "b.jpg").unwrap());
        assert_eq!(
            list_unmatched_thumbnails(&conn).unwrap(),
            vec![
                row("A", "b.jpg", "empty_hash", None),
                row("B", "a.jpg", "empty_hash", None),
            ]
        );
    }
}
