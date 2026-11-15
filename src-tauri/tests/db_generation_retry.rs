//! Integration tests for the DB layer's
//! `thumbnail_attempts`/`metadata_attempts` counters and the
//! queries built on top of them (increment/reset, the attempts-carrying
//! listing variants, and the exhausted-attempts listings feeding the retry
//! UI). Real tempdir-backed SQLite file, matching `db_dedup_integrity.rs`'s
//! established convention.
//!
//! The two counters have the same operations, so the per-counter tests loop
//! over both `Counter`s instead of keeping a mirrored copy for each.

mod common;

use common::{init_temp_db, insert_video, insert_video_with_status};
use gb_core::retry::MAX_GENERATION_ATTEMPTS;
use graybrowser_lib::db;
use graybrowser_lib::db::queries;
use rusqlite::Connection;

fn thumbnail_attempts(conn: &Connection, id: &str) -> i64 {
    conn.query_row(
        "SELECT thumbnail_attempts FROM videos WHERE id = ?1",
        [id],
        |r| r.get(0),
    )
    .unwrap()
}

fn metadata_attempts(conn: &Connection, id: &str) -> i64 {
    conn.query_row(
        "SELECT metadata_attempts FROM videos WHERE id = ?1",
        [id],
        |r| r.get(0),
    )
    .unwrap()
}

/// `(id, file_path, file_name, attempts)` of a row the retry UI lists as exhausted.
type ExhaustedRow = (String, String, String, i64);

/// One of the two retry counters and the queries that operate on it.
struct Counter {
    name: &'static str,
    read: fn(&Connection, &str) -> i64,
    increment: fn(&Connection, &str) -> rusqlite::Result<()>,
    reset: fn(&Connection, &str) -> rusqlite::Result<()>,
    reset_for: fn(&mut Connection, &[String]) -> rusqlite::Result<()>,
    exhausted: fn(&db::Db) -> Vec<ExhaustedRow>,
}

const THUMBNAIL: Counter = Counter {
    name: "thumbnail",
    read: thumbnail_attempts,
    increment: queries::increment_thumbnail_attempts,
    reset: queries::reset_thumbnail_attempts,
    reset_for: queries::reset_thumbnail_attempts_for,
    exhausted: |db| {
        queries::list_videos_with_exhausted_thumbnail_attempts(&db.read_pool)
            .unwrap()
            .into_iter()
            .map(|r| (r.id, r.file_path, r.file_name, r.thumbnail_attempts))
            .collect()
    },
};

const METADATA: Counter = Counter {
    name: "metadata",
    read: metadata_attempts,
    increment: queries::increment_metadata_attempts,
    reset: queries::reset_metadata_attempts,
    reset_for: queries::reset_metadata_attempts_for,
    exhausted: |db| {
        queries::list_videos_with_exhausted_metadata_attempts(&db.read_pool)
            .unwrap()
            .into_iter()
            .map(|r| (r.id, r.file_path, r.file_name, r.metadata_attempts))
            .collect()
    },
};

/// Each counter paired with the other one, to check neither disturbs its twin.
const PAIRS: [(Counter, Counter); 2] = [(THUMBNAIL, METADATA), (METADATA, THUMBNAIL)];

#[test]
fn a_freshly_inserted_video_starts_with_zero_attempts_of_either_kind() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    insert_video(&conn, "v1", "D:\\videos\\a.mp4");

    assert_eq!(thumbnail_attempts(&conn, "v1"), 0);
    assert_eq!(metadata_attempts(&conn, "v1"), 0);
}

#[test]
fn increment_accumulates_across_repeated_calls() {
    for counter in [THUMBNAIL, METADATA] {
        let (_dir, db) = init_temp_db();
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "v1", "D:\\videos\\a.mp4");

        for expected in 1..=3 {
            (counter.increment)(&conn, "v1").unwrap();
            assert_eq!(
                (counter.read)(&conn, "v1"),
                expected,
                "{} counter",
                counter.name
            );
        }
    }
}

#[test]
fn increment_thumbnail_and_metadata_attempts_are_independent_counters() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "v1", "D:\\videos\\a.mp4");
    }

    let conn = db.writer.lock().unwrap();
    queries::increment_thumbnail_attempts(&conn, "v1").unwrap();
    queries::increment_thumbnail_attempts(&conn, "v1").unwrap();
    queries::increment_metadata_attempts(&conn, "v1").unwrap();

    assert_eq!(thumbnail_attempts(&conn, "v1"), 2);
    assert_eq!(metadata_attempts(&conn, "v1"), 1);
}

