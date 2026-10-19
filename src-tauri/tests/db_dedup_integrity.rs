//! Integration tests for the DB layer: the `full_hash`
//! read/write round trip feeding `gb_core::dedup`'s pure grouping functions,
//! and the `path_collisions` table, which persists route-X's UNIQUE
//! collision as a duplicate candidate; the same table is also reused for
//! route-Y's coincidental-rehash-match collisions, see
//! src-tauri/src/scan/mod.rs.
//! Real tempdir-backed SQLite file, matching
//! `db_tag_integrity.rs`'s established convention.

use graybrowser_lib::db::{self, queries};
use rusqlite::{params, Connection};

fn init_temp_db() -> (tempfile::TempDir, db::Db) {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");
    let db = db::init(&db_path).expect("db::init should succeed against a fresh tempdir file");
    (dir, db)
}

fn insert_video(conn: &Connection, id: &str, file_path: &str, status: &str, quick_hash: &str) {
    conn.execute(
        "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status)
         VALUES (?1, ?2, ?3, 1024, ?4, ?5)",
        params![
            id,
            file_path,
            file_path.rsplit('\\').next().unwrap_or(file_path),
            quick_hash,
            status,
        ],
    )
    .unwrap();
}

/// Every `path_collisions` row must reference `videos.id` values that still
/// exist, on both sides -- the app-layer orphan-prevention guarantee
/// `delete_video_cascade` now extends to this table.
fn orphaned_path_collisions(conn: &Connection) -> Vec<(i64, String, String)> {
    let mut stmt = conn
        .prepare(
            "SELECT pc.id, pc.video_id, pc.colliding_video_id
             FROM path_collisions pc
             LEFT JOIN videos v1 ON v1.id = pc.video_id
             LEFT JOIN videos v2 ON v2.id = pc.colliding_video_id
             WHERE v1.id IS NULL OR v2.id IS NULL",
        )
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn update_full_hash_round_trips_through_list_online_video_hash_info() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "v1", "D:\\videos\\a.mp4", "online", "qh1");
    }
    {
        let conn = db.writer.lock().unwrap();
        queries::update_full_hash(&conn, "v1", "full-hash-value").unwrap();
    }

    let rows = queries::list_online_video_hash_info(&db.read_pool).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "v1");
    assert_eq!(rows[0].quick_hash, "qh1");
    assert_eq!(rows[0].file_size, 1024);
    assert_eq!(rows[0].full_hash.as_deref(), Some("full-hash-value"));
}

#[test]
fn list_online_video_hash_info_excludes_offline_and_empty_quick_hash_rows() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "online-real", "D:\\videos\\a.mp4", "online", "qh1");
        insert_video(&conn, "offline-real", "D:\\videos\\b.mp4", "offline", "qh2");
        // Offline placeholder rows use an empty quick_hash per
        // gb_core::dedup's own doc comment.
        insert_video(
            &conn,
            "online-empty-hash",
            "D:\\videos\\c.mp4",
            "online",
            "",
        );
    }

    let rows = queries::list_online_video_hash_info(&db.read_pool).unwrap();
    let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["online-real"],
        "only the online row with a non-empty quick_hash should be returned"
    );
}

#[test]
fn record_path_collision_is_idempotent_for_the_same_pair() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "offline-1", "D:\\old\\a.mp4", "offline", "qh1");
        insert_video(&conn, "online-1", "D:\\new\\a.mp4", "online", "qh2");
    }
    {
        let conn = db.writer.lock().unwrap();
        queries::record_path_collision(&conn, "offline-1", "online-1", "D:\\new\\a.mp4").unwrap();
        queries::record_path_collision(&conn, "offline-1", "online-1", "D:\\new\\a.mp4").unwrap();
    }

    let rows = queries::list_path_collisions(&db.read_pool).unwrap();
    assert_eq!(
        rows.len(),
        1,
        "re-detecting the same collision pair must not duplicate the row"
    );
    assert_eq!(rows[0].video_id, "offline-1");
    assert_eq!(rows[0].colliding_video_id, "online-1");
}

#[test]
fn record_path_collision_refreshes_attempted_path_on_repeat_detection() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "offline-1", "D:\\old\\a.mp4", "offline", "qh1");
        insert_video(&conn, "online-1", "D:\\new\\a.mp4", "online", "qh2");
    }
    {
        let conn = db.writer.lock().unwrap();
        queries::record_path_collision(&conn, "offline-1", "online-1", "D:\\new\\a.mp4").unwrap();
        queries::record_path_collision(&conn, "offline-1", "online-1", "D:\\new\\renamed.mp4")
            .unwrap();
    }

    let rows = queries::list_path_collisions(&db.read_pool).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].attempted_path, "D:\\new\\renamed.mp4");
}

