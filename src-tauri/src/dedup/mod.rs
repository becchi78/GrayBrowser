//! Duplicate-detection orchestration: wires `gb_core::dedup`'s pure
//! two-stage grouping to real file I/O (full_hash computation) and the DB
//! (`queries::list_online_video_hash_info`/`update_full_hash`/
//! `list_path_collisions`), then packages the result into `DuplicateGroup`s
//! the frontend can render directly.
//!
//! `refresh_duplicate_groups` mirrors `thumbnail::enqueue_missing_thumbnails`'s
//! fire-and-forget background-thread shape, but does not need a worker pool:
//! by the time any full_hash I/O happens, candidate groups are already a
//! small subset of the library (the cheap quick_hash+file_size
//! filter narrows things down first), so one sequential background thread is
//! enough -- adding worker-pool parallelism here would be premature
//! optimization for a workload this small.
//!
//! **This module never deletes a video file.** `full_hash_file` only reads
//! (`File::open` + `gb_core::hash::full_hash`); nothing in this file calls
//! `std::fs::remove_file`/`remove_dir*` against a video's `file_path` at all.
//! Removing a confirmed-duplicate *catalog entry* (the `videos` row, its
//! `video_tags`/`path_collisions` rows, and its cached thumbnail -- never the
//! source video file) is `commands::dedup_cmds::delete_duplicate_video`'s job,
//! not this module's.

use std::fs::File;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use crate::adapters::long_path;
use crate::db::{queries, Db};
use crate::events::DedupNotifier;

/// `scan::hash_file`'s full_hash counterpart: same long-path + `File::open`
/// shape, but `gb_core::hash::full_hash` only needs `Read` (not `Read +
/// Seek`), so there's no `file_size` argument to thread through.
pub fn full_hash_file(file_path: &str) -> anyhow::Result<String> {
    let mut file = File::open(long_path::to_long_path(Path::new(file_path)))?;
    Ok(gb_core::hash::full_hash(&mut file)?)
}

#[derive(Clone, serde::Serialize)]
pub struct DuplicateGroupMember {
    pub video_id: String,
    pub file_path: String,
    pub file_name: String,
    pub file_size: i64,
    pub status: String,
    pub created_at: String,
}

