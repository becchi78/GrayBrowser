use super::process::{reconcile_known_path, register_new_path};
use super::*;
use crate::thumbnail;
use gb_core::scan_pipeline::ScannedFile;
use std::fs::File;

/// A catalog with folders to register videos in and move them between, behind a
/// registry whose probe the test controls. Shared by the tests of every
/// detection path.
pub(crate) mod move_fixture {
    use super::*;
    use crate::db::test_support::init_temp_db;
    use crate::test_support::registry_over;
    use gb_core::testing::fake_folder_probe::FakeFolderProbe;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Arc;

    pub const CONTENT: &[u8] = b"the same video bytes";

    pub struct MoveFixture {
        pub db: Db,
        pub thumbs: tempfile::TempDir,
        pub dirs: Vec<tempfile::TempDir>,
        pub probe: Arc<FakeFolderProbe>,
        pub registry: ReachabilityRegistry,
        pub watch_folders: Vec<String>,
        _db_dir: tempfile::TempDir,
    }

    impl MoveFixture {
        pub fn new(folder_count: usize) -> Self {
            let (db_dir, db) = init_temp_db();
            let dirs: Vec<_> = (0..folder_count)
                .map(|_| tempfile::tempdir().unwrap())
                .collect();
            let watch_folders: Vec<String> = dirs
                .iter()
                .map(|d| d.path().to_string_lossy().to_string())
                .collect();
            queries::set_watch_folders(&db.writer.lock().unwrap(), &watch_folders).unwrap();
            let probe = Arc::new(FakeFolderProbe::default());
            let registry = registry_over(probe.clone());
            Self {
                db,
                thumbs: tempfile::tempdir().unwrap(),
                dirs,
                probe,
                registry,
                watch_folders,
                _db_dir: db_dir,
            }
        }

        pub fn folder(&self, i: usize) -> &str {
            &self.watch_folders[i]
        }

        /// Writes `name` into folder `i` and registers it through a manual scan
        /// that never consults reachability, so the registry stays unprobed.
        pub fn register_at(&self, i: usize, name: &str) -> PathBuf {
            let path = self.write_at(i, name);
            scan_folders(
                &[self.folder(i).to_string()],
                &self.db,
                self.thumbs.path(),
                &AssumeReachable,
            )
            .unwrap();
            path
        }

        pub fn write_at(&self, i: usize, name: &str) -> PathBuf {
            let path = self.dirs[i].path().join(name);
            fs::write(&path, CONTENT).unwrap();
            path
        }

        pub fn id_at(&self, path: &Path) -> String {
            let conn = self.db.read_pool.get().unwrap();
            queries::find_video_by_path(&conn, &path.to_string_lossy())
                .unwrap()
                .unwrap()
                .id
        }

        pub fn rows(&self) -> Vec<(String, String)> {
            let conn = self.db.read_pool.get().unwrap();
            let mut stmt = conn
                .prepare("SELECT file_path, status FROM videos ORDER BY file_path")
                .unwrap();
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows
        }

        pub fn tag(&self, id: &str, name: &str) {
            let conn = self.db.writer.lock().unwrap();
            conn.execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", [name])
                .unwrap();
            conn.execute(
                "INSERT INTO video_tags (video_id, tag_id)
                 SELECT ?1, id FROM tags WHERE name = ?2",
                [id, name],
            )
            .unwrap();
        }

        pub fn tag_count(&self, id: &str) -> i64 {
            let conn = self.db.read_pool.get().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM video_tags WHERE video_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap()
        }

        /// Where `id`'s first thumbnail slot lives for a file at `path`.
        pub fn thumbnail_slot(&self, id: &str, path: &Path) -> PathBuf {
            thumbnail::paths::video_thumbnail_dir(
                self.thumbs.path(),
                &self.watch_folders,
                &path.to_string_lossy(),
            )
            .join(format!("{id}_0.webp"))
        }

        pub fn put_thumbnail(&self, id: &str, path: &Path) {
            let slot = self.thumbnail_slot(id, path);
            fs::create_dir_all(slot.parent().unwrap()).unwrap();
            fs::write(slot, "thumb").unwrap();
        }
    }
}

use crate::db::test_support::init_temp_db;
use std::fs;

