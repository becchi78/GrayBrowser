//! Integration tests for `queries::rename_watch_folder_videos` (backend for
//! the folder path edit action), against a real
//! tempdir-backed SQLite file.
//!
//! Mirrors `db_folder_removal.rs`'s fixtures/helpers (same
//! `gb_core::paths::folder_like_prefix`-based folder-boundary matching
//! underlies both) and additionally checks the core promise of this
//! feature: `id`/tags/rating/`created_at` all survive a folder path edit
//! untouched, only `file_path` (and, per row, `status`) change.
//!
//! `queries::rename_watch_folder_videos` covers the scenarios below
//! (nonexistent target path, colliding target path, atomicity, and
//! online/offline status outcomes). Rejecting an overlapping new path is
//! validated one layer up, in
//! `commands::settings_cmds::validate_rename_target` -- pure and unit-tested
//! there without needing a Tauri `State`/`AppHandle`, so it is not
//! duplicated in this integration-test file.

use graybrowser_lib::db::{self, queries};
use rusqlite::{params, Connection};

fn init_temp_db() -> (tempfile::TempDir, db::Db) {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");
    let db = db::init(&db_path).expect("db::init should succeed against a fresh tempdir file");
    (dir, db)
}

fn insert_video(conn: &Connection, id: &str, file_path: &str, status: &str) {
    let file_name = file_path.rsplit('\\').next().unwrap_or(file_path);
    conn.execute(
        "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status, rating, created_at)
         VALUES (?1, ?2, ?3, 1024, 'hash', ?4, 0, '2026-01-01 00:00:00')",
        params![id, file_path, file_name, status],
    )
    .unwrap();
}

fn insert_tag(conn: &Connection, name: &str) -> i64 {
    conn.execute("INSERT INTO tags (name) VALUES (?1)", params![name])
        .unwrap();
    conn.last_insert_rowid()
}

fn assign_tag(conn: &Connection, video_id: &str, tag_id: i64) {
    conn.execute(
        "INSERT INTO video_tags (video_id, tag_id) VALUES (?1, ?2)",
        params![video_id, tag_id],
    )
    .unwrap();
}

struct VideoSnapshot {
    file_path: String,
    file_name: String,
    status: String,
    rating: i64,
    created_at: String,
}

fn snapshot(conn: &Connection, video_id: &str) -> VideoSnapshot {
    conn.query_row(
        "SELECT file_path, file_name, status, rating, created_at FROM videos WHERE id = ?1",
        params![video_id],
        |r| {
            Ok(VideoSnapshot {
                file_path: r.get(0)?,
                file_name: r.get(1)?,
                status: r.get(2)?,
                rating: r.get(3)?,
                created_at: r.get(4)?,
            })
        },
    )
    .unwrap()
}