impl From<queries::VideoRow> for DuplicateGroupMember {
    fn from(row: queries::VideoRow) -> Self {
        Self {
            video_id: row.id,
            file_path: row.file_path,
            file_name: row.file_name,
            file_size: row.file_size,
            status: row.status,
            created_at: row.created_at,
        }
    }
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateGroupKind {
    /// Two or more `status='online'` rows sharing `quick_hash`+`file_size`
    /// (stage 1) whose `full_hash` also matches (stage 2) --
    /// `gb_core::dedup::confirm_duplicates_by_full_hash`.
    QuickHashConfirmed,
    /// A `path_collisions` pair -- recorded via either 経路X
    /// (`register_new_path`'s `BlockedByCollision`) or 経路Y
    /// (`reconcile_known_path`'s coincidental rehash match) -- whose
    /// `full_hash` also matches.
    PathCollisionConfirmed,
    /// A `path_collisions` pair (either route, see `PathCollisionConfirmed`)
    /// whose `full_hash` is unknown (I/O failure, vanished file) or does not
    /// match. Surfaced identically to
    /// the confirmed case -- a path collision alone is already treated as a
    /// duplicate candidate, independent of whether content verification
    /// succeeds.
    PathCollisionUnconfirmed,
}

#[derive(Clone, serde::Serialize)]
pub struct DuplicateGroup {
    pub kind: DuplicateGroupKind,
    pub members: Vec<DuplicateGroupMember>,
}

/// Ensures `video_id`'s `full_hash` is known, computing and writing it back
/// (`queries::update_full_hash`) if it hasn't been already. Returns `None`
/// -- logging a WARN rather than propagating an `Err` -- whenever anything
/// along the way can't produce an answer: the row no longer exists (deleted
/// since the caller's id list was built), the file can't be opened (moved/
/// locked/permission/vanished), or the write-back itself fails. This mirrors
/// this codebase's established "skip and continue" policy for per-file I/O
/// trouble during background batch work (see
/// `scan::register_new_path`'s `hash_file` failure handling) -- one member's
/// I/O trouble must never abort the whole duplicate-detection pass.
fn ensure_full_hash(db: &Db, video_id: &str) -> Option<String> {
    let row = match queries::find_video_by_id(&db.read_pool, video_id) {
        Ok(Some(row)) => row,
        Ok(None) => {
            log::warn!("dedup: video {video_id} no longer exists; skipping full_hash");
            return None;
        }
        Err(e) => {
            log::warn!("dedup: failed to look up video {video_id}: {e}");
            return None;
        }
    };
    if let Some(existing) = row.full_hash {
        return Some(existing);
    }
    match full_hash_file(&row.file_path) {
        Ok(hash) => {
            let conn = db.writer.lock().unwrap();
            if let Err(e) = queries::update_full_hash(&conn, video_id, &hash) {
                log::warn!("dedup: failed to write back full_hash for {video_id}: {e}");
            }
            Some(hash)
        }
        Err(e) => {
            log::warn!(
                "dedup: failed to compute full_hash for {video_id} ({}): {e}",
                row.file_path
            );
            None
        }
    }
}

/// Turns bare video ids into full `DuplicateGroupMember`s for the frontend.
/// An id whose row has vanished since the id list was built (deleted
/// concurrently, e.g. via `delete_duplicate_video`) is silently dropped
/// rather than failing the whole group -- same reasoning as
/// `ensure_full_hash`.
fn hydrate_members(db: &Db, ids: &[String]) -> Vec<DuplicateGroupMember> {
    ids.iter()
        .filter_map(|id| match queries::find_video_by_id(&db.read_pool, id) {
            Ok(Some(row)) => Some(DuplicateGroupMember::from(row)),
            Ok(None) => None,
            Err(e) => {
                log::warn!(
                    "dedup: failed to look up video {id} while hydrating a duplicate group: {e}"
                );
                None
            }
        })
        .collect()
}

/// Runs the full two-stage detection plus the
/// `path_collisions` pass (populated by both 経路X and 経路Y, see
/// `queries::record_path_collision`'s doc comment), synchronously. Callers
/// that don't want to block should use `refresh_duplicate_groups` instead.
pub fn detect_duplicate_groups(db: &Db) -> anyhow::Result<Vec<DuplicateGroup>> {
    let mut groups = Vec::new();

    // Stage 1 (cheap quick_hash+file_size grouping) + stage 2 (full_hash
    // confirmation, computed/backfilled lazily only for candidate-group
    // members).
    let hash_infos = queries::list_online_video_hash_info(&db.read_pool)?;
    for candidate_group in gb_core::dedup::group_candidates_by_quick_hash(&hash_infos) {
        let mut owned_group: Vec<gb_core::dedup::VideoHashInfo> =
            Vec::with_capacity(candidate_group.len());
        for video in candidate_group {
            let full_hash = ensure_full_hash(db, &video.id);
            owned_group.push(gb_core::dedup::VideoHashInfo {
                id: video.id.clone(),
                quick_hash: video.quick_hash.clone(),
                file_size: video.file_size,
                full_hash,
            });
        }
        for confirmed_ids in gb_core::dedup::confirm_duplicates_by_full_hash(&owned_group) {
            groups.push(DuplicateGroup {
                kind: DuplicateGroupKind::QuickHashConfirmed,
                members: hydrate_members(db, &confirmed_ids),
            });
        }
    }

    // path_collisions pairs (経路X and 経路Y both land here indistinguishably
    // -- see queries::record_path_collision's doc comment): always surfaced
    // as a group, regardless of whether content verification confirms or
    // refutes the match -- a collision pair is already treated as a
    // duplicate candidate on its own.
    for collision in queries::list_path_collisions(&db.read_pool)? {
        let ids = [collision.video_id, collision.colliding_video_id];
        let hash_a = ensure_full_hash(db, &ids[0]);
        let hash_b = ensure_full_hash(db, &ids[1]);
        let kind = if matches!((&hash_a, &hash_b), (Some(a), Some(b)) if a == b) {
            DuplicateGroupKind::PathCollisionConfirmed
        } else {
            DuplicateGroupKind::PathCollisionUnconfirmed
        };
        groups.push(DuplicateGroup {
            kind,
            members: hydrate_members(db, &ids),
        });
    }

    Ok(groups)
}

/// Shared state holding the most recently completed `detect_duplicate_groups`
/// result, so `commands::dedup_cmds::list_duplicate_groups` can answer
/// instantly without re-running detection on every call -- same
/// `app.manage()`-shared-state shape as `thumbnail::ThumbnailQueueHandle`.
#[derive(Clone)]
pub struct DuplicateGroupsState(Arc<Mutex<Vec<DuplicateGroup>>>);

impl Default for DuplicateGroupsState {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(Vec::new())))
    }
}

