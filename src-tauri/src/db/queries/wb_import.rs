//! Importing a video from a `.wb` file together with its tags.

use gb_core::scan_pipeline::NewVideo;
use rusqlite::{params, Connection, Transaction};

use super::tags::{get_or_create_tag, TagRow};
use super::videos::insert_video_row;

// --- `.wb` import ------------------------------------------------------

/// Outcome of `import_wb_video`, distinguishing "this `.wb` row created a new
/// `videos` row" from "the `file_path` was already registered, so the whole
/// call was a no-op" -- the import log needs to tally both counts
/// separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WbImportOutcome {
    /// A new `videos` row was inserted and `tags_assigned` `video_tags`
    /// associations were written for it. Counts every input `tags` element
    /// that was actually assigned (duplicates within the slice each count,
    /// since `ON CONFLICT(video_id, tag_id) DO NOTHING` makes re-assigning
    /// the same tag harmless) -- except an element that normalizes to an
    /// empty tag name (see `find_or_create_tag_folding_case`), which is
    /// skipped and therefore not counted here.
    Inserted { tags_assigned: usize },
    /// `file_path` was already registered (e.g. a prior scan, or a repeat
    /// `.wb` import run). Nothing was written -- in particular, rating/kana/
    /// roma/tags are left exactly as they were, so a user edit is
    /// never clobbered by re-running the legacy import.
    Skipped,
}

/// Error from `find_or_create_tag_folding_case`'s "create" path. Kept
/// private/minimal -- it only exists to let `import_wb_video`'s tag loop
/// tell "this raw `.wb` tag string can't become a valid tag name, skip just
/// this one" apart from "a real SQLite failure happened, roll back the whole
/// transaction".
#[derive(Debug)]
enum WbTagResolutionError {
    /// `gb_core::tags::normalize_tag_name` rejected `display_name_if_new`
    /// (width-folds to empty/whitespace-only). `wb_import::split_tags`
    /// already trims and drops blank lines with Rust's own Unicode-aware
    /// `str::trim` -- which already strips the full-width space
    /// (`\u{3000}`) `normalize_tag_name`'s width-fold would otherwise turn
    /// into one -- so this should not occur on real `.wb` data; kept as a
    /// defensive case rather than a `debug_assert`/panic so one malformed
    /// tag can never take down an otherwise-valid video import.
    Empty,
    Sqlite(rusqlite::Error),
}

impl From<rusqlite::Error> for WbTagResolutionError {
    fn from(e: rusqlite::Error) -> Self {
        WbTagResolutionError::Sqlite(e)
    }
}