#[test]
fn reset_returns_the_counter_to_zero() {
    for counter in [THUMBNAIL, METADATA] {
        let (_dir, db) = init_temp_db();
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "v1", "D:\\videos\\a.mp4");
        (counter.increment)(&conn, "v1").unwrap();
        (counter.increment)(&conn, "v1").unwrap();
        assert_eq!((counter.read)(&conn, "v1"), 2, "{} counter", counter.name);

        (counter.reset)(&conn, "v1").unwrap();
        assert_eq!((counter.read)(&conn, "v1"), 0, "{} counter", counter.name);
    }
}

#[test]
fn reset_for_zeroes_only_the_given_ids_and_leaves_the_other_counter_alone() {
    for (counter, other) in PAIRS {
        let (_dir, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();
        for id in ["v1", "v2", "v3"] {
            insert_video(&conn, id, &format!("D:\\videos\\{id}.mp4"));
            (counter.increment)(&conn, id).unwrap();
            (counter.increment)(&conn, id).unwrap();
        }
        (other.increment)(&conn, "v1").unwrap();

        (counter.reset_for)(
            &mut conn,
            &["v1".to_string(), "v3".to_string(), "no-such-id".to_string()],
        )
        .unwrap();

        let name = counter.name;
        assert_eq!((counter.read)(&conn, "v1"), 0, "{name} counter");
        assert_eq!(
            (counter.read)(&conn, "v2"),
            2,
            "{name} counter: unlisted id is untouched"
        );
        assert_eq!((counter.read)(&conn, "v3"), 0, "{name} counter");
        assert_eq!(
            (other.read)(&conn, "v1"),
            1,
            "{name} reset must leave the {} counter untouched",
            other.name
        );
    }
}

#[test]
fn reset_for_with_no_ids_changes_nothing() {
    for counter in [THUMBNAIL, METADATA] {
        let (_dir, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();
        insert_video(&conn, "v1", "D:\\videos\\a.mp4");
        (counter.increment)(&conn, "v1").unwrap();

        (counter.reset_for)(&mut conn, &[]).unwrap();

        assert_eq!((counter.read)(&conn, "v1"), 1, "{} counter", counter.name);
    }
}

#[test]
fn reset_video_derived_data_clears_derived_columns_only_for_the_target_video() {
    let (_dir, db) = init_temp_db();
    let conn = db.writer.lock().unwrap();
    insert_video(&conn, "v1", "D:\\videos\\a.mp4");
    insert_video(&conn, "v2", "D:\\videos\\b.mp4");
    conn.execute(
        "UPDATE videos SET thumbnail_ready = 1, thumbnail_attempts = 2, duration = 9,
         probed_at = CURRENT_TIMESTAMP, metadata_attempts = 3, width = 640, height = 480,
         video_codec = 'h264', audio_codec = 'aac', bitrate = 1000, fps = 30.0, rating = 4",
        [],
    )
    .unwrap();

    queries::reset_video_derived_data(&conn, "v1").unwrap();

    // Rolled into one string: a 14-column tuple has no Debug/PartialEq.
    let row = |id: &str| -> String {
        conn.query_row(
            "SELECT thumbnail_ready || '|' || thumbnail_attempts || '|' || IFNULL(duration, 'N')
                    || '|' || IFNULL(probed_at, 'N') || '|' || metadata_attempts
                    || '|' || IFNULL(width, 'N') || '|' || IFNULL(height, 'N')
                    || '|' || IFNULL(video_codec, 'N') || '|' || IFNULL(audio_codec, 'N')
                    || '|' || IFNULL(bitrate, 'N') || '|' || IFNULL(fps, 'N')
                    || '|' || rating || '|' || file_size || '|' || quick_hash
             FROM videos WHERE id = ?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(row("v1"), "0|0|N|N|0|N|N|N|N|N|N|4|1024|hash");
    assert!(row("v2").starts_with("1|2|9|"));
    assert!(row("v2").contains("|640|480|h264|aac|1000|30.0|"));
}

#[test]
fn list_online_video_paths_with_thumbnail_attempts_returns_the_current_counter_and_excludes_offline(
) {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "online-1", "D:\\videos\\a.mp4");
        insert_video_with_status(&conn, "offline-1", "D:\\videos\\b.mp4", "offline");
        queries::increment_thumbnail_attempts(&conn, "online-1").unwrap();
        queries::increment_thumbnail_attempts(&conn, "online-1").unwrap();
    }

    let rows = queries::list_online_video_paths_with_thumbnail_attempts(&db.read_pool).unwrap();
    assert_eq!(
        rows,
        vec![(
            "online-1".to_string(),
            "D:\\videos\\a.mp4".to_string(),
            2i64,
            false
        )]
    );
}

#[test]
fn list_online_video_paths_with_thumbnail_attempts_reports_the_current_ready_flag() {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "ready-1", "D:\\videos\\a.mp4");
        insert_video(&conn, "not-ready-1", "D:\\videos\\b.mp4");
        queries::mark_thumbnail_ready(&conn, "ready-1").unwrap();
    }

    let mut rows = queries::list_online_video_paths_with_thumbnail_attempts(&db.read_pool).unwrap();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        rows,
        vec![
            (
                "not-ready-1".to_string(),
                "D:\\videos\\b.mp4".to_string(),
                0i64,
                false
            ),
            (
                "ready-1".to_string(),
                "D:\\videos\\a.mp4".to_string(),
                0i64,
                true
            ),
        ]
    );
}

