//! Helpers shared by the integration tests. Every file under `tests/` is its
//! own crate, so `db::test_support` (compiled only for the library's own unit
//! tests) is out of reach; this module is pulled in with `mod common;`.
//!
//! Each test crate uses only a subset, so the unused remainder must not trip
//! `dead_code` under `-D warnings`.
#![allow(dead_code)]

use std::sync::Arc;

use graybrowser_lib::adapters::folder_probe::RealFolderProbe;
use graybrowser_lib::db;
use graybrowser_lib::events::NoopReachabilityNotifier;
use graybrowser_lib::reachability::ReachabilityRegistry;
use rusqlite::{params, Connection};

/// A fresh, fully migrated database in a tempdir. Hold the returned
/// `TempDir` for as long as the database is used.
pub fn init_temp_db() -> (tempfile::TempDir, db::Db) {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");
    let db = db::init(&db_path).expect("db::init should succeed against a fresh tempdir file");
    (dir, db)
}

/// A throwaway `thumbnails/` root: scenarios that do not look at thumbnail
/// placement only need a valid, empty directory to pass down.
pub fn temp_thumbs_root() -> tempfile::TempDir {
    tempfile::tempdir().expect("failed to create thumbs root tempdir")
}

/// Probes the real filesystem, so a tempdir stands in for a share and
/// removing it simulates the share going away.
pub fn real_registry() -> ReachabilityRegistry {
    ReachabilityRegistry::new(
        Arc::new(RealFolderProbe),
        Arc::new(NoopReachabilityNotifier),
    )
}

/// The last path component of a Windows-style path, as the scanner stores it
/// in `videos.file_name`.
fn file_name_of(file_path: &str) -> &str {
    file_path.rsplit('\\').next().unwrap_or(file_path)
}

/// A `videos` row to insert via raw SQL, bypassing `queries::insert_video` so
/// each test controls `status`/`quick_hash`/`mtime`/`created_at` directly.
/// Defaults: online, 1024 bytes, `quick_hash` "hash", rating 0,
/// `created_at` "2026-01-01 00:00:00" (fixed, so ordering never depends on
/// the wall clock), no `mtime`/`kana`/`roma`.
pub struct NewVideo<'a> {
    id: &'a str,
    file_path: &'a str,
    status: &'a str,
    quick_hash: &'a str,
    file_size: i64,
    rating: i64,
    created_at: &'a str,
    mtime: Option<i64>,
    kana: Option<&'a str>,
    roma: Option<&'a str>,
}

pub fn video<'a>(id: &'a str, file_path: &'a str) -> NewVideo<'a> {
    NewVideo {
        id,
        file_path,
        status: "online",
        quick_hash: "hash",
        file_size: 1024,
        rating: 0,
        created_at: "2026-01-01 00:00:00",
        mtime: None,
        kana: None,
        roma: None,
    }
}

impl<'a> NewVideo<'a> {
    pub fn status(mut self, status: &'a str) -> Self {
        self.status = status;
        self
    }

    pub fn quick_hash(mut self, quick_hash: &'a str) -> Self {
        self.quick_hash = quick_hash;
        self
    }

    pub fn file_size(mut self, file_size: i64) -> Self {
        self.file_size = file_size;
        self
    }

    pub fn rating(mut self, rating: i64) -> Self {
        self.rating = rating;
        self
    }

    pub fn created_at(mut self, created_at: &'a str) -> Self {
        self.created_at = created_at;
        self
    }

    pub fn mtime(mut self, mtime: Option<i64>) -> Self {
        self.mtime = mtime;
        self
    }

    pub fn kana_roma(mut self, kana: Option<&'a str>, roma: Option<&'a str>) -> Self {
        self.kana = kana;
        self.roma = roma;
        self
    }

    pub fn insert(self, conn: &Connection) {
        conn.execute(
            "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status,
                                 rating, created_at, mtime, kana, roma)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                self.id,
                self.file_path,
                file_name_of(self.file_path),
                self.file_size,
                self.quick_hash,
                self.status,
                self.rating,
                self.created_at,
                self.mtime,
                self.kana,
                self.roma,
            ],
        )
        .unwrap();
    }
}

