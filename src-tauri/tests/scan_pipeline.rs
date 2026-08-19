//! Integration test: validate -> quick_hash -> DB registration against real
//! tempdir files (both the target folder being scanned and the SQLite DB
//! itself), mixing a normal video file with a machine-dependent-char-named
//! one and confirming the DB ends up with exactly the expected split.

use std::fs;

use graybrowser_lib::{db, scan};

fn init_temp_db() -> (tempfile::TempDir, db::Db) {
    let dir = tempfile::tempdir().expect("failed to create tempdir");
    let db_path = dir.path().join("app.db");
    let db = db::init(&db_path).expect("db::init should succeed against a fresh tempdir file");
    (dir, db)
}

#[test]
fn scan_registers_normal_files_and_skips_machine_dependent_ones() {
    let (_db_dir, db) = init_temp_db();
    let scan_dir = tempfile::tempdir().expect("failed to create scan tempdir");
    fs::write(
        scan_dir.path().join("normal_movie.mp4"),
        b"fake video bytes",
    )
    .unwrap();
    fs::write(
        scan_dir.path().join("\u{2460}movie.mp4"),
        b"fake video bytes 2",
    )
    .unwrap();
    fs::write(
        scan_dir.path().join("notes.txt"),
        b"not a video, must be ignored entirely",
    )
    .unwrap();

    let folders = vec![scan_dir.path().to_string_lossy().to_string()];
    let summary = scan::scan_folders(&folders, &db).expect("scan should succeed");

    // notes.txt is filtered out before it ever reaches validate/hash, so
    // `scanned` only counts the two video-extension files.
    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.registered, 1);
    assert_eq!(summary.skipped, 1);

    let conn = db.writer.lock().unwrap();

    let video_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
        .unwrap();
    assert_eq!(video_count, 1);
    let (video_name, status, quick_hash): (String, String, String) = conn
        .query_row(
            "SELECT file_name, status, quick_hash FROM videos",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(video_name, "normal_movie.mp4");
    assert_eq!(status, "online");
    assert!(!quick_hash.is_empty());

    let skipped_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM skipped_files", [], |r| r.get(0))
        .unwrap();
    assert_eq!(skipped_count, 1);
    let (skipped_name, reason): (String, String) = conn
        .query_row("SELECT file_name, reason FROM skipped_files", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(skipped_name, "\u{2460}movie.mp4");
    assert_eq!(reason, "machine_dependent_char");
}

#[test]
fn rescanning_the_same_folder_is_idempotent() {
    let (_db_dir, db) = init_temp_db();
    let scan_dir = tempfile::tempdir().expect("failed to create scan tempdir");
    fs::write(scan_dir.path().join("movie.mp4"), b"bytes").unwrap();
    fs::write(scan_dir.path().join("\u{2460}movie.mp4"), b"bytes2").unwrap();
    let folders = vec![scan_dir.path().to_string_lossy().to_string()];

    scan::scan_folders(&folders, &db).unwrap();
    scan::scan_folders(&folders, &db).unwrap();

    let conn = db.writer.lock().unwrap();
    let video_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
        .unwrap();
    assert_eq!(video_count, 1);
    let skipped_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM skipped_files", [], |r| r.get(0))
        .unwrap();
    assert_eq!(skipped_count, 1);
}

/// Regression test for a bug found while adding logging: a file that
/// fails to open (originally found via a real exclusive lock during manual
/// testing) used to abort `scan_folders` entirely via `?`, instead of being
/// skipped per the documented policy ("safely skip and
/// retry on the next scan/restart"). Reproduces the failure deterministically
/// and in-process by opening the file with `share_mode(0)` (deny all
/// sharing), which makes `scan_folders`'s own `File::open` on the same path
/// fail with a real Windows sharing violation -- the same error observed
/// during manual testing with an externally locked file.
#[test]
fn scan_continues_past_a_file_it_cannot_open_and_registers_the_rest() {
    use std::os::windows::fs::OpenOptionsExt;

    let (_db_dir, db) = init_temp_db();
    let scan_dir = tempfile::tempdir().expect("failed to create scan tempdir");
    let locked_path = scan_dir.path().join("locked.mp4");
    fs::write(&locked_path, b"fake video bytes").unwrap();
    fs::write(scan_dir.path().join("normal.mp4"), b"fake video bytes 2").unwrap();

    let _lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&locked_path)
        .expect("failed to open the file exclusively for test setup");

    let folders = vec![scan_dir.path().to_string_lossy().to_string()];
    let summary = scan::scan_folders(&folders, &db)
        .expect("a single unreadable file must not abort the whole scan");

    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.registered, 1); // only normal.mp4
    assert_eq!(summary.skipped, 0); // locked.mp4 isn't machine-dependent, so not "skipped" either

    let conn = db.writer.lock().unwrap();
    let video_name: String = conn
        .query_row("SELECT file_name FROM videos", [], |r| r.get(0))
        .unwrap();
    assert_eq!(video_name, "normal.mp4");
}