#[test]
fn list_videos_missing_metadata_with_attempts_returns_the_current_counter_and_excludes_probed_and_offline(
) {
    let (_dir, db) = init_temp_db();
    {
        let conn = db.writer.lock().unwrap();
        insert_video(&conn, "unprobed", "D:\\videos\\a.mp4");
        insert_video_with_status(&conn, "offline", "D:\\videos\\b.mp4", "offline");
        insert_video(&conn, "probed", "D:\\videos\\c.mp4");
        conn.execute(
            "UPDATE videos SET probed_at = CURRENT_TIMESTAMP WHERE id = 'probed'",
            [],
        )
        .unwrap();
        queries::increment_metadata_attempts(&conn, "unprobed").unwrap();
    }

    let rows = queries::list_videos_missing_metadata_with_attempts(&db.read_pool).unwrap();
    assert_eq!(
        rows,
        vec![(
            "unprobed".to_string(),
            "D:\\videos\\a.mp4".to_string(),
            1i64
        )]
    );
}

/// Boundary of the exhausted listing: one below the limit is not listed, the
/// limit itself and anything beyond it are, and an offline row never is.
#[test]
fn exhausted_listing_only_includes_online_rows_at_or_above_the_limit() {
    for counter in [THUMBNAIL, METADATA] {
        let (_dir, db) = init_temp_db();
        {
            let conn = db.writer.lock().unwrap();
            insert_video(&conn, "below-limit", "D:\\videos\\below.mp4");
            insert_video(&conn, "at-limit", "D:\\videos\\at.mp4");
            insert_video(&conn, "above-limit", "D:\\videos\\above.mp4");
            insert_video_with_status(
                &conn,
                "offline-at-limit",
                "D:\\videos\\offline.mp4",
                "offline",
            );

            for _ in 0..(MAX_GENERATION_ATTEMPTS - 1) {
                (counter.increment)(&conn, "below-limit").unwrap();
            }
            for _ in 0..MAX_GENERATION_ATTEMPTS {
                (counter.increment)(&conn, "at-limit").unwrap();
            }
            for _ in 0..(MAX_GENERATION_ATTEMPTS + 2) {
                (counter.increment)(&conn, "above-limit").unwrap();
            }
            for _ in 0..MAX_GENERATION_ATTEMPTS {
                (counter.increment)(&conn, "offline-at-limit").unwrap();
            }
        }

        let rows = (counter.exhausted)(&db);
        let ids: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
        let name = counter.name;

        assert!(
            !ids.contains(&"below-limit"),
            "{name}: attempts one below the limit must not be reported as exhausted"
        );
        assert!(
            ids.contains(&"at-limit"),
            "{name}: attempts exactly at the limit must be reported as exhausted"
        );
        assert!(
            ids.contains(&"above-limit"),
            "{name}: attempts beyond the limit must still be reported as exhausted"
        );
        assert!(
            !ids.contains(&"offline-at-limit"),
            "{name}: an offline video must never be reported, regardless of its attempts count"
        );
        assert_eq!(ids.len(), 2, "{name}");

        let at_limit = rows.iter().find(|r| r.0 == "at-limit").unwrap();
        assert_eq!(at_limit.1, "D:\\videos\\at.mp4");
        assert_eq!(at_limit.2, "at.mp4");
        assert_eq!(at_limit.3, i64::from(MAX_GENERATION_ATTEMPTS), "{name}");
    }
}