#[test]
fn delete_video_cascade_removes_path_collisions_referencing_the_deleted_video_from_either_side() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "offline-1", "D:\\old\\a.mp4", "offline", "qh1");
        insert_video(&conn, "online-1", "D:\\new\\a.mp4", "online", "qh2");
        insert_video(&conn, "offline-2", "D:\\old\\b.mp4", "offline", "qh3");
        insert_video(&conn, "online-2", "D:\\new\\b.mp4", "online", "qh4");
    }
    {
        let conn = db.writer.lock().unwrap();
        // online-1 is the deleted video's collision partner as the
        // colliding_video_id side.
        queries::record_path_collision(&conn, "offline-1", "online-1", "D:\\new\\a.mp4").unwrap();
        // An unrelated pair that must survive the deletion below.
        queries::record_path_collision(&conn, "offline-2", "online-2", "D:\\new\\b.mp4").unwrap();
    }

    {
        let mut conn = db.writer.lock().unwrap();
        queries::delete_video_cascade(&mut conn, "online-1").unwrap();
    }

    let rows = queries::list_path_collisions(&db.read_pool).unwrap();
    assert_eq!(
        rows.len(),
        1,
        "the collision referencing the deleted video (as colliding_video_id) must be gone"
    );
    assert_eq!(rows[0].video_id, "offline-2");
    assert_eq!(rows[0].colliding_video_id, "online-2");

    let conn = db.writer.lock().unwrap();
    assert!(
        orphaned_path_collisions(&conn).is_empty(),
        "no path_collisions row may reference a nonexistent videos.id after delete_video_cascade"
    );
}

#[test]
fn delete_video_cascade_removes_path_collisions_where_deleted_video_is_the_video_id_side() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "offline-1", "D:\\old\\a.mp4", "offline", "qh1");
        insert_video(&conn, "online-1", "D:\\new\\a.mp4", "online", "qh2");
    }
    {
        let conn = db.writer.lock().unwrap();
        queries::record_path_collision(&conn, "offline-1", "online-1", "D:\\new\\a.mp4").unwrap();
    }

    {
        let mut conn = db.writer.lock().unwrap();
        queries::delete_video_cascade(&mut conn, "offline-1").unwrap();
    }

    let rows = queries::list_path_collisions(&db.read_pool).unwrap();
    assert!(
        rows.is_empty(),
        "the collision referencing the deleted video (as video_id) must be gone"
    );
}

#[test]
fn delete_videos_cascade_removes_listed_videos_and_leaves_others_intact() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "a", "D:\\v\\a.mp4", "online", "qh1");
        insert_video(&conn, "b", "D:\\v\\b.mp4", "offline", "qh2");
        insert_video(&conn, "c", "D:\\v\\c.mp4", "online", "qh3");
        insert_video(&conn, "d", "D:\\v\\d.mp4", "online", "qh4");
        queries::record_path_collision(&conn, "b", "a", "D:\\v\\a.mp4").unwrap();
        queries::record_path_collision(&conn, "c", "d", "D:\\v\\d.mp4").unwrap();
    }
    {
        let mut conn = db.writer.lock().unwrap();
        for id in ["a", "c", "d"] {
            queries::assign_tag_to_video(&mut conn, id, "tag").unwrap();
        }
    }

    let deleted = {
        let mut conn = db.writer.lock().unwrap();
        let ids = vec!["a".to_string(), "missing".to_string(), "c".to_string()];
        queries::delete_videos_cascade(&mut conn, &ids).unwrap()
    };
    assert_eq!(deleted, 2);

    let conn = db.writer.lock().unwrap();
    let remaining: Vec<String> = conn
        .prepare("SELECT id FROM videos ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(remaining, vec!["b", "d"]);
    let tagged: Vec<String> = conn
        .prepare("SELECT video_id FROM video_tags ORDER BY video_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(tagged, vec!["d"], "only the surviving video keeps its tag");
    // Both collisions touched a deleted video (a, or c), so none may remain.
    let collisions: i64 = conn
        .query_row("SELECT COUNT(*) FROM path_collisions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(collisions, 0);
    assert!(orphaned_path_collisions(&conn).is_empty());
}

#[test]
fn delete_videos_cascade_keeps_collisions_between_surviving_videos() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "a", "D:\\v\\a.mp4", "online", "qh1");
        insert_video(&conn, "b", "D:\\v\\b.mp4", "offline", "qh2");
        insert_video(&conn, "c", "D:\\v\\c.mp4", "online", "qh3");
        queries::record_path_collision(&conn, "b", "a", "D:\\v\\a.mp4").unwrap();
    }
    {
        let mut conn = db.writer.lock().unwrap();
        assert_eq!(
            queries::delete_videos_cascade(&mut conn, &["c".to_string()]).unwrap(),
            1
        );
        assert_eq!(queries::delete_videos_cascade(&mut conn, &[]).unwrap(), 0);
    }
    assert_eq!(
        queries::list_path_collisions(&db.read_pool).unwrap().len(),
        1
    );
}

#[test]
fn delete_videos_cascade_rolls_back_everything_when_a_later_delete_fails() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "a", "D:\\v\\a.mp4", "online", "qh1");
        insert_video(&conn, "b", "D:\\v\\b.mp4", "online", "qh2");
        conn.execute_batch(
            "CREATE TRIGGER block_b BEFORE DELETE ON videos WHEN OLD.id = 'b'
             BEGIN SELECT RAISE(ABORT, 'blocked'); END;",
        )
        .unwrap();
    }
    {
        let mut conn = db.writer.lock().unwrap();
        queries::assign_tag_to_video(&mut conn, "a", "tag").unwrap();
        let ids = vec!["a".to_string(), "b".to_string()];
        assert!(queries::delete_videos_cascade(&mut conn, &ids).is_err());
    }

    let conn = db.writer.lock().unwrap();
    let videos: i64 = conn
        .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
        .unwrap();
    let tags: i64 = conn
        .query_row("SELECT COUNT(*) FROM video_tags", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        (videos, tags),
        (2, 1),
        "the first video's delete must roll back"
    );
}