/// A throwaway `thumbnails/` root for tests that don't care about its
/// contents -- none of the scenarios below reactivate a video that
/// actually has cached thumbnails to move, so this only needs to be a
/// valid, empty directory.
fn temp_thumbs_root() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// A path-follow target that's already claimed by a *different*,
/// currently-online row. A single scan can't produce this: with no
/// concurrent writer, `process_detected_file`'s `find_video_by_path`
/// routes any already-known path to `reconcile_known_path` before
/// `register_new_path` runs. It is only reachable through a race, where
/// another detection path registers the row between that lookup and the
/// writer-lock section. `register_new_path` is called directly here,
/// bypassing the lookup, to construct that state and prove the collision
/// guard doesn't panic and leaves both existing rows exactly as they were.
///
/// Also proves a corollary of `file_path` being `UNIQUE`: the "new file"
/// can never actually be registered as a separate row at the path the
/// online row (`r2_id`) already occupies (an insert there is structurally
/// impossible, not merely undesirable) -- the DB ends up with exactly the
/// original two rows, unchanged, not three.
#[test]
fn register_new_path_leaves_both_rows_untouched_when_the_target_path_collides_with_an_online_row() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();

    // `r1_id`: an offline candidate for path-follow.
    let old_dir = tempfile::tempdir().unwrap();
    let old_path = old_dir.path().join("old.mp4");
    fs::write(&old_path, b"shared content").unwrap();
    let old_folders = vec![old_dir.path().to_string_lossy().to_string()];
    scan_folders(&old_folders, &db, thumbs_root.path(), &AssumeReachable).unwrap();
    let r1_id: String = {
        let conn = db.writer.lock().unwrap();
        conn.execute("UPDATE videos SET status = 'offline'", [])
            .unwrap();
        conn.query_row("SELECT id FROM videos", [], |r| r.get(0))
            .unwrap()
    };

    // `r2_id`: an online row already claiming the path we're about to target.
    let new_dir = tempfile::tempdir().unwrap();
    let new_path = new_dir.path().join("new.mp4");
    fs::write(&new_path, b"different content, a different length").unwrap();
    let new_folders = vec![new_dir.path().to_string_lossy().to_string()];
    scan_folders(&new_folders, &db, thumbs_root.path(), &AssumeReachable).unwrap();
    let r2_id: String = {
        let conn = db.writer.lock().unwrap();
        conn.query_row(
            "SELECT id FROM videos WHERE file_path = ?1",
            [new_path.to_string_lossy().to_string()],
            |r| r.get(0),
        )
        .unwrap()
    };

    // Overwrite new.mp4's bytes so its quick_hash+file_size now match
    // the offline row's -- the physical trigger for a path-follow match --
    // while its path is still the online row's already-registered path.
    fs::write(&new_path, b"shared content").unwrap();
    let metadata = fs::metadata(&new_path).unwrap();
    let scanned_file = ScannedFile {
        file_path: new_path.to_string_lossy().to_string(),
        file_name: "new.mp4".to_string(),
        file_size: metadata.len(),
        mtime: mtime_from_metadata(&metadata).unwrap(),
    };

    let outcome =
        register_new_path(&db, thumbs_root.path(), &scanned_file, &AssumeReachable).unwrap();
    match outcome {
        ProcessOutcome::BlockedByCollision {
            video_id,
            colliding_video_id,
        } => {
            assert_eq!(video_id, r1_id);
            assert_eq!(colliding_video_id, r2_id);
        }
        _ => panic!("expected BlockedByCollision"),
    }

    let conn = db.writer.lock().unwrap();
    let (r1_status, r1_path): (String, String) = conn
        .query_row(
            "SELECT status, file_path FROM videos WHERE id = ?1",
            [&r1_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        r1_status, "offline",
        "R1 must stay offline, not be rewritten"
    );
    assert_eq!(r1_path, old_path.to_string_lossy().to_string());

    let (r2_status, r2_path): (String, String) = conn
        .query_row(
            "SELECT status, file_path FROM videos WHERE id = ?1",
            [&r2_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(r2_status, "online", "R2 must be untouched");
    assert_eq!(r2_path, new_path.to_string_lossy().to_string());

    let total_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        total_count, 2,
        "file_path is UNIQUE -- a third row at R2's exact path is structurally \
             impossible, so no insert happens at all; the file is left unregistered \
             this pass"
    );

    // The collision must also be persisted to `path_collisions`
    // (queries::record_path_collision), not merely WARN-logged -- this
    // is what makes it visible to `dedup::detect_duplicate_groups`
    // later.
    let collisions = queries::list_path_collisions(&db.read_pool).unwrap();
    assert_eq!(
        collisions.len(),
        1,
        "the collision must be recorded exactly once"
    );
    assert_eq!(collisions[0].video_id, r1_id);
    assert_eq!(collisions[0].colliding_video_id, r2_id);
    assert_eq!(
        collisions[0].attempted_path,
        new_path.to_string_lossy().to_string()
    );
}

/// A `known` (already-registered) row (`r2_id`) has its on-disk content
/// overwritten so that its *new* quick_hash+file_size coincide with an
/// unrelated `offline` row's (`r1_id`; see `reconcile_known_path`'s
/// `NeedsRehash` arm). `reconcile_known_path` rehashes the known row in
/// place (never reactivating the offline one) and must also persist the
/// pair to `path_collisions` so `dedup::detect_duplicate_groups` can
/// surface it as a duplicate candidate, instead of silently leaving the
/// offline row with no trace of the coincidence.
#[test]
fn reconcile_known_path_records_a_path_collision_when_the_rehash_coincidentally_matches_an_offline_row(
) {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();

    // `r1_id`: an unrelated offline row, content "shared content".
    let old_dir = tempfile::tempdir().unwrap();
    let old_path = old_dir.path().join("old.mp4");
    fs::write(&old_path, b"shared content").unwrap();
    let old_folders = vec![old_dir.path().to_string_lossy().to_string()];
    scan_folders(&old_folders, &db, thumbs_root.path(), &AssumeReachable).unwrap();
    let r1_id: String = {
        let conn = db.writer.lock().unwrap();
        conn.execute("UPDATE videos SET status = 'offline'", [])
            .unwrap();
        conn.query_row("SELECT id FROM videos", [], |r| r.get(0))
            .unwrap()
    };

    // `r2_id`: a known, online row -- registered with different content,
    // so its initial quick_hash/file_size do *not* match `r1_id`'s.
    let new_dir = tempfile::tempdir().unwrap();
    let new_path = new_dir.path().join("new.mp4");
    fs::write(&new_path, b"original R2 content, a different length").unwrap();
    let new_folders = vec![new_dir.path().to_string_lossy().to_string()];
    scan_folders(&new_folders, &db, thumbs_root.path(), &AssumeReachable).unwrap();
    let r2_id: String = {
        let conn = db.writer.lock().unwrap();
        conn.query_row(
            "SELECT id FROM videos WHERE file_path = ?1",
            [new_path.to_string_lossy().to_string()],
            |r| r.get(0),
        )
        .unwrap()
    };

    // Overwrite the known row's file in place (same path) with bytes
    // identical to the offline row's -- the physical trigger for both
    // NeedsRehash (file_size differs from the stored metadata) and the
    // coincidental-match check (new quick_hash+file_size equal the
    // offline row's).
    fs::write(&new_path, b"shared content").unwrap();
    let metadata = fs::metadata(&new_path).unwrap();
    let scanned_file = ScannedFile {
        file_path: new_path.to_string_lossy().to_string(),
        file_name: "new.mp4".to_string(),
        file_size: metadata.len(),
        mtime: mtime_from_metadata(&metadata).unwrap(),
    };
    let known = {
        let conn = db.read_pool.get().unwrap();
        queries::find_video_by_path(&conn, &scanned_file.file_path)
            .unwrap()
            .unwrap()
    };

    let outcome = reconcile_known_path(&db, thumbs_root.path(), &scanned_file, &known).unwrap();
    assert!(
        matches!(outcome, ProcessOutcome::Reconciled),
        "R2 is always just reconciled in place, never reactivated/replaced"
    );

    let conn = db.writer.lock().unwrap();
    let (r2_status, r2_path, r2_quick_hash): (String, String, String) = conn
        .query_row(
            "SELECT status, file_path, quick_hash FROM videos WHERE id = ?1",
            [&r2_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(r2_status, "online", "R2 keeps its own online status");
    assert_eq!(r2_path, new_path.to_string_lossy().to_string());

    let r1_status: String = conn
        .query_row("SELECT status FROM videos WHERE id = ?1", [&r1_id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        r1_status, "offline",
        "reconcile_known_path never reactivates R1 -- that's out of scope for this arm"
    );

    let collisions = queries::list_path_collisions(&db.read_pool).unwrap();
    assert_eq!(
        collisions.len(),
        1,
        "the coincidental rehash match must be recorded exactly once"
    );
    assert_eq!(
        collisions[0].video_id, r1_id,
        "video_id is the offline candidate"
    );
    assert_eq!(
        collisions[0].colliding_video_id, r2_id,
        "colliding_video_id is the known/online row that got rehashed"
    );
    assert_eq!(collisions[0].attempted_path, r2_path);
    assert_eq!(
        r2_quick_hash,
        gb_core::hash::quick_hash(&mut File::open(&new_path).unwrap(), metadata.len())
            .unwrap()
            .to_string(),
        "R2's stored quick_hash must reflect its new content"
    );
}

/// Self-collision regression: `known` itself can be `status='offline'`
/// at the moment `reconcile_known_path` runs its `NeedsRehash` arm -- this
/// happens whenever a row is reconnecting from offline to online at its
/// *own* unchanged path (e.g. a backup restore or drive reconnect that
/// preserves file bytes but not `mtime`), since `update_video_status`'s
/// online flip only happens *after* the coincidental-match lookup, not
/// before. If the freshly-recomputed quick_hash+file_size still equal
/// `known`'s own stale stored values (content genuinely unchanged, only
/// `mtime` drifted), `find_offline_candidates_by_quick_hash_and_size`
/// matches `known` against itself. Unfiltered, that would record a
/// `path_collisions` row pairing `known.id` with itself -- a nonsensical
/// "duplicate of itself" that `dedup::detect_duplicate_groups` would
/// then surface to the user. This must never happen.
#[test]
fn reconcile_known_path_does_not_record_a_self_collision_when_the_rehash_matches_its_own_stale_row()
{
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("video.mp4");
    fs::write(&path, b"shared content").unwrap();
    let folders = vec![dir.path().to_string_lossy().to_string()];
    scan_folders(&folders, &db, thumbs_root.path(), &AssumeReachable).unwrap();

    let video_id: String = {
        let conn = db.writer.lock().unwrap();
        conn.query_row("SELECT id FROM videos", [], |r| r.get(0))
            .unwrap()
    };

    // Simulate the row having gone offline (e.g. drive disconnect) and
    // its stored mtime now disagreeing with the file's real mtime on
    // disk (e.g. the drive/backup preserved content but not
    // timestamps), *without* touching the file's actual bytes or the
    // stored quick_hash/file_size -- both still match what a fresh
    // rehash of the unchanged file will compute.
    {
        let conn = db.writer.lock().unwrap();
        conn.execute(
            "UPDATE videos SET status = 'offline', mtime = mtime - 1000",
            [],
        )
        .unwrap();
    }

    let metadata = fs::metadata(&path).unwrap();
    let scanned_file = ScannedFile {
        file_path: path.to_string_lossy().to_string(),
        file_name: "video.mp4".to_string(),
        file_size: metadata.len(),
        mtime: mtime_from_metadata(&metadata).unwrap(),
    };
    let known = {
        let conn = db.read_pool.get().unwrap();
        queries::find_video_by_path(&conn, &scanned_file.file_path)
            .unwrap()
            .unwrap()
    };
    assert_eq!(
        known.status, "offline",
        "precondition: known must still be offline when NeedsRehash is classified, since \
             classify_discovered_file's mtime mismatch is what routes here in the first place"
    );

    let outcome = reconcile_known_path(&db, thumbs_root.path(), &scanned_file, &known).unwrap();
    assert!(
        matches!(outcome, ProcessOutcome::Reconciled),
        "reconnecting at the same path with unchanged content is still an ordinary reconcile"
    );

    let conn = db.writer.lock().unwrap();
    let status: String = conn
        .query_row(
            "SELECT status FROM videos WHERE id = ?1",
            [&video_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "online", "the row must be flipped back online");

    let collisions = queries::list_path_collisions(&db.read_pool).unwrap();
    assert!(
        collisions.is_empty(),
        "a row must never be recorded as colliding with itself in path_collisions"
    );
}

/// Regression companion to the test above: an ordinary `NeedsRehash`
/// reconciliation whose new content does *not* coincide with any
/// `offline` row's quick_hash+file_size must not write anything to
/// `path_collisions`.
#[test]
fn reconcile_known_path_does_not_record_a_collision_when_no_offline_row_matches_the_rehash() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("video.mp4");
    fs::write(&path, b"original content").unwrap();
    let folders = vec![dir.path().to_string_lossy().to_string()];
    scan_folders(&folders, &db, thumbs_root.path(), &AssumeReachable).unwrap();

    // Change the content to something with no matching offline
    // candidate anywhere in the DB (there isn't even another row).
    fs::write(&path, b"changed content, nothing else shares this hash").unwrap();
    let metadata = fs::metadata(&path).unwrap();
    let scanned_file = ScannedFile {
        file_path: path.to_string_lossy().to_string(),
        file_name: "video.mp4".to_string(),
        file_size: metadata.len(),
        mtime: mtime_from_metadata(&metadata).unwrap(),
    };
    let known = {
        let conn = db.read_pool.get().unwrap();
        queries::find_video_by_path(&conn, &scanned_file.file_path)
            .unwrap()
            .unwrap()
    };

    let outcome = reconcile_known_path(&db, thumbs_root.path(), &scanned_file, &known).unwrap();
    assert!(matches!(outcome, ProcessOutcome::Reconciled));

    let collisions = queries::list_path_collisions(&db.read_pool).unwrap();
    assert!(
        collisions.is_empty(),
        "an ordinary rehash with no offline-candidate match must not write to \
             path_collisions"
    );
}

/// Regression test for `register_new_path`'s `Reactivate` branch: a
/// path-follow reactivation must not just rewrite `file_path` in the
/// DB, it must also move the video's already-generated cached
/// thumbnails from the subdirectory resolved for its *old* file_path to
/// the one resolved for its *new* file_path (`thumbnail::paths::
/// move_video_thumbnails`) -- otherwise a later `get_thumbnails` call
/// would look in the (now wrong) new location and find nothing, even
/// though the thumbnails were already generated and simply never moved.
#[test]
fn reactivating_a_video_via_path_follow_moves_its_cached_thumbnails_to_the_new_folders_subdir() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();

    let old_dir = tempfile::tempdir().unwrap();
    let new_dir = tempfile::tempdir().unwrap();
    let old_path = old_dir.path().join("old.mp4");
    fs::write(&old_path, b"shared content for path-follow").unwrap();

    let old_folder = old_dir.path().to_string_lossy().to_string();
    let new_folder = new_dir.path().to_string_lossy().to_string();
    let watch_folders = vec![old_folder.clone(), new_folder.clone()];
    {
        let conn = db.writer.lock().unwrap();
        queries::set_watch_folders(&conn, &watch_folders).unwrap();
    }

    // Register the video under old_dir, then simulate it going offline
    // (e.g. the drive holding old_dir was disconnected).
    scan_folders(
        std::slice::from_ref(&old_folder),
        &db,
        thumbs_root.path(),
        &AssumeReachable,
    )
    .unwrap();
    let video_id: String = {
        let conn = db.writer.lock().unwrap();
        conn.execute("UPDATE videos SET status = 'offline'", [])
            .unwrap();
        conn.query_row("SELECT id FROM videos", [], |r| r.get(0))
            .unwrap()
    };

    // Simulate 6 already-generated thumbnail files sitting in the
    // subdirectory resolved for the video's *old* file_path.
    let old_video_dir = thumbnail::paths::video_thumbnail_dir(
        thumbs_root.path(),
        &watch_folders,
        &old_path.to_string_lossy(),
    );
    std::fs::create_dir_all(&old_video_dir).unwrap();
    for i in 0..thumbnail::worker::THUMBNAILS_PER_VIDEO {
        std::fs::write(
            old_video_dir.join(format!("{video_id}_{i}.webp")),
            format!("slot-{i}"),
        )
        .unwrap();
    }

    // The file reappears at a new path (under a different registered
    // folder) with identical content -- a path-follow match.
    let new_path = new_dir.path().join("new.mp4");
    fs::write(&new_path, b"shared content for path-follow").unwrap();

    scan_folders(
        std::slice::from_ref(&new_folder),
        &db,
        thumbs_root.path(),
        &AssumeReachable,
    )
    .unwrap();

    let conn = db.writer.lock().unwrap();
    let (status, file_path): (String, String) = conn
        .query_row(
            "SELECT status, file_path FROM videos WHERE id = ?1",
            [&video_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "online", "the video must be reactivated online");
    assert_eq!(
        file_path,
        new_path.to_string_lossy().to_string(),
        "the video's id must be reused (reactivated), not re-registered"
    );
    drop(conn);

    let new_video_dir = thumbnail::paths::video_thumbnail_dir(
        thumbs_root.path(),
        &watch_folders,
        &new_path.to_string_lossy(),
    );
    for i in 0..thumbnail::worker::THUMBNAILS_PER_VIDEO {
        assert!(
            !old_video_dir.join(format!("{video_id}_{i}.webp")).exists(),
            "slot {i} must no longer exist in the old folder's thumbnail subdirectory"
        );
        assert_eq!(
            std::fs::read_to_string(new_video_dir.join(format!("{video_id}_{i}.webp"))).unwrap(),
            format!("slot-{i}"),
            "slot {i} must exist, with its original content, in the new folder's \
                 thumbnail subdirectory"
        );
    }
}

/// Registers `content` at a fresh path, marks the row as fully derived
/// (thumbnails ready, duration probed) and writes its 6 slot files plus
/// a neighbouring video's 6 slot files in the same directory. Returns
/// `(video_id, path, video_dir, thumbs_root)` once set up.
fn setup_video_with_derived_data(
    db: &Db,
    content: &[u8],
) -> (
    String,
    std::path::PathBuf,
    std::path::PathBuf,
    tempfile::TempDir,
    tempfile::TempDir,
) {
    let thumbs_root = temp_thumbs_root();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("video.mp4");
    fs::write(&path, content).unwrap();
    let folders = vec![dir.path().to_string_lossy().to_string()];
    {
        let conn = db.writer.lock().unwrap();
        queries::set_watch_folders(&conn, &folders).unwrap();
    }
    scan_folders(&folders, db, thumbs_root.path(), &AssumeReachable).unwrap();
    let video_id: String = {
        let conn = db.writer.lock().unwrap();
        conn.execute(
            "UPDATE videos SET thumbnail_ready = 1, thumbnail_attempts = 2,
                 duration = 5, probed_at = CURRENT_TIMESTAMP, metadata_attempts = 1",
            [],
        )
        .unwrap();
        conn.query_row("SELECT id FROM videos", [], |r| r.get(0))
            .unwrap()
    };
    let video_dir = thumbnail::paths::video_thumbnail_dir(
        thumbs_root.path(),
        &folders,
        &path.to_string_lossy(),
    );
    fs::create_dir_all(&video_dir).unwrap();
    for i in 0..thumbnail::worker::THUMBNAILS_PER_VIDEO {
        fs::write(video_dir.join(format!("{video_id}_{i}.webp")), "mine").unwrap();
        fs::write(video_dir.join(format!("other-video_{i}.webp")), "other").unwrap();
    }
    (video_id, path, video_dir, thumbs_root, dir)
}

fn derived_state(db: &Db, id: &str) -> (i64, i64, Option<i64>, Option<String>, i64) {
    let conn = db.writer.lock().unwrap();
    conn.query_row(
        "SELECT thumbnail_ready, thumbnail_attempts, duration, probed_at, metadata_attempts
             FROM videos WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )
    .unwrap()
}

fn rescan_with_current_file(db: &Db, thumbs_root: &Path, path: &Path, mtime_bump: i64) {
    let metadata = fs::metadata(path).unwrap();
    process_detected_file(
        db,
        thumbs_root,
        path,
        metadata.len(),
        mtime_from_metadata(&metadata).unwrap() + mtime_bump,
        &AssumeReachable,
    )
    .unwrap();
}

#[test]
fn a_size_change_invalidates_derived_data_and_removes_only_that_videos_thumbnails() {
    let (_db_dir, db) = init_temp_db();
    let (id, path, video_dir, thumbs_root, _dir) =
        setup_video_with_derived_data(&db, b"truncated head");

    fs::write(&path, b"the complete file, now much longer than before").unwrap();
    rescan_with_current_file(&db, thumbs_root.path(), &path, 0);

    assert_eq!(derived_state(&db, &id), (0, 0, None, None, 0));
    for i in 0..thumbnail::worker::THUMBNAILS_PER_VIDEO {
        assert!(!video_dir.join(format!("{id}_{i}.webp")).exists());
        assert!(
            video_dir.join(format!("other-video_{i}.webp")).exists(),
            "another video's thumbnails must be left alone"
        );
    }
}

#[test]
fn a_quick_hash_change_at_the_same_size_invalidates_derived_data() {
    let (_db_dir, db) = init_temp_db();
    let (id, path, video_dir, thumbs_root, _dir) =
        setup_video_with_derived_data(&db, b"aaaaaaaaaaaaaaaa");

    fs::write(&path, b"bbbbbbbbbbbbbbbb").unwrap();
    rescan_with_current_file(&db, thumbs_root.path(), &path, 100);

    assert_eq!(derived_state(&db, &id), (0, 0, None, None, 0));
    assert!(!video_dir.join(format!("{id}_0.webp")).exists());
}

#[test]
fn an_mtime_only_change_keeps_derived_data_and_thumbnails() {
    let (_db_dir, db) = init_temp_db();
    let (id, path, video_dir, thumbs_root, _dir) =
        setup_video_with_derived_data(&db, b"unchanged content");

    rescan_with_current_file(&db, thumbs_root.path(), &path, 100);

    let (ready, attempts, duration, probed_at, meta_attempts) = derived_state(&db, &id);
    assert_eq!(
        (ready, attempts, duration, meta_attempts),
        (1, 2, Some(5), 1)
    );
    assert!(probed_at.is_some());
    for i in 0..thumbnail::worker::THUMBNAILS_PER_VIDEO {
        assert!(video_dir.join(format!("{id}_{i}.webp")).exists());
    }
}

/// Registers `content` under `old_dir`, flips it offline, and returns
/// `(video_id, old_path)` -- the starting point for path-follow tests.
fn register_then_take_offline(
    db: &Db,
    thumbs_root: &Path,
    old_dir: &Path,
    content: &[u8],
) -> (String, std::path::PathBuf) {
    let old_path = old_dir.join("old.mp4");
    fs::write(&old_path, content).unwrap();
    let folders = vec![old_dir.to_string_lossy().to_string()];
    scan_folders(&folders, db, thumbs_root, &AssumeReachable).unwrap();
    let conn = db.writer.lock().unwrap();
    conn.execute("UPDATE videos SET status = 'offline'", [])
        .unwrap();
    let id = conn
        .query_row("SELECT id FROM videos", [], |r| r.get(0))
        .unwrap();
    (id, old_path)
}

fn scanned_file_of(path: &Path) -> ScannedFile {
    let metadata = fs::metadata(path).unwrap();
    ScannedFile {
        file_path: path.to_string_lossy().to_string(),
        file_name: path.file_name().unwrap().to_string_lossy().to_string(),
        file_size: metadata.len(),
        mtime: mtime_from_metadata(&metadata).unwrap(),
    }
}

fn known_row_of(db: &Db, scanned_file: &ScannedFile) -> queries::VideoRow {
    let conn = db.read_pool.get().unwrap();
    queries::find_video_by_path(&conn, &scanned_file.file_path)
        .unwrap()
        .unwrap()
}

#[test]
fn a_thumbnail_move_failure_does_not_undo_the_path_follow_reactivation() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let old_dir = tempfile::tempdir().unwrap();
    let new_dir = tempfile::tempdir().unwrap();
    let watch_folders = vec![
        old_dir.path().to_string_lossy().to_string(),
        new_dir.path().to_string_lossy().to_string(),
    ];
    {
        let conn = db.writer.lock().unwrap();
        queries::set_watch_folders(&conn, &watch_folders).unwrap();
    }
    let (video_id, old_path) =
        register_then_take_offline(&db, thumbs_root.path(), old_dir.path(), b"moved content");

    let old_video_dir = thumbnail::paths::video_thumbnail_dir(
        thumbs_root.path(),
        &watch_folders,
        &old_path.to_string_lossy(),
    );
    fs::create_dir_all(&old_video_dir).unwrap();
    fs::write(old_video_dir.join(format!("{video_id}_0.webp")), "slot-0").unwrap();

    let new_path = new_dir.path().join("new.mp4");
    fs::write(&new_path, b"moved content").unwrap();
    let new_video_dir = thumbnail::paths::video_thumbnail_dir(
        thumbs_root.path(),
        &watch_folders,
        &new_path.to_string_lossy(),
    );
    // A directory squatting on the destination slot makes the rename fail.
    fs::create_dir_all(new_video_dir.join(format!("{video_id}_0.webp"))).unwrap();

    let outcome = register_new_path(
        &db,
        thumbs_root.path(),
        &scanned_file_of(&new_path),
        &AssumeReachable,
    )
    .unwrap();
    assert!(matches!(
    outcome,
    ProcessOutcome::PathFollowed { video_id: ref id } if *id == video_id
            ));

    let conn = db.writer.lock().unwrap();
    let (status, file_path): (String, String) = conn
        .query_row(
            "SELECT status, file_path FROM videos WHERE id = ?1",
            [&video_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "online");
    assert_eq!(file_path, new_path.to_string_lossy().to_string());
    assert!(
        old_video_dir.join(format!("{video_id}_0.webp")).exists(),
        "the thumbnail that failed to move stays where it was"
    );
}

#[test]
fn a_failed_path_follow_write_leaves_the_offline_row_untouched_and_is_skipped() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let old_dir = tempfile::tempdir().unwrap();
    let new_dir = tempfile::tempdir().unwrap();
    let (video_id, old_path) =
        register_then_take_offline(&db, thumbs_root.path(), old_dir.path(), b"moved content");
    let new_path = new_dir.path().join("new.mp4");
    fs::write(&new_path, b"moved content").unwrap();
    {
        let conn = db.writer.lock().unwrap();
        conn.execute_batch(
            "CREATE TRIGGER block_path_update BEFORE UPDATE OF file_path ON videos
                 BEGIN SELECT RAISE(ABORT, 'blocked'); END;",
        )
        .unwrap();
    }

    let outcome = register_new_path(
        &db,
        thumbs_root.path(),
        &scanned_file_of(&new_path),
        &AssumeReachable,
    )
    .unwrap();
    assert!(matches!(outcome, ProcessOutcome::SkippedUnreadable));

    let conn = db.writer.lock().unwrap();
    let (status, file_path): (String, String) = conn
        .query_row(
            "SELECT status, file_path FROM videos WHERE id = ?1",
            [&video_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "offline");
    assert_eq!(file_path, old_path.to_string_lossy().to_string());
}

#[test]
fn an_unreadable_new_file_is_skipped_without_touching_the_db() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let dir = tempfile::tempdir().unwrap();
    let scanned_file = ScannedFile {
        file_path: dir
            .path()
            .join("vanished.mp4")
            .to_string_lossy()
            .to_string(),
        file_name: "vanished.mp4".to_string(),
        file_size: 10,
        mtime: 1,
    };
    let outcome =
        register_new_path(&db, thumbs_root.path(), &scanned_file, &AssumeReachable).unwrap();
    assert!(matches!(outcome, ProcessOutcome::SkippedUnreadable));
    let conn = db.writer.lock().unwrap();
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM videos", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn a_new_path_with_no_offline_match_is_registered_as_a_new_row() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fresh.mp4");
    fs::write(&path, b"fresh content").unwrap();
    let outcome = register_new_path(
        &db,
        thumbs_root.path(),
        &scanned_file_of(&path),
        &AssumeReachable,
    )
    .unwrap();
    assert!(matches!(outcome, ProcessOutcome::Registered));
    let conn = db.writer.lock().unwrap();
    let (status, count): (String, i64) = conn
        .query_row("SELECT status, COUNT(*) FROM videos", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((status.as_str(), count), ("online", 1));
}

#[test]
fn reconcile_known_path_reports_unchanged_for_an_online_row_with_matching_metadata() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("video.mp4");
    fs::write(&path, b"stable content").unwrap();
    scan_folders(
        &[dir.path().to_string_lossy().to_string()],
        &db,
        thumbs_root.path(),
        &AssumeReachable,
    )
    .unwrap();
    let scanned_file = scanned_file_of(&path);
    let known = known_row_of(&db, &scanned_file);
    let outcome = reconcile_known_path(&db, thumbs_root.path(), &scanned_file, &known).unwrap();
    assert!(matches!(outcome, ProcessOutcome::Unchanged));
}

#[test]
fn reconcile_known_path_brings_an_unchanged_offline_row_back_online() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let dir = tempfile::tempdir().unwrap();
    let (video_id, path) =
        register_then_take_offline(&db, thumbs_root.path(), dir.path(), b"stable content");
    let scanned_file = scanned_file_of(&path);
    let known = known_row_of(&db, &scanned_file);
    let outcome = reconcile_known_path(&db, thumbs_root.path(), &scanned_file, &known).unwrap();
    assert!(matches!(outcome, ProcessOutcome::Reconciled));
    let conn = db.writer.lock().unwrap();
    let status: String = conn
        .query_row(
            "SELECT status FROM videos WHERE id = ?1",
            [&video_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, "online");
}

#[test]
fn reconcile_known_path_leaves_the_row_as_is_when_the_rehash_cannot_read_the_file() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let dir = tempfile::tempdir().unwrap();
    let (video_id, path) =
        register_then_take_offline(&db, thumbs_root.path(), dir.path(), b"original");
    let mut scanned_file = scanned_file_of(&path);
    scanned_file.mtime += 100;
    let known = known_row_of(&db, &scanned_file);
    fs::remove_file(&path).unwrap();
    let outcome = reconcile_known_path(&db, thumbs_root.path(), &scanned_file, &known).unwrap();
    assert!(matches!(outcome, ProcessOutcome::SkippedUnreadable));
    let conn = db.writer.lock().unwrap();
    let (status, mtime): (String, i64) = conn
        .query_row(
            "SELECT status, mtime FROM videos WHERE id = ?1",
            [&video_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "offline");
    assert_eq!(Some(mtime), known.mtime);
}

#[test]
fn reconcile_known_path_updates_metadata_and_goes_online_when_an_offline_row_changed_content() {
    let (_db_dir, db) = init_temp_db();
    let thumbs_root = temp_thumbs_root();
    let dir = tempfile::tempdir().unwrap();
    let (video_id, path) =
        register_then_take_offline(&db, thumbs_root.path(), dir.path(), b"original");
    fs::write(&path, b"original, now longer").unwrap();
    let scanned_file = scanned_file_of(&path);
    let known = known_row_of(&db, &scanned_file);
    let outcome = reconcile_known_path(&db, thumbs_root.path(), &scanned_file, &known).unwrap();
    assert!(matches!(outcome, ProcessOutcome::Reconciled));
    let conn = db.writer.lock().unwrap();
    let (status, size, mtime): (String, i64, i64) = conn
        .query_row(
            "SELECT status, file_size, mtime FROM videos WHERE id = ?1",
            [&video_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(status, "online");
    assert_eq!(size, scanned_file.file_size as i64);
    assert_eq!(mtime, scanned_file.mtime);
}

use super::move_fixture::MoveFixture;

fn process(fx: &MoveFixture, path: &Path, reach: &dyn FolderReachability) -> ProcessOutcome {
    let metadata = fs::metadata(path).unwrap();
    process_detected_file(
        &fx.db,
        fx.thumbs.path(),
        path,
        metadata.len(),
        mtime_from_metadata(&metadata).unwrap(),
        reach,
    )
    .unwrap()
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

/// A video is registered in folder A, A then disappears (renamed or
/// unplugged) while the registry still caches it as reachable, and the file
/// shows up in B. No hook has run yet.
#[test]
fn a_move_out_of_a_vanished_folder_is_followed_before_any_hook_ran() {
    let fx = MoveFixture::new(2);
    let old = fx.register_at(0, "old.mp4");
    let id = fx.id_at(&old);
    fx.tag(&id, "keep");
    fx.put_thumbnail(&id, &old);
    fx.probe.set_reachable(fx.folder(0), false);
    let new = fx.dirs[1].path().join("new.mp4");
    fs::rename(&old, &new).unwrap();

    let outcome = process(&fx, &new, &fx.registry);

    assert!(matches!(outcome, ProcessOutcome::PathFollowed { video_id } if video_id == id));
    assert_eq!(fx.rows(), vec![(path_str(&new), "online".to_string())]);
    assert_eq!(fx.id_at(&new), id);
    assert_eq!(fx.tag_count(&id), 1);
    assert!(fx.thumbnail_slot(&id, &new).exists());
    assert!(!fx.thumbnail_slot(&id, &old).exists());
}

#[test]
fn a_move_is_followed_when_the_unreachable_hook_already_flipped_the_rows() {
    let fx = MoveFixture::new(2);
    let old = fx.register_at(0, "old.mp4");
    let id = fx.id_at(&old);
    {
        let db = fx.db.clone();
        fx.registry.set_on_unreachable(move |folder| {
            crate::offline_sync::mark_folder_offline(&db, folder).unwrap();
        });
    }
    fx.probe.set_reachable(fx.folder(0), false);
    assert!(!fx.registry.is_reachable(fx.folder(0)));
    let new = fx.dirs[1].path().join("new.mp4");
    fs::rename(&old, &new).unwrap();

    let outcome = process(&fx, &new, &fx.registry);

    assert!(matches!(outcome, ProcessOutcome::PathFollowed { video_id } if video_id == id));
    assert_eq!(fx.rows(), vec![(path_str(&new), "online".to_string())]);
    assert_eq!(
        fx.probe.call_count_for(fx.folder(0)),
        1,
        "an offline candidate needs no further check"
    );
}

#[test]
fn a_copy_of_a_file_in_a_reachable_folder_is_a_new_row() {
    let fx = MoveFixture::new(2);
    let old = fx.register_at(0, "old.mp4");
    let copy = fx.write_at(1, "copy.mp4");

    let outcome = process(&fx, &copy, &fx.registry);

    assert!(matches!(outcome, ProcessOutcome::Registered));
    let mut expected = vec![
        (path_str(&copy), "online".to_string()),
        (path_str(&old), "online".to_string()),
    ];
    expected.sort();
    assert_eq!(fx.rows(), expected);
}

#[test]
fn only_the_row_in_the_unreachable_folder_is_followed() {
    let fx = MoveFixture::new(3);
    let in_a = fx.register_at(0, "a.mp4");
    let in_c = fx.register_at(2, "c.mp4");
    let id_a = fx.id_at(&in_a);
    fx.probe.set_reachable(fx.folder(0), false);
    let new = fx.write_at(1, "new.mp4");

    let outcome = process(&fx, &new, &fx.registry);

    assert!(matches!(outcome, ProcessOutcome::PathFollowed { video_id } if video_id == id_a));
    let mut expected = vec![
        (path_str(&in_c), "online".to_string()),
        (path_str(&new), "online".to_string()),
    ];
    expected.sort();
    assert_eq!(fx.rows(), expected);
}

#[test]
fn rows_in_reachable_folders_are_not_touched_by_the_check() {
    let fx = MoveFixture::new(3);
    let in_a = fx.register_at(0, "a.mp4");
    let in_c = fx.register_at(2, "c.mp4");
    let new = fx.write_at(1, "new.mp4");

    let outcome = process(&fx, &new, &fx.registry);

    assert!(matches!(outcome, ProcessOutcome::Registered));
    let rows = fx.rows();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|(_, status)| status == "online"));
    assert!(rows.iter().any(|(p, _)| *p == path_str(&in_a)));
    assert!(rows.iter().any(|(p, _)| *p == path_str(&in_c)));
}

#[test]
fn a_burst_of_copies_probes_each_reachable_folder_only_once() {
    let fx = MoveFixture::new(2);
    fx.register_at(0, "old.mp4");
    for i in 0..6 {
        let copy = fx.write_at(1, &format!("copy{i}.mp4"));
        assert!(matches!(
            process(&fx, &copy, &fx.registry),
            ProcessOutcome::Registered
        ));
    }

    assert_eq!(fx.probe.call_count_for(fx.folder(0)), 1);
    assert_eq!(fx.probe.call_count_for(fx.folder(1)), 1);
    assert_eq!(fx.rows().len(), 7);
}

#[test]
fn a_lookalike_outside_every_watch_folder_is_left_alone() {
    let fx = MoveFixture::new(2);
    let new = fx.write_at(1, "new.mp4");
    let stray = tempfile::tempdir().unwrap();
    fs::write(stray.path().join("stray.mp4"), move_fixture::CONTENT).unwrap();
    scan_folders(
        &[path_str(stray.path())],
        &fx.db,
        fx.thumbs.path(),
        &AssumeReachable,
    )
    .unwrap();

    let outcome = process(&fx, &new, &fx.registry);

    assert!(matches!(outcome, ProcessOutcome::Registered));
    assert_eq!(fx.rows().len(), 2);
    assert!(
        fx.probe.calls().is_empty(),
        "no owning folder, nothing to probe"
    );
}

#[test]
fn a_manual_scan_follows_a_move_out_of_a_vanished_folder() {
    let fx = MoveFixture::new(2);
    let old = fx.register_at(0, "old.mp4");
    let id = fx.id_at(&old);
    fx.probe.set_reachable(fx.folder(0), false);
    let new = fx.dirs[1].path().join("new.mp4");
    fs::rename(&old, &new).unwrap();

    let summary = scan_folders(
        &[fx.folder(1).to_string()],
        &fx.db,
        fx.thumbs.path(),
        &fx.registry,
    )
    .unwrap();

    assert_eq!((summary.reactivated, summary.registered), (1, 0));
    assert_eq!(summary.collisions, 0);
    assert_eq!(fx.id_at(&new), id);
    assert_eq!(fx.rows().len(), 1);
}

/// The only video of a folder moved somewhere no watch folder covers:
/// the walk finds nothing (the `NothingDiscovered` guard), but the
/// one-by-one check gets NotFound from the real filesystem, so the old
/// row still goes offline.
#[test]
fn a_manual_scan_offlines_the_only_video_moved_out_of_its_folder() {
    let fx = MoveFixture::new(1);
    let old = fx.register_at(0, "only.mp4");
    let elsewhere = tempfile::tempdir().unwrap();
    fs::rename(&old, elsewhere.path().join("only.mp4")).unwrap();

    let summary = scan_folders(
        &[fx.folder(0).to_string()],
        &fx.db,
        fx.thumbs.path(),
        &fx.registry,
    )
    .unwrap();

    assert_eq!(summary.went_offline, 1);
    assert_eq!(fx.rows(), vec![(path_str(&old), "offline".to_string())]);
}

/// The per-path check answers NotFound for every file of a deleted
/// folder, so a folder that is gone must be kept away from it. With the
/// real filesystem, a watch folder deleted outright leaves all of its
/// videos online, for every count up to the check's limit.
#[test]
fn a_manual_scan_keeps_the_videos_of_a_deleted_watch_folder_online() {
    for count in [
        1,
        3,
        gb_core::reconcile::MAX_CANDIDATES_FOR_INDIVIDUAL_CHECK,
    ] {
        let fx = MoveFixture::new(1);
        for i in 1..count {
            fx.write_at(0, &format!("v{i}.mp4"));
        }
        // Registers every file written so far in one scan.
        fx.register_at(0, "v0.mp4");
        assert_eq!(fx.rows().len(), count);
        fs::remove_dir_all(fx.dirs[0].path()).unwrap();

        let summary = scan_folders(
            &[fx.folder(0).to_string()],
            &fx.db,
            fx.thumbs.path(),
            &fx.registry,
        )
        .unwrap();

        assert_eq!(summary.went_offline, 0, "count={count}");
        assert!(
            fx.rows().iter().all(|(_, status)| status == "online"),
            "count={count}"
        );
    }
}

/// Stand-in for a listing that missed a file which is in fact still
/// there: the walk finds nothing, and the OS says the path exists, so
/// the guard's hold stands.
#[test]
fn a_manual_scan_keeps_the_only_video_online_when_the_os_says_it_exists() {
    use gb_core::testing::fake_path_existence::FakePathExistence;

    let fx = MoveFixture::new(1);
    let old = fx.register_at(0, "only.mp4");
    let elsewhere = tempfile::tempdir().unwrap();
    fs::rename(&old, elsewhere.path().join("only.mp4")).unwrap();
    let existence = FakePathExistence::with_paths(&[&path_str(&old)]);

    let summary = scan_folders_with_existence(
        &[fx.folder(0).to_string()],
        &fx.db,
        fx.thumbs.path(),
        &fx.registry,
        &existence,
    )
    .unwrap();

    assert_eq!(summary.went_offline, 0);
    assert_eq!(fx.rows(), vec![(path_str(&old), "online".to_string())]);
}