fn tag_names_for(conn: &Connection, video_id: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT tags.name FROM tags
             JOIN video_tags ON video_tags.tag_id = tags.id
             WHERE video_tags.video_id = ?1
             ORDER BY tags.name",
        )
        .unwrap();
    stmt.query_map(params![video_id], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn video_ids(conn: &Connection) -> Vec<String> {
    let mut stmt = conn.prepare("SELECT id FROM videos ORDER BY id").unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn path_collision_rows(conn: &Connection) -> Vec<(String, String, String)> {
    let mut stmt = conn
        .prepare("SELECT video_id, colliding_video_id, attempted_path FROM path_collisions ORDER BY video_id")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

// --- Core promise: id/tags/rating/created_at survive a folder rename -------

#[test]
fn renaming_a_folder_preserves_id_tags_rating_and_created_at() {
    let (dir, db) = init_temp_db();
    // Neither the old nor the new folder needs to exist on disk for this
    // test -- it only asserts that non-file_path/status columns are
    // untouched, independent of the online/offline outcome (covered
    // separately below).
    let old_folder = r"C:\OldVideos".to_string();
    let new_folder = dir.path().join("NewVideos").to_string_lossy().into_owned();

    let mut conn = db.writer.lock().unwrap();
    insert_video(&conn, "v1", r"C:\OldVideos\a.mp4", "online");
    let tag_id = insert_tag(&conn, "action");
    assign_tag(&conn, "v1", tag_id);
    queries::set_rating(&conn, "v1", 4).unwrap();

    let before = snapshot(&conn, "v1");
    assert_eq!(before.rating, 4);
    assert_eq!(before.created_at, "2026-01-01 00:00:00");

    let outcome = queries::rename_watch_folder_videos(&mut conn, &old_folder, &new_folder).unwrap();
    assert_eq!(outcome.renamed_count, 1);
    assert_eq!(outcome.collision_skipped_count, 0);

    // id itself never changes -- the row updated in place is still "v1".
    assert_eq!(video_ids(&conn), vec!["v1"]);

    let after = snapshot(&conn, "v1");
    assert_eq!(after.file_path, format!("{new_folder}\\a.mp4"));
    assert_eq!(after.file_name, "a.mp4", "file_name must be untouched");
    assert_eq!(after.rating, 4, "rating must survive the rename");
    assert_eq!(
        after.created_at, "2026-01-01 00:00:00",
        "created_at must survive the rename"
    );
    assert_eq!(
        tag_names_for(&conn, "v1"),
        vec!["action".to_string()],
        "the tag assignment must survive the rename"
    );
}

// --- New path does not exist on disk -> stays/becomes offline -------

#[test]
fn rename_to_a_nonexistent_new_path_leaves_the_row_offline() {
    let (dir, db) = init_temp_db();
    let old_folder = r"C:\OldVideos".to_string();
    // A real, but empty, tempdir subfolder -- the file itself is never
    // created there.
    let new_folder = dir.path().join("Missing").to_string_lossy().into_owned();

    let mut conn = db.writer.lock().unwrap();
    insert_video(&conn, "v1", r"C:\OldVideos\a.mp4", "online");

    let outcome = queries::rename_watch_folder_videos(&mut conn, &old_folder, &new_folder).unwrap();
    assert_eq!(outcome.renamed_count, 1);

    let after = snapshot(&conn, "v1");
    assert_eq!(after.file_path, format!("{new_folder}\\a.mp4"));
    assert_eq!(after.status, "offline");
}

// --- New path exists on disk -> becomes/stays online --------------

#[test]
fn rename_to_an_existing_new_path_marks_the_row_online() {
    let (dir, db) = init_temp_db();
    let old_folder = r"C:\OldVideos".to_string();
    let new_folder_dir = dir.path().join("NewVideos");
    std::fs::create_dir_all(&new_folder_dir).unwrap();
    std::fs::write(new_folder_dir.join("a.mp4"), b"fake video bytes").unwrap();
    let new_folder = new_folder_dir.to_string_lossy().into_owned();

    let mut conn = db.writer.lock().unwrap();
    // Starts offline (e.g. the drive was disconnected before the user
    // edited the path) -- the rename's own existence check must flip it
    // back to online without waiting for a rescan.
    insert_video(&conn, "v1", r"C:\OldVideos\a.mp4", "offline");

    let outcome = queries::rename_watch_folder_videos(&mut conn, &old_folder, &new_folder).unwrap();
    assert_eq!(outcome.renamed_count, 1);

    let after = snapshot(&conn, "v1");
    assert_eq!(after.file_path, format!("{new_folder}\\a.mp4"));
    assert_eq!(after.status, "online");
}

#[test]
fn rename_reflects_a_per_row_mix_of_online_and_offline_outcomes() {
    let (dir, db) = init_temp_db();
    let old_folder = r"C:\OldVideos".to_string();
    let new_folder_dir = dir.path().join("NewVideos");
    std::fs::create_dir_all(&new_folder_dir).unwrap();
    // Only "present.mp4" actually exists under the new folder.
    std::fs::write(new_folder_dir.join("present.mp4"), b"fake video bytes").unwrap();
    let new_folder = new_folder_dir.to_string_lossy().into_owned();

    let mut conn = db.writer.lock().unwrap();
    insert_video(&conn, "present", r"C:\OldVideos\present.mp4", "offline");
    insert_video(&conn, "absent", r"C:\OldVideos\absent.mp4", "online");

    let outcome = queries::rename_watch_folder_videos(&mut conn, &old_folder, &new_folder).unwrap();
    assert_eq!(outcome.renamed_count, 2);
    assert_eq!(outcome.collision_skipped_count, 0);

    assert_eq!(snapshot(&conn, "present").status, "online");
    assert_eq!(snapshot(&conn, "absent").status, "offline");
}

// --- A colliding new path is skipped, not fatal to the whole batch --

#[test]
fn a_colliding_row_is_skipped_and_recorded_while_other_rows_still_rename() {
    let (dir, db) = init_temp_db();
    let old_folder = r"C:\OldVideos".to_string();
    let new_folder = dir.path().join("NewVideos").to_string_lossy().into_owned();

    let mut conn = db.writer.lock().unwrap();
    // "clashing" will be renamed to `{new_folder}\taken.mp4`, which
    // "outside" already occupies -- a pre-existing, unrelated row, not
    // itself under `old_folder`.
    insert_video(&conn, "clashing", r"C:\OldVideos\taken.mp4", "online");
    insert_video(&conn, "free", r"C:\OldVideos\free.mp4", "online");
    insert_video(
        &conn,
        "outside",
        &format!("{new_folder}\\taken.mp4"),
        "online",
    );

    let outcome = queries::rename_watch_folder_videos(&mut conn, &old_folder, &new_folder).unwrap();
    assert_eq!(outcome.renamed_count, 1, "only \"free\" should be renamed");
    assert_eq!(
        outcome.collision_skipped_count, 1,
        "\"clashing\" must be skipped"
    );

    // The skipped row keeps its original path/status entirely -- nothing
    // about it changed.
    let clashing = snapshot(&conn, "clashing");
    assert_eq!(clashing.file_path, r"C:\OldVideos\taken.mp4");
    assert_eq!(clashing.status, "online");

    // The non-colliding row under the same folder still renamed normally.
    let free = snapshot(&conn, "free");
    assert_eq!(free.file_path, format!("{new_folder}\\free.mp4"));

    // The unrelated pre-existing occupant of the target path is untouched.
    let outside = snapshot(&conn, "outside");
    assert_eq!(outside.file_path, format!("{new_folder}\\taken.mp4"));

    // The collision is recorded for the duplicate-candidates UI to surface.
    assert_eq!(
        path_collision_rows(&conn),
        vec![(
            "clashing".to_string(),
            "outside".to_string(),
            format!("{new_folder}\\taken.mp4"),
        )]
    );
}

#[test]
fn a_colliding_row_versus_an_offline_occupant_is_still_skipped() {
    // file_path is UNIQUE across every row regardless of status -- an
    // offline occupant of the target path is just as much a collision as an
    // online one would be, since the batch UPDATE would otherwise violate
    // that UNIQUE constraint.
    let (dir, db) = init_temp_db();
    let old_folder = r"C:\OldVideos".to_string();
    let new_folder = dir.path().join("NewVideos").to_string_lossy().into_owned();

    let mut conn = db.writer.lock().unwrap();
    insert_video(&conn, "clashing", r"C:\OldVideos\taken.mp4", "online");
    insert_video(
        &conn,
        "outside_offline",
        &format!("{new_folder}\\taken.mp4"),
        "offline",
    );

    let outcome = queries::rename_watch_folder_videos(&mut conn, &old_folder, &new_folder).unwrap();
    assert_eq!(outcome.renamed_count, 0);
    assert_eq!(outcome.collision_skipped_count, 1);

    let clashing = snapshot(&conn, "clashing");
    assert_eq!(clashing.file_path, r"C:\OldVideos\taken.mp4");
}

// --- Atomicity: every non-colliding row in the folder renames -------------
// together within the single transaction (an indirect check -- a genuine
// mid-transaction failure is impractical to force against SQLite here; the
// shared-transaction code path itself is reviewed directly in
// `queries::rename_watch_folder_videos`).

#[test]
fn every_non_colliding_row_under_the_folder_is_renamed_together() {
    let (dir, db) = init_temp_db();
    let old_folder = r"C:\OldVideos".to_string();
    let new_folder = dir.path().join("NewVideos").to_string_lossy().into_owned();

    let mut conn = db.writer.lock().unwrap();
    insert_video(&conn, "a", r"C:\OldVideos\a.mp4", "online");
    insert_video(&conn, "b", r"C:\OldVideos\Sub\b.mp4", "online");
    insert_video(&conn, "c", r"C:\OldVideos\Sub\Deep\c.mp4", "online");
    // A sibling folder that merely shares a string prefix must be left
    // completely alone (same folder-boundary-safety fixture as
    // db_folder_removal.rs).
    insert_video(&conn, "sibling", r"C:\OldVideos2\d.mp4", "online");

    let outcome = queries::rename_watch_folder_videos(&mut conn, &old_folder, &new_folder).unwrap();
    assert_eq!(outcome.renamed_count, 3);
    assert_eq!(outcome.collision_skipped_count, 0);

    assert_eq!(
        snapshot(&conn, "a").file_path,
        format!("{new_folder}\\a.mp4")
    );
    assert_eq!(
        snapshot(&conn, "b").file_path,
        format!("{new_folder}\\Sub\\b.mp4")
    );
    assert_eq!(
        snapshot(&conn, "c").file_path,
        format!("{new_folder}\\Sub\\Deep\\c.mp4")
    );
    assert_eq!(
        snapshot(&conn, "sibling").file_path,
        r"C:\OldVideos2\d.mp4",
        "a sibling folder with a shared string prefix must not be touched"
    );
}

// --- renamed_videos: the data settings_cmds::rename_watch_folder relies on
// to move thumbnails video-by-video rather than as a whole-subdirectory
// rename (see `queries::RenameWatchFolderOutcome`'s doc comment) ---------

#[test]
fn renamed_videos_reports_the_old_and_new_file_path_for_every_actually_renamed_row() {
    let (dir, db) = init_temp_db();
    let old_folder = r"C:\OldVideos".to_string();
    let new_folder = dir.path().join("NewVideos").to_string_lossy().into_owned();

    let mut conn = db.writer.lock().unwrap();
    insert_video(&conn, "a", r"C:\OldVideos\a.mp4", "online");
    insert_video(&conn, "b", r"C:\OldVideos\Sub\b.mp4", "online");

    let outcome = queries::rename_watch_folder_videos(&mut conn, &old_folder, &new_folder).unwrap();

    let mut renamed = outcome.renamed_videos;
    renamed.sort_by(|a, b| a.video_id.cmp(&b.video_id));
    assert_eq!(renamed.len(), 2);
    assert_eq!(renamed[0].video_id, "a");
    assert_eq!(renamed[0].old_file_path, r"C:\OldVideos\a.mp4");
    assert_eq!(renamed[0].new_file_path, format!("{new_folder}\\a.mp4"));
    assert_eq!(renamed[1].video_id, "b");
    assert_eq!(renamed[1].old_file_path, r"C:\OldVideos\Sub\b.mp4");
    assert_eq!(
        renamed[1].new_file_path,
        format!("{new_folder}\\Sub\\b.mp4")
    );
}

#[test]
fn renamed_videos_excludes_a_collision_skipped_row() {
    let (dir, db) = init_temp_db();
    let old_folder = r"C:\OldVideos".to_string();
    let new_folder = dir.path().join("NewVideos").to_string_lossy().into_owned();

    let mut conn = db.writer.lock().unwrap();
    insert_video(&conn, "clashing", r"C:\OldVideos\taken.mp4", "online");
    insert_video(&conn, "free", r"C:\OldVideos\free.mp4", "online");
    insert_video(
        &conn,
        "outside",
        &format!("{new_folder}\\taken.mp4"),
        "online",
    );

    let outcome = queries::rename_watch_folder_videos(&mut conn, &old_folder, &new_folder).unwrap();

    assert_eq!(
        outcome.renamed_videos.len(),
        1,
        "only the non-colliding row must be reported"
    );
    assert_eq!(outcome.renamed_videos[0].video_id, "free");
    assert_eq!(
        outcome.renamed_videos[0].old_file_path,
        r"C:\OldVideos\free.mp4"
    );
    assert_eq!(
        outcome.renamed_videos[0].new_file_path,
        format!("{new_folder}\\free.mp4")
    );
}

#[test]
fn rename_of_an_already_empty_folder_is_a_no_op() {
    let (dir, db) = init_temp_db();
    let old_folder = r"C:\Empty".to_string();
    let new_folder = dir.path().join("StillEmpty").to_string_lossy().into_owned();

    let mut conn = db.writer.lock().unwrap();
    insert_video(&conn, "elsewhere", r"D:\Other\a.mp4", "online");

    let outcome = queries::rename_watch_folder_videos(&mut conn, &old_folder, &new_folder).unwrap();
    assert_eq!(outcome.renamed_count, 0);
    assert_eq!(outcome.collision_skipped_count, 0);
    assert_eq!(snapshot(&conn, "elsewhere").file_path, r"D:\Other\a.mp4");
}
