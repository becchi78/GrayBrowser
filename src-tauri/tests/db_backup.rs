//! Backups against real tempdir-backed SQLite files.

use std::path::Path;

use gb_core::backup::{backup_file_name, BackupKind};
use graybrowser_lib::db;
use graybrowser_lib::db::backup::{create_backup, KEEP_MANUAL, KEEP_PREMIGRATE};

fn list_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn seed_video(db: &db::Db, id: &str) {
    db.writer
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status)
             VALUES (?1, 'D:\\v\\a.mp4', 'a.mp4', 1, 'qh', 'online')",
            [id],
        )
        .unwrap();
}

fn video_ids(conn: &rusqlite::Connection) -> Vec<String> {
    let mut stmt = conn.prepare("SELECT id FROM videos ORDER BY id").unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// Builds a database at schema version 7 (one pending migration) holding a
/// single video row.
fn create_v7_database(db_path: &Path) {
    let conn = rusqlite::Connection::open(db_path).unwrap();
    conn.execute_batch(include_str!("../migrations/0001_initial.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../migrations/0002_add_mtime.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../migrations/0003_add_kana_roma.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../migrations/0004_add_video_metadata.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../migrations/0005_add_sort_indexes.sql"))
        .unwrap();
    conn.execute_batch(include_str!("../migrations/0006_add_path_collisions.sql"))
        .unwrap();
    conn.execute_batch(include_str!(
        "../migrations/0007_add_generation_retry_columns.sql"
    ))
    .unwrap();
    for version in 1..=7 {
        conn.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            [version],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status)
         VALUES ('legacy', 'D:\\v\\a.mp4', 'a.mp4', 1, 'qh', 'online')",
        [],
    )
    .unwrap();
}

#[test]
fn manual_backup_is_created_and_readable() {
    let dir = tempfile::tempdir().unwrap();
    let db = db::init(&dir.path().join("app.db")).unwrap();
    seed_video(&db, "v1");

    let backups = dir.path().join("backups");
    let conn = db.read_pool.get().unwrap();
    let path = create_backup(&conn, &backups, BackupKind::Manual, 1_700_000_000).unwrap();

    assert_eq!(
        path.file_name().unwrap().to_str().unwrap(),
        backup_file_name(BackupKind::Manual, 1_700_000_000)
    );
    assert_eq!(list_names(&backups).len(), 1, "no temp file left behind");
    let restored = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(video_ids(&restored), vec!["v1"]);
}

#[test]
fn premigrate_backup_is_taken_only_when_migrations_are_pending() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("app.db");
    let backups = dir.path().join("backups");
    create_v7_database(&db_path);

    drop(db::init(&db_path).unwrap());
    let names = list_names(&backups);
    assert_eq!(names.len(), 1);
    assert!(names[0].starts_with("app-premigrate-"));
    // The backup holds the pre-migration state (version 7, data intact).
    let snapshot = rusqlite::Connection::open(backups.join(&names[0])).unwrap();
    let version: i64 = snapshot
        .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 7);
    assert_eq!(video_ids(&snapshot), vec!["legacy"]);

    // Already up to date: no further backup.
    drop(db::init(&db_path).unwrap());
    assert_eq!(list_names(&backups), names);
}

#[test]
fn no_premigrate_backup_for_a_brand_new_database() {
    let dir = tempfile::tempdir().unwrap();
    drop(db::init(&dir.path().join("app.db")).unwrap());
    assert!(!dir.path().join("backups").exists());
}

#[test]
fn corrupted_database_can_be_recovered_from_a_backup() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("app.db");
    let backups = dir.path().join("backups");

    let backup_path = {
        let db = db::init(&db_path).unwrap();
        seed_video(&db, "precious");
        let conn = db.read_pool.get().unwrap();
        create_backup(&conn, &backups, BackupKind::Manual, 1_700_000_000).unwrap()
    };

    // Corrupt the live database and drop its WAL sidecars, as the restore
    // procedure instructs.
    std::fs::write(&db_path, vec![0xAB; 8192]).unwrap();
    for ext in ["db-wal", "db-shm"] {
        let _ = std::fs::remove_file(dir.path().join(format!("app.{ext}")));
    }
    assert!(db::init(&db_path).is_err(), "corrupt file must not open");

    std::fs::copy(&backup_path, &db_path).unwrap();
    let db = db::init(&db_path).expect("restored backup should open");
    let conn = db.read_pool.get().unwrap();
    assert_eq!(video_ids(&conn), vec!["precious"]);
}

#[test]
fn failed_backup_leaves_no_file_and_keeps_existing_generations() {
    let dir = tempfile::tempdir().unwrap();
    let backups = dir.path().join("backups");
    std::fs::create_dir_all(&backups).unwrap();
    for i in 0..KEEP_MANUAL as i64 {
        std::fs::write(
            backups.join(backup_file_name(BackupKind::Manual, 1000 + i)),
            b"x",
        )
        .unwrap();
    }
    let before = list_names(&backups);

    let garbage = dir.path().join("garbage.db");
    std::fs::write(&garbage, vec![0xCD; 8192]).unwrap();
    let conn = rusqlite::Connection::open(&garbage).unwrap();

    let result = create_backup(&conn, &backups, BackupKind::Manual, 2000);
    assert!(result.is_err());
    assert_eq!(list_names(&backups), before);
}

#[test]
fn old_generations_are_pruned_per_kind() {
    let dir = tempfile::tempdir().unwrap();
    let db = db::init(&dir.path().join("app.db")).unwrap();
    let backups = dir.path().join("backups");
    let conn = db.read_pool.get().unwrap();

    for i in 0..(KEEP_PREMIGRATE as i64 + 2) {
        create_backup(&conn, &backups, BackupKind::Premigrate, 1000 + i).unwrap();
    }
    for i in 0..(KEEP_MANUAL as i64 + 3) {
        create_backup(&conn, &backups, BackupKind::Manual, 5000 + i).unwrap();
    }

    let names = list_names(&backups);
    let premigrate: Vec<_> = names
        .iter()
        .filter(|n| n.contains("-premigrate-"))
        .collect();
    let manual: Vec<_> = names.iter().filter(|n| n.contains("-manual-")).collect();
    assert_eq!(premigrate.len(), KEEP_PREMIGRATE);
    assert_eq!(manual.len(), KEEP_MANUAL);
    // The oldest of each kind went first.
    assert!(!names.contains(&backup_file_name(BackupKind::Premigrate, 1000)));
    assert!(names.contains(&backup_file_name(
        BackupKind::Premigrate,
        1000 + KEEP_PREMIGRATE as i64 + 1
    )));
    assert!(!names.contains(&backup_file_name(BackupKind::Manual, 5000)));
}