/// Finds an existing tag whose name folds (via `gb_core::wb_import::wb_tag_merge_key`)
/// to `merge_key`, or creates one named `display_name_if_new` (after running
/// it through `gb_core::tags::normalize_tag_name`) if none exists.
///
/// Scans every row in `tags` (via `tx`, so it sees any tag rows this same
/// transaction has already created) rather than doing the fold in SQL --
/// real data tops out at a few hundred tags, so an O(n) scan per call is not
/// worth indexing or caching for. The *first* matching existing row (by
/// `tags` table order) wins so import_wb_video's outcome is deterministic;
/// no new `tags` row is created when a fold match exists.
///
/// The "create" path normalizes `display_name_if_new` first (width-fold +
/// trim, no case-fold) so it upholds the invariant that every
/// `tags.name` is already width-folded -- otherwise a `.wb`-only full-width
/// tag (e.g. "Ａction") would be stored as-is, and a later manual tag entry
/// of the half-width "Action" (normalized by `normalize_tag_name` the same
/// way user-typed tags always are) would fail to match it on `UNIQUE(name)`
/// and create a duplicate row, defeating the point of this function.
fn find_or_create_tag_folding_case(
    tx: &Transaction,
    merge_key: &str,
    display_name_if_new: &str,
) -> Result<TagRow, WbTagResolutionError> {
    let mut stmt = tx.prepare("SELECT id, name FROM tags")?;
    let existing = stmt
        .query_map([], |r| {
            Ok(TagRow {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);

    if let Some(found) = existing
        .into_iter()
        .find(|row| gb_core::wb_import::wb_tag_merge_key(&row.name) == merge_key)
    {
        return Ok(found);
    }

    let normalized = gb_core::tags::normalize_tag_name(display_name_if_new)
        .map_err(|_| WbTagResolutionError::Empty)?;
    Ok(get_or_create_tag(tx, &normalized)?)
}

/// Imports one `.wb` video (the parsed `ImportCandidate`, minus the caller-
/// assembled `NewVideo`) in a single transaction, mirroring
/// `assign_tag_to_video`'s app-layer referential-integrity pattern:
///
/// 1. `INSERT INTO videos ... ON CONFLICT(file_path) DO NOTHING` -- the same
///    idempotency `insert_video` already relies on for repeat scans, reused
///    here so re-running a `.wb` import is always safe.
/// 2. If that insert changed 0 rows, the video was already registered --
///    commit immediately (nothing else was touched) and report `Skipped`,
///    deliberately *not* overwriting rating/kana/roma/tags so a user
///    edit always wins over stale `.wb` data.
/// 3. Otherwise (1 row inserted): write `rating`/`kana`/`roma`, then for each
///    raw `.wb` tag string, fold it via `wb_tag_merge_key` through
///    `find_or_create_tag_folding_case` and assign it
///    (`ON CONFLICT(video_id, tag_id) DO NOTHING`, same as
///    `assign_tag_to_video`).
///
/// Any error at any step drops `tx` uncommitted, rolling back the entire
/// video row, rating/kana/roma write, and every tag/assignment attempted so
/// far -- no orphaned `videos` or `video_tags` rows are ever left behind.
pub fn import_wb_video(
    conn: &mut Connection,
    video: &NewVideo,
    rating: u8,
    kana: &str,
    roma: &str,
    tags: &[String],
) -> rusqlite::Result<WbImportOutcome> {
    let tx = conn.transaction()?;
    let outcome = import_wb_video_in_tx(&tx, video, rating, kana, roma, tags)?;
    tx.commit()?;
    Ok(outcome)
}

/// The body of `import_wb_video`, run inside the caller's transaction so a
/// caller can make further writes atomic with the import.
pub(super) fn import_wb_video_in_tx(
    tx: &Transaction,
    video: &NewVideo,
    rating: u8,
    kana: &str,
    roma: &str,
    tags: &[String],
) -> rusqlite::Result<WbImportOutcome> {
    let inserted = insert_video_row(tx, video)?;

    if inserted == 0 {
        return Ok(WbImportOutcome::Skipped);
    }

    tx.execute(
        "UPDATE videos SET rating = ?1, kana = ?2, roma = ?3 WHERE id = ?4",
        params![rating, kana, roma, video.id],
    )?;

    let mut tags_assigned = 0usize;
    for raw_tag in tags {
        let merge_key = gb_core::wb_import::wb_tag_merge_key(raw_tag);
        let tag = match find_or_create_tag_folding_case(tx, &merge_key, raw_tag) {
            Ok(tag) => tag,
            // Not a SQLite failure -- this one raw tag string can't become a
            // valid tag name (see `WbTagResolutionError::Empty`). Skip just
            // it and keep importing the rest of this video's tags/rating/
            // kana/roma rather than rolling back the whole row over it.
            Err(WbTagResolutionError::Empty) => {
                log::warn!(
                    "skipping .wb tag {raw_tag:?} for video {}: normalizes to an empty tag name",
                    video.id
                );
                continue;
            }
            Err(WbTagResolutionError::Sqlite(e)) => return Err(e),
        };
        tx.execute(
            "INSERT INTO video_tags (video_id, tag_id) VALUES (?1, ?2)
             ON CONFLICT(video_id, tag_id) DO NOTHING",
            params![video.id, tag.id],
        )?;
        tags_assigned += 1;
    }

    Ok(WbImportOutcome::Inserted { tags_assigned })
}

#[cfg(test)]
mod wb_import_tests {
    use super::*;
    use crate::db::queries::find_video_by_path;
    use crate::db::test_support::init_temp_db;

    fn new_video(id: &str, file_path: &str) -> NewVideo {
        NewVideo {
            id: id.to_string(),
            file_path: file_path.to_string(),
            file_name: "movie.mp4".to_string(),
            file_size: 1234,
            quick_hash: "h".to_string(),
            status: "online",
            mtime: 1_700_000_000,
        }
    }

    fn tags_of(conn: &Connection) -> Vec<String> {
        let mut stmt = conn.prepare("SELECT name FROM tags ORDER BY name").unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    fn video_tag_ids_for(conn: &Connection, video_id: &str) -> Vec<i64> {
        let mut stmt = conn
            .prepare("SELECT tag_id FROM video_tags WHERE video_id = ?1 ORDER BY tag_id")
            .unwrap();
        stmt.query_map(params![video_id], |r| r.get::<_, i64>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    }

    #[test]
    fn new_video_creates_row_rating_kana_roma_and_tags() {
        let (_dir, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();

        let video = new_video("id-1", "C:/videos/a.mp4");
        let outcome = import_wb_video(
            &mut conn,
            &video,
            4,
            "アクション",
            "akushon",
            &["Action".to_string(), "Comedy".to_string()],
        )
        .unwrap();

        assert_eq!(outcome, WbImportOutcome::Inserted { tags_assigned: 2 });

        let row = find_video_by_path(&conn, "C:/videos/a.mp4")
            .unwrap()
            .expect("video row should exist");
        assert_eq!(row.id, "id-1");
        assert_eq!(row.rating, 4);

        let (kana, roma): (Option<String>, Option<String>) = conn
            .query_row("SELECT kana, roma FROM videos WHERE id = 'id-1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(kana.as_deref(), Some("アクション"));
        assert_eq!(roma.as_deref(), Some("akushon"));

        assert_eq!(
            tags_of(&conn),
            vec!["Action".to_string(), "Comedy".to_string()]
        );
        assert_eq!(video_tag_ids_for(&conn, "id-1").len(), 2);
    }

    #[test]
    fn repeat_import_of_same_file_path_is_fully_skipped_and_does_not_overwrite() {
        let (_dir, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();

        let video = new_video("id-1", "C:/videos/a.mp4");
        let first = import_wb_video(
            &mut conn,
            &video,
            2,
            "かな1",
            "roma1",
            &["Original".to_string()],
        )
        .unwrap();
        assert_eq!(first, WbImportOutcome::Inserted { tags_assigned: 1 });

        // Second call: different id (as a fresh UUID generated by a re-run
        // would be), different rating/kana/roma/tags -- all of it must be
        // ignored because file_path already exists.
        let video_retry = new_video("id-2-different-uuid", "C:/videos/a.mp4");
        let second = import_wb_video(
            &mut conn,
            &video_retry,
            5,
            "かな2",
            "roma2",
            &["Different".to_string(), "MoreTags".to_string()],
        )
        .unwrap();
        assert_eq!(second, WbImportOutcome::Skipped);

        // Exactly one videos row, unchanged from the first call.
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let row = find_video_by_path(&conn, "C:/videos/a.mp4")
            .unwrap()
            .expect("video row should exist");
        assert_eq!(row.id, "id-1");
        assert_eq!(row.rating, 2, "second call's rating=5 must be ignored");

        let (kana, roma): (Option<String>, Option<String>) = conn
            .query_row("SELECT kana, roma FROM videos WHERE id = 'id-1'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(kana.as_deref(), Some("かな1"));
        assert_eq!(roma.as_deref(), Some("roma1"));

        // Only the first call's tag exists; the second call's tags were
        // never touched.
        assert_eq!(tags_of(&conn), vec!["Original".to_string()]);
        assert_eq!(video_tag_ids_for(&conn, "id-1").len(), 1);
    }

    #[test]
    fn case_and_width_variant_tags_fold_into_one_tags_row() {
        let (_dir, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();

        let video_a = new_video("id-a", "C:/videos/a.mp4");
        import_wb_video(&mut conn, &video_a, 0, "", "", &["Action".to_string()]).unwrap();

        let video_b = new_video("id-b", "C:/videos/b.mp4");
        import_wb_video(&mut conn, &video_b, 0, "", "", &["action".to_string()]).unwrap();

        // Still exactly one "Action"/"action" tag row -- the second import's
        // lowercase variant folded onto the first's, no new row was created.
        assert_eq!(tags_of(&conn), vec!["Action".to_string()]);

        let tag_id: i64 = conn
            .query_row("SELECT id FROM tags WHERE name = 'Action'", [], |r| {
                r.get(0)
            })
            .unwrap();

        assert_eq!(video_tag_ids_for(&conn, "id-a"), vec![tag_id]);
        assert_eq!(video_tag_ids_for(&conn, "id-b"), vec![tag_id]);
    }

    #[test]
    fn error_partway_through_rolls_back_video_row_and_earlier_tag_writes() {
        let (_dir, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();

        // A trigger that fails the *second* tag's insert into `tags`,
        // simulating a mid-transaction error after the video row, its
        // rating/kana/roma, and the first tag/assignment have already been
        // written within the same (uncommitted) transaction.
        conn.execute(
            "CREATE TRIGGER fail_on_boom_tag BEFORE INSERT ON tags
             WHEN NEW.name = 'boom'
             BEGIN SELECT RAISE(ABORT, 'simulated failure'); END;",
            [],
        )
        .unwrap();

        let video = new_video("id-1", "C:/videos/a.mp4");
        let result = import_wb_video(
            &mut conn,
            &video,
            3,
            "kana",
            "roma",
            &["safe".to_string(), "boom".to_string()],
        );
        assert!(result.is_err(), "the simulated failure should propagate");

        let video_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(video_count, 0, "the videos row must be rolled back too");

        let tag_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            tag_count, 0,
            "the first tag ('safe'), already inserted before the failure, must be rolled back"
        );

        let vt_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM video_tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            vt_count, 0,
            "no video_tags orphan may remain for the rolled-back video/tag"
        );
    }

    #[test]
    fn a_new_tag_created_from_a_full_width_wb_tag_is_stored_width_folded() {
        let (_dir, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();

        // Full-width "Action" (U+FF21/FF43/FF54/FF49/FF4F/FF4E), same
        // construction as wb_import.rs's own
        // wb_tag_merge_key_folds_full_width_characters test. No tag named
        // (half-width) "Action" exists yet, so this must go through
        // find_or_create_tag_folding_case's *create* path.
        let full_width_action = "\u{FF21}\u{FF43}\u{FF54}\u{FF49}\u{FF4F}\u{FF4E}";

        let video = new_video("id-1", "C:/videos/a.mp4");
        let outcome = import_wb_video(
            &mut conn,
            &video,
            0,
            "",
            "",
            &[full_width_action.to_string()],
        )
        .unwrap();
        assert_eq!(outcome, WbImportOutcome::Inserted { tags_assigned: 1 });

        // Stored half-width, per the "tags.name is always
        // width-folded" invariant -- not the raw full-width `.wb` string.
        assert_eq!(tags_of(&conn), vec!["Action".to_string()]);

        // A later manual tag entry of the half-width "Action" (as
        // gb_core::tags::normalize_tag_name would produce from user input)
        // must now match this same row on UNIQUE(name), not create a
        // duplicate.
        let tag_id: i64 = conn
            .query_row("SELECT id FROM tags WHERE name = 'Action'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(video_tag_ids_for(&conn, "id-1"), vec![tag_id]);
    }

    #[test]
    fn a_tag_that_normalizes_to_empty_is_skipped_without_failing_the_rest_of_the_import() {
        let (_dir, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();

        // A full-width space alone width-folds to a plain space, which then
        // trims away to nothing -- gb_core::tags::normalize_tag_name rejects
        // it. (In practice wb_import::split_tags's own Unicode-aware
        // str::trim already drops a tag line like this before it ever
        // reaches import_wb_video; this test exercises the defensive path
        // directly.)
        let video = new_video("id-1", "C:/videos/a.mp4");
        let outcome = import_wb_video(
            &mut conn,
            &video,
            0,
            "",
            "",
            &["Action".to_string(), "\u{3000}".to_string()],
        )
        .unwrap();

        // The video row and its one valid tag were still fully imported --
        // one bad tag string does not roll back the whole video.
        assert_eq!(outcome, WbImportOutcome::Inserted { tags_assigned: 1 });
        assert_eq!(tags_of(&conn), vec!["Action".to_string()]);
        assert_eq!(video_tag_ids_for(&conn, "id-1").len(), 1);

        let video_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(video_count, 1);
    }
}