impl DuplicateGroupsState {
    pub fn get(&self) -> Vec<DuplicateGroup> {
        self.0.lock().unwrap().clone()
    }

    fn set(&self, groups: Vec<DuplicateGroup>) {
        *self.0.lock().unwrap() = groups;
    }
}

/// Fire-and-forget: runs `detect_duplicate_groups` on a background thread
/// (never blocks the caller -- `.setup()` at startup, or after `start_scan`
/// completes, mirroring `thumbnail::enqueue_missing_thumbnails`), writes the
/// result into `state`, and notifies the frontend. Errors are logged only,
/// not propagated -- consistent with every other fire-and-forget background
/// pass in this codebase.
pub fn refresh_duplicate_groups<N>(db: Db, state: DuplicateGroupsState, notifier: Arc<N>)
where
    N: DedupNotifier + 'static,
{
    thread::spawn(move || match detect_duplicate_groups(&db) {
        Ok(groups) => {
            state.set(groups.clone());
            notifier.notify_updated(&groups);
        }
        Err(e) => {
            log::error!("failed to detect duplicate groups: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::init_temp_db;
    use std::fs;

    fn insert_online_video(db: &Db, id: &str, file_path: &Path, quick_hash: &str, file_size: i64) {
        let conn = db.writer.lock().unwrap();
        conn.execute(
            "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status)
             VALUES (?1, ?2, 'v.mp4', ?3, ?4, 'online')",
            rusqlite::params![
                id,
                file_path.to_string_lossy().to_string(),
                file_size,
                quick_hash
            ],
        )
        .unwrap();
    }

    fn insert_offline_video(db: &Db, id: &str, file_path: &str, quick_hash: &str) {
        let conn = db.writer.lock().unwrap();
        conn.execute(
            "INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status)
             VALUES (?1, ?2, 'v.mp4', 1, ?3, 'offline')",
            rusqlite::params![id, file_path, quick_hash],
        )
        .unwrap();
    }

    #[test]
    fn full_hash_file_matches_gb_core_full_hash_for_the_same_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        let content = b"hello world, dedup round-trip test content";
        fs::write(&path, content).unwrap();

        let via_file = full_hash_file(&path.to_string_lossy()).unwrap();

        let mut cursor = std::io::Cursor::new(content.to_vec());
        let expected = gb_core::hash::full_hash(&mut cursor).unwrap();
        assert_eq!(via_file, expected);
    }

    #[test]
    fn full_hash_file_returns_err_for_a_nonexistent_path() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist.mp4");
        assert!(full_hash_file(&missing.to_string_lossy()).is_err());
    }

    #[test]
    fn detect_duplicate_groups_confirms_two_online_videos_sharing_content() {
        let (_db_dir, db) = init_temp_db();
        let files_dir = tempfile::tempdir().unwrap();
        let content = b"identical bytes shared by two files";

        let path_a = files_dir.path().join("a.mp4");
        let path_b = files_dir.path().join("b.mp4");
        fs::write(&path_a, content).unwrap();
        fs::write(&path_b, content).unwrap();

        insert_online_video(&db, "video-a", &path_a, "qh-shared", content.len() as i64);
        insert_online_video(&db, "video-b", &path_b, "qh-shared", content.len() as i64);

        let groups = detect_duplicate_groups(&db).unwrap();
        assert_eq!(groups.len(), 1);
        assert!(matches!(
            groups[0].kind,
            DuplicateGroupKind::QuickHashConfirmed
        ));
        let mut ids: Vec<&str> = groups[0]
            .members
            .iter()
            .map(|m| m.video_id.as_str())
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["video-a", "video-b"]);

        // full_hash should have been backfilled for both members.
        let conn = db.writer.lock().unwrap();
        for id in ["video-a", "video-b"] {
            let full_hash: Option<String> = conn
                .query_row("SELECT full_hash FROM videos WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .unwrap();
            assert!(
                full_hash.is_some(),
                "full_hash should be backfilled for {id}"
            );
        }
    }

    #[test]
    fn detect_duplicate_groups_never_groups_offline_or_empty_quick_hash_rows() {
        let (_db_dir, db) = init_temp_db();
        let files_dir = tempfile::tempdir().unwrap();
        let content = b"shared content";

        let path_a = files_dir.path().join("a.mp4");
        let path_b = files_dir.path().join("b.mp4");
        let path_offline = files_dir.path().join("offline.mp4");
        let path_empty = files_dir.path().join("empty_hash.mp4");
        for p in [&path_a, &path_b, &path_offline, &path_empty] {
            fs::write(p, content).unwrap();
        }

        insert_online_video(&db, "video-a", &path_a, "qh-shared", content.len() as i64);
        insert_online_video(&db, "video-b", &path_b, "qh-shared", content.len() as i64);
        // Same quick_hash+file_size as video-a/-b, but offline -- must never
        // join the group.
        insert_offline_video(
            &db,
            "video-offline",
            &path_offline.to_string_lossy(),
            "qh-shared",
        );
        // Online, but with the empty-quick_hash placeholder -- must never
        // match anything, even another empty-quick_hash row.
        insert_online_video(
            &db,
            "video-empty-hash",
            &path_empty,
            "",
            content.len() as i64,
        );

        let groups = detect_duplicate_groups(&db).unwrap();
        assert_eq!(groups.len(), 1);
        let mut ids: Vec<&str> = groups[0]
            .members
            .iter()
            .map(|m| m.video_id.as_str())
            .collect();
        ids.sort();
        assert_eq!(
            ids,
            vec!["video-a", "video-b"],
            "offline rows and empty-quick_hash rows must never be included in a group"
        );
    }

    #[test]
    fn detect_duplicate_groups_always_surfaces_path_collision_pairs_labeled_by_content_match() {
        let (_db_dir, db) = init_temp_db();
        let files_dir = tempfile::tempdir().unwrap();

        // Confirmed pair: identical content.
        let confirmed_content = b"same content on both sides of the collision";
        let confirmed_a = files_dir.path().join("confirmed_a.mp4");
        let confirmed_b = files_dir.path().join("confirmed_b.mp4");
        fs::write(&confirmed_a, confirmed_content).unwrap();
        fs::write(&confirmed_b, confirmed_content).unwrap();
        insert_online_video(
            &db,
            "confirmed-1",
            &confirmed_a,
            "qh-a",
            confirmed_content.len() as i64,
        );
        insert_online_video(
            &db,
            "confirmed-2",
            &confirmed_b,
            "qh-b",
            confirmed_content.len() as i64,
        );

        // Unconfirmed pair: different content.
        let unconfirmed_a = files_dir.path().join("unconfirmed_a.mp4");
        let unconfirmed_b = files_dir.path().join("unconfirmed_b.mp4");
        fs::write(&unconfirmed_a, b"content one").unwrap();
        fs::write(&unconfirmed_b, b"content two, deliberately different").unwrap();
        insert_online_video(&db, "unconfirmed-1", &unconfirmed_a, "qh-c", 11);
        insert_online_video(&db, "unconfirmed-2", &unconfirmed_b, "qh-d", 22);

        {
            let conn = db.writer.lock().unwrap();
            queries::record_path_collision(
                &conn,
                "confirmed-1",
                "confirmed-2",
                &confirmed_b.to_string_lossy(),
            )
            .unwrap();
            queries::record_path_collision(
                &conn,
                "unconfirmed-1",
                "unconfirmed-2",
                &unconfirmed_b.to_string_lossy(),
            )
            .unwrap();
        }

        let groups = detect_duplicate_groups(&db).unwrap();

        let confirmed_group = groups
            .iter()
            .find(|g| g.members.iter().any(|m| m.video_id == "confirmed-1"))
            .expect("confirmed pair must be surfaced as a group");
        assert!(matches!(
            confirmed_group.kind,
            DuplicateGroupKind::PathCollisionConfirmed
        ));
        assert_eq!(confirmed_group.members.len(), 2);

        let unconfirmed_group = groups
            .iter()
            .find(|g| g.members.iter().any(|m| m.video_id == "unconfirmed-1"))
            .expect("unconfirmed pair must still be surfaced as a group");
        assert!(matches!(
            unconfirmed_group.kind,
            DuplicateGroupKind::PathCollisionUnconfirmed
        ));
        assert_eq!(unconfirmed_group.members.len(), 2);
    }

    #[test]
    fn duplicate_groups_state_get_reflects_the_last_set_snapshot() {
        let state = DuplicateGroupsState::default();
        assert!(state.get().is_empty());

        state.set(vec![DuplicateGroup {
            kind: DuplicateGroupKind::QuickHashConfirmed,
            members: vec![],
        }]);
        assert_eq!(state.get().len(), 1);
    }
}