/// A rescan of a file whose mtime+file_size haven't
/// changed must not recompute quick_hash -- `ScanSummary.unchanged` counts
/// it, `ScanSummary.reconciled`/`registered` do not, and the stored
/// quick_hash is byte-for-byte identical to what the first scan wrote.
#[test]
fn rescanning_an_unchanged_file_does_not_recompute_quick_hash() {
    let (_db_dir, db) = init_temp_db();
    let scan_dir = tempfile::tempdir().expect("failed to create scan tempdir");
    fs::write(scan_dir.path().join("movie.mp4"), b"stable bytes").unwrap();
    let folders = vec![scan_dir.path().to_string_lossy().to_string()];

    let first = scan::scan_folders(&folders, &db).unwrap();
    assert_eq!(first.registered, 1);

    let quick_hash_after_first: String = {
        let conn = db.writer.lock().unwrap();
        conn.query_row("SELECT quick_hash FROM videos", [], |r| r.get(0))
            .unwrap()
    };

    let second = scan::scan_folders(&folders, &db).unwrap();
    assert_eq!(second.scanned, 1);
    assert_eq!(second.registered, 0);
    assert_eq!(second.reconciled, 0);
    assert_eq!(second.unchanged, 1);

    let quick_hash_after_second: String = {
        let conn = db.writer.lock().unwrap();
        conn.query_row("SELECT quick_hash FROM videos", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(quick_hash_after_first, quick_hash_after_second);
}

/// A rescan of a known path whose content size changed
/// must recompute quick_hash and persist the new size/hash -- counted as
/// `reconciled`, not `unchanged` or a second `registered` row.
#[test]
fn rescanning_a_modified_file_updates_quick_hash_and_size() {
    let (_db_dir, db) = init_temp_db();
    let scan_dir = tempfile::tempdir().expect("failed to create scan tempdir");
    let video_path = scan_dir.path().join("movie.mp4");
    fs::write(&video_path, b"original bytes").unwrap();
    let folders = vec![scan_dir.path().to_string_lossy().to_string()];

    scan::scan_folders(&folders, &db).unwrap();
    let (id, quick_hash_before): (String, String) = {
        let conn = db.writer.lock().unwrap();
        conn.query_row("SELECT id, quick_hash FROM videos", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap()
    };

    // Different size guarantees a NeedsRehash classification even if the
    // filesystem's mtime resolution happens not to advance within the test.
    fs::write(&video_path, b"a completely different, longer set of bytes").unwrap();

    let second = scan::scan_folders(&folders, &db).unwrap();
    assert_eq!(second.registered, 0);
    assert_eq!(second.reconciled, 1);
    assert_eq!(second.unchanged, 0);

    let conn = db.writer.lock().unwrap();
    let video_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        video_count, 1,
        "content change must update the row in place, not insert a second one"
    );

    let (id_after, quick_hash_after, file_size_after): (String, String, i64) = conn
        .query_row("SELECT id, quick_hash, file_size FROM videos", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!(id_after, id, "id must be preserved across a content update");
    assert_ne!(quick_hash_after, quick_hash_before);
    assert_eq!(
        file_size_after,
        "a completely different, longer set of bytes".len() as i64
    );
}

/// A known row that went offline (simulated directly,
/// since the actual offline-detection sweep is implemented separately)
/// reconnects to `online` when its exact original path is scanned again, without
/// registering a second row or losing its id.
#[test]
fn rescanning_an_offline_rows_original_path_reconnects_it_to_online() {
    let (_db_dir, db) = init_temp_db();
    let scan_dir = tempfile::tempdir().expect("failed to create scan tempdir");
    fs::write(scan_dir.path().join("movie.mp4"), b"bytes").unwrap();
    let folders = vec![scan_dir.path().to_string_lossy().to_string()];

    scan::scan_folders(&folders, &db).unwrap();
    let id: String = {
        let conn = db.writer.lock().unwrap();
        conn.execute("UPDATE videos SET status = 'offline'", [])
            .unwrap();
        conn.query_row("SELECT id FROM videos", [], |r| r.get(0))
            .unwrap()
    };

    let second = scan::scan_folders(&folders, &db).unwrap();
    assert_eq!(second.registered, 0);
    assert_eq!(second.reconciled, 1);
    assert_eq!(second.unchanged, 0);

    let conn = db.writer.lock().unwrap();
    let (id_after, status_after): (String, String) = conn
        .query_row("SELECT id, status FROM videos", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(id_after, id);
    assert_eq!(status_after, "online");
}

/// Manual scan's missing-video detection is
/// the escape hatch for videos the NAS-polling ratio guard can't
/// self-resolve. This mirrors nas_poll.rs's
/// `a_lone_known_videos_disappearance_is_held_online_not_flipped_offline` --
/// with only one known video in the folder, its disappearance makes
/// discovered_paths empty, tripping decide_missing_video_ids'
/// NothingDiscovered guard rather than being treated as confirmed deletion.
#[test]
fn rescanning_after_a_lone_known_video_disappears_holds_it_online() {
    let (_db_dir, db) = init_temp_db();
    let scan_dir = tempfile::tempdir().expect("failed to create scan tempdir");
    let video_path = scan_dir.path().join("movie.mp4");
    fs::write(&video_path, b"bytes").unwrap();
    let folders = vec![scan_dir.path().to_string_lossy().to_string()];

    scan::scan_folders(&folders, &db).unwrap();
    fs::remove_file(&video_path).unwrap();
    let second = scan::scan_folders(&folders, &db).unwrap();

    assert_eq!(second.went_offline, 0);
    let conn = db.writer.lock().unwrap();
    assert_eq!(second.reactivated, 0);
    let (status, count): (String, i64) = (
        conn.query_row("SELECT status FROM videos", [], |r| r.get(0))
            .unwrap(),
        conn.query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
            .unwrap(),
    );
    assert_eq!(
        status, "online",
        "a lone missing file must be held, not flipped offline"
    );
    assert_eq!(count, 1, "the row must be kept regardless");
}

/// The flip side: a single file missing out of a large-enough library (1 of
/// 6, well under the 80% ratio threshold) is a confirmed, ordinary deletion
/// and transitions to offline as before the guard existed.
#[test]
fn rescanning_after_one_file_disappears_from_a_larger_library_flips_it_offline() {
    let (_db_dir, db) = init_temp_db();
    let scan_dir = tempfile::tempdir().expect("failed to create scan tempdir");
    let paths: Vec<_> = (0..6)
        .map(|i| scan_dir.path().join(format!("movie{i}.mp4")))
        .collect();
    for p in &paths {
        fs::write(p, b"bytes").unwrap();
    }
    let folders = vec![scan_dir.path().to_string_lossy().to_string()];

    scan::scan_folders(&folders, &db).unwrap();
    fs::remove_file(&paths[0]).unwrap();
    let second = scan::scan_folders(&folders, &db).unwrap();

    assert_eq!(second.went_offline, 1);
    let conn = db.writer.lock().unwrap();
    let online_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM videos WHERE status = 'online'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let offline_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM videos WHERE status = 'offline'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(online_count, 5);
    assert_eq!(offline_count, 1);
}

/// Two separate watch folders must never cross-contaminate: a video known
/// under folder A must not be judged missing just because folder B's walk
/// didn't discover it (it was never expected to -- it isn't under B at
/// all). Exercises the per-folder scoping in `scan_folders`/
/// `reconcile_missing_videos` directly, per the explicit review request.
#[test]
fn missing_detection_does_not_cross_contaminate_between_folders() {
    let (_db_dir, db) = init_temp_db();
    let folder_a = tempfile::tempdir().expect("failed to create folder A");
    let folder_b = tempfile::tempdir().expect("failed to create folder B");
    // Folder A needs >= 5 known videos so a single deletion there stays
    // below the ratio guard and actually proceeds to "missing" -- this
    // test's whole point is to prove that proceeding is scoped to A only.
    let a_paths: Vec<_> = (0..6)
        .map(|i| folder_a.path().join(format!("a{i}.mp4")))
        .collect();
    for p in &a_paths {
        fs::write(p, b"bytes").unwrap();
    }
    fs::write(folder_b.path().join("b0.mp4"), b"bytes").unwrap();
    let folders = vec![
        folder_a.path().to_string_lossy().to_string(),
        folder_b.path().to_string_lossy().to_string(),
    ];

    scan::scan_folders(&folders, &db).unwrap();
    fs::remove_file(&a_paths[0]).unwrap();
    let second = scan::scan_folders(&folders, &db).unwrap();

    // Only a0.mp4 (under folder A) should go offline; folder B's single
    // video must be untouched by folder A's walk results.
    assert_eq!(second.went_offline, 1);
    let conn = db.writer.lock().unwrap();
    let b_status: String = conn
        .query_row(
            "SELECT status FROM videos WHERE file_name = 'b0.mp4'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(b_status, "online");
}

/// Path-follow: a file with the same
/// quick_hash+file_size as a known `offline` row, discovered at a brand-new
/// path, is followed (`file_path` rewritten, `id` preserved) rather than
/// registered as a second row.
#[test]
fn a_file_reappearing_at_a_new_path_is_path_followed_not_re_registered() {
    let (_db_dir, db) = init_temp_db();
    let old_dir = tempfile::tempdir().expect("failed to create old-location tempdir");
    let new_dir = tempfile::tempdir().expect("failed to create new-location tempdir");
    let old_path = old_dir.path().join("movie.mp4");
    fs::write(&old_path, b"identical bytes").unwrap();

    let old_folders = vec![old_dir.path().to_string_lossy().to_string()];
    scan::scan_folders(&old_folders, &db).unwrap();
    let id_before: String = {
        let conn = db.writer.lock().unwrap();
        conn.execute("UPDATE videos SET status = 'offline'", [])
            .unwrap();
        conn.query_row("SELECT id FROM videos", [], |r| r.get(0))
            .unwrap()
    };

    // The file moves to a brand-new folder with byte-identical content (same
    // quick_hash+file_size). Scanning only the new folder means the new path
    // is genuinely unknown to the DB, so this must go through path-follow,
    // not a fresh insert.
    fs::remove_file(&old_path).unwrap();
    let new_path = new_dir.path().join("movie.mp4");
    fs::write(&new_path, b"identical bytes").unwrap();
    let new_folders = vec![new_dir.path().to_string_lossy().to_string()];

    let summary = scan::scan_folders(&new_folders, &db).unwrap();
    assert_eq!(summary.reactivated, 1);
    assert_eq!(summary.registered, 0);
    assert_eq!(summary.collisions, 0);

    let conn = db.writer.lock().unwrap();
    let video_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
        .unwrap();
    assert_eq!(video_count, 1, "the move must not create a second row");
    let (id_after, path_after, status_after): (String, String, String) = conn
        .query_row("SELECT id, file_path, status FROM videos", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!(id_after, id_before, "id must be preserved across the move");
    assert_eq!(path_after, new_path.to_string_lossy().to_string());
    assert_eq!(status_after, "online");
}

/// `scan_folders`/`process_detected_file` (via `WalkDir` + `File::open`
/// for quick_hash) must succeed against a folder deeper than `MAX_PATH`
/// (260 chars) without requiring the machine-wide `LongPathsEnabled`
/// registry opt-in. The fixture tree itself is built through
/// `long_path::to_long_path` for the same reason -- `create_dir_all` is
/// just as subject to `MAX_PATH` as the code under test, so a CI machine
/// without the registry opt-in must be able to construct this fixture at
/// all.
///
/// This also confirms the DB's `file_path`
/// must come back in **plain** form, never `\\?\`-prefixed, even though the
/// walk that discovered it used a prefixed root internally.
#[test]
fn scanning_a_folder_deeper_than_max_path_succeeds_and_stores_a_plain_file_path() {
    use graybrowser_lib::adapters::long_path;

    let (_db_dir, db) = init_temp_db();
    let scan_dir = tempfile::tempdir().expect("failed to create scan tempdir");

    let mut deep = scan_dir.path().to_path_buf();
    for i in 0..10 {
        deep = deep.join(format!("segment_{i:03}_a_long_deep_directory_name"));
    }
    fs::create_dir_all(long_path::to_long_path(&deep))
        .expect("deep dir creation should succeed via \\\\?\\ prefixing");

    let video_path = deep.join("deeply_nested_movie.mp4");
    assert!(
        video_path.to_string_lossy().len() > 260,
        "fixture path must actually exceed MAX_PATH to be a meaningful test: {} chars",
        video_path.to_string_lossy().len()
    );
    fs::write(long_path::to_long_path(&video_path), b"deep video bytes")
        .expect("writing the fixture file should succeed via \\\\?\\ prefixing");

    let folders = vec![scan_dir.path().to_string_lossy().to_string()];
    let summary = scan::scan_folders(&folders, &db).expect("scan should succeed past MAX_PATH");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.registered, 1);

    let conn = db.writer.lock().unwrap();
    let (file_path, quick_hash): (String, String) = conn
        .query_row("SELECT file_path, quick_hash FROM videos", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(
        file_path,
        video_path.to_string_lossy().to_string(),
        "file_path must be stored in plain form, not \\\\?\\-prefixed"
    );
    assert!(
        !file_path.starts_with(r"\\?\"),
        "DB file_path must never carry the \\\\?\\ prefix"
    );
    assert!(
        !quick_hash.is_empty(),
        "quick_hash must have been computed via File::open past MAX_PATH"
    );

    // Rescanning the same deep folder must also work end-to-end past
    // MAX_PATH (WalkDir root prefixing + entry-path stripping stay
    // consistent across repeated walks) and remain idempotent.
    let summary2 =
        scan::scan_folders(&folders, &db).expect("rescan should also succeed past MAX_PATH");
    assert_eq!(summary2.unchanged, 1);
    assert_eq!(summary2.registered, 0);
    let video_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
        .unwrap();
    assert_eq!(video_count, 1);
}