/// An online video with every other column at its `video()` default.
pub fn insert_video(conn: &Connection, id: &str, file_path: &str) {
    video(id, file_path).insert(conn);
}

pub fn insert_video_with_status(conn: &Connection, id: &str, file_path: &str, status: &str) {
    video(id, file_path).status(status).insert(conn);
}

pub fn insert_tag(conn: &Connection, name: &str) -> i64 {
    conn.execute("INSERT INTO tags (name) VALUES (?1)", params![name])
        .unwrap();
    conn.last_insert_rowid()
}

pub fn assign_tag(conn: &Connection, video_id: &str, tag_id: i64) {
    conn.execute(
        "INSERT INTO video_tags (video_id, tag_id) VALUES (?1, ?2)",
        params![video_id, tag_id],
    )
    .unwrap();
}

pub fn video_ids(conn: &Connection) -> Vec<String> {
    let mut stmt = conn.prepare("SELECT id FROM videos ORDER BY id").unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

pub fn insert_skipped_file(conn: &Connection, file_path: &str) {
    conn.execute(
        "INSERT INTO skipped_files (file_path, file_name, reason, detected_char)
         VALUES (?1, ?2, 'unsupported_char', '①')",
        params![file_path, file_name_of(file_path)],
    )
    .unwrap();
}

pub fn skipped_paths(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT file_path FROM skipped_files ORDER BY file_path")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

/// Tag names in the order `list_all_tags` returns them.
pub fn tag_names(db: &db::Db) -> Vec<String> {
    db::queries::list_all_tags(&db.read_pool)
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect()
}

/// `(id, sort_order)` of every tag, by id.
pub fn sort_orders(conn: &Connection) -> Vec<(i64, Option<i64>)> {
    conn.prepare("SELECT id, sort_order FROM tags ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// The migration scripts, in version order. Read straight from the SQL files
/// rather than from `db::migrations::MIGRATIONS`, so a dropped or reordered
/// entry in the production list cannot silently change what these fixtures
/// build.
const MIGRATION_SQL: &[&str] = &[
    include_str!("../../migrations/0001_initial.sql"),
    include_str!("../../migrations/0002_add_mtime.sql"),
    include_str!("../../migrations/0003_add_kana_roma.sql"),
    include_str!("../../migrations/0004_add_video_metadata.sql"),
    include_str!("../../migrations/0005_add_sort_indexes.sql"),
    include_str!("../../migrations/0006_add_path_collisions.sql"),
    include_str!("../../migrations/0007_add_generation_retry_columns.sql"),
    include_str!("../../migrations/0008_add_thumbnail_ready.sql"),
    include_str!("../../migrations/0009_add_skipped_files_dismissed.sql"),
    include_str!("../../migrations/0010_add_tags_sort_order.sql"),
    include_str!("../../migrations/0011_create_wb_unmatched_thumbnails.sql"),
];

/// The schema version a freshly created database ends at.
pub fn latest_schema_version() -> i64 {
    db::migrations::MIGRATIONS.len() as i64
}

/// Applies migrations `1..=version` to `conn` and records each in
/// `schema_version`, reproducing a database file last opened by the app at
/// that version.
pub fn apply_up_to(conn: &Connection, version: usize) {
    assert!(
        (1..=MIGRATION_SQL.len()).contains(&version),
        "no such migration version: {version}"
    );
    for sql in &MIGRATION_SQL[..version] {
        conn.execute_batch(sql).unwrap();
    }
    // 0001 creates `schema_version` itself.
    for v in 1..=version {
        conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            [v as i64],
        )
        .unwrap();
    }
}

/// True for the 8 lowercase hex characters a `.wb` movie hash consists of.
pub fn is_lowercase_hex8(hash: &str) -> bool {
    hash.len() == 8
        && hash
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}
