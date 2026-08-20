//! Integration tests for `paths::migrate_legacy_nested_app_dir` -- the
//! one-time migration that moves real user data (`app.db`, `thumbnails/`,
//! `logs/`) out of a pre-existing double-nested
//! `<app_dir>\GrayBrowser\` folder left behind by builds where
//! `resolve_app_dir` unconditionally appended a `GrayBrowser` subfolder even
//! when the executable already lived inside one.

use std::fs;
use std::path::Path;

use graybrowser_lib::paths::migrate_legacy_nested_app_dir;

/// Writes `contents` to `dir/name`, creating `dir` (and any missing
/// ancestors) first.
fn write_file(dir: &Path, name: &str, contents: &str) {
    fs::create_dir_all(dir).expect("failed to create parent directory for fixture file");
    fs::write(dir.join(name), contents).expect("failed to write fixture file");
}

#[test]
fn moves_the_database_thumbnails_and_logs_out_of_the_legacy_nested_folder() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    let app_dir = tmp.path();
    let old_dir = app_dir.join("GrayBrowser");

    write_file(&old_dir, "app.db", "db-contents");
    write_file(&old_dir, "app.db-wal", "wal-contents");
    write_file(&old_dir, "app.db-shm", "shm-contents");
    write_file(&old_dir.join("thumbnails"), "abc.webp", "thumb-contents");
    write_file(&old_dir.join("logs"), "app_rCURRENT.log", "log-contents");

    migrate_legacy_nested_app_dir(app_dir).expect("migration should succeed");

    assert_eq!(
        fs::read_to_string(app_dir.join("app.db")).unwrap(),
        "db-contents"
    );
    assert_eq!(
        fs::read_to_string(app_dir.join("app.db-wal")).unwrap(),
        "wal-contents"
    );
    assert_eq!(
        fs::read_to_string(app_dir.join("app.db-shm")).unwrap(),
        "shm-contents"
    );
    assert_eq!(
        fs::read_to_string(app_dir.join("thumbnails").join("abc.webp")).unwrap(),
        "thumb-contents"
    );
    assert_eq!(
        fs::read_to_string(app_dir.join("logs").join("app_rCURRENT.log")).unwrap(),
        "log-contents"
    );

    // The now-empty legacy folder was cleaned up.
    assert!(!old_dir.exists());
}

#[test]
fn is_idempotent_once_the_new_location_already_has_a_database() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    let app_dir = tmp.path();

    // Simulate an already-migrated install: the new location has the real
    // database, and (for whatever reason -- e.g. a leftover from a partial
    // previous run) the old nested folder still exists with its own,
    // different `app.db`.
    write_file(app_dir, "app.db", "already-migrated-db");
    let old_dir = app_dir.join("GrayBrowser");
    write_file(&old_dir, "app.db", "stale-legacy-db");

    migrate_legacy_nested_app_dir(app_dir).expect("migration should succeed as a no-op");

    // The already-present database at the new location must not be
    // clobbered by the stale legacy one.
    assert_eq!(
        fs::read_to_string(app_dir.join("app.db")).unwrap(),
        "already-migrated-db"
    );
    // Nothing was touched: the (irrelevant, already-superseded) legacy
    // folder is left exactly as it was.
    assert_eq!(
        fs::read_to_string(old_dir.join("app.db")).unwrap(),
        "stale-legacy-db"
    );
}

#[test]
fn migrates_the_database_even_when_some_non_database_entries_are_missing() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    let app_dir = tmp.path();
    let old_dir = app_dir.join("GrayBrowser");

    // No `logs/` this time -- e.g. logging failed to initialize on a prior
    // run, so the folder was never created.
    write_file(&old_dir, "app.db", "db-contents");
    write_file(&old_dir.join("thumbnails"), "abc.webp", "thumb-contents");

    migrate_legacy_nested_app_dir(app_dir).expect("migration should succeed");

    assert_eq!(
        fs::read_to_string(app_dir.join("app.db")).unwrap(),
        "db-contents"
    );
    assert_eq!(
        fs::read_to_string(app_dir.join("thumbnails").join("abc.webp")).unwrap(),
        "thumb-contents"
    );
    assert!(!app_dir.join("logs").exists());
    assert!(!old_dir.exists());
}

#[test]
fn merges_legacy_logs_into_a_new_logs_folder_pre_created_by_logging_init() {
    // Regression test for the actual lib.rs call order:
    // `create_dir_all(&app_dir)` -> `logging::init(&app_dir)` ->
    // `migrate_legacy_nested_app_dir(&app_dir)` -> `db::init(...)`.
    // `logging::init` synchronously creates `app_dir/logs/` via
    // flexi_logger's `FileSpec::directory(...)` *before* the migration
    // runs, so by the time the migration inspects the legacy `logs/`
    // folder, an `app_dir/logs/` destination already exists. A naive
    // "destination already exists -> already migrated, skip" check would
    // wrongly leave the legacy log files stranded forever -- this asserts
    // they get merged into the new `logs/` instead.
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    let app_dir = tmp.path();
    let old_dir = app_dir.join("GrayBrowser");

    write_file(&old_dir, "app.db", "db-contents");
    write_file(
        &old_dir.join("logs"),
        "app_r00000.log",
        "legacy-log-contents",
    );

    // Reproduce lib.rs's call order exactly: logging::init runs, and only
    // then does the migration. `logging::init` never fails the caller (it
    // downgrades any error to a stderr warning and returns `None`), so this
    // is safe to call from a test even if another test in this binary
    // already initialized the global logger.
    let _handle = graybrowser_lib::logging::init(app_dir);
    assert!(
        app_dir.join("logs").is_dir(),
        "logging::init should have pre-created app_dir/logs/"
    );

    migrate_legacy_nested_app_dir(app_dir).expect("migration should succeed");

    assert_eq!(
        fs::read_to_string(app_dir.join("logs").join("app_r00000.log")).unwrap(),
        "legacy-log-contents"
    );
    assert_eq!(
        fs::read_to_string(app_dir.join("app.db")).unwrap(),
        "db-contents"
    );
    // The legacy nested folder is now fully empty and was cleaned up.
    assert!(!old_dir.exists());
}

#[test]
fn does_nothing_for_a_fresh_install_with_no_database_anywhere() {
    let tmp = tempfile::tempdir().expect("failed to create tempdir");
    let app_dir = tmp.path();

    migrate_legacy_nested_app_dir(app_dir).expect("migration should succeed as a no-op");

    assert!(!app_dir.join("app.db").exists());
    assert!(!app_dir.join("GrayBrowser").exists());
}
