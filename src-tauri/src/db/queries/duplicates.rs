//! Records of different files that resolve to the same registered path.

use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection};

pub struct PathCollisionRow {
    pub video_id: String,
    pub colliding_video_id: String,
    pub attempted_path: String,
}

/// Records a duplicate-candidate pair pending full_hash confirmation
/// (`dedup::detect_duplicate_groups` reads these back uniformly). Used in two
/// situations in `scan::mod`, both producing the same "offline candidate
/// `video_id` / online row `colliding_video_id` may share content" shape:
/// - A path-follow target is taken (`register_new_path`'s
///   `BlockedByCollision`): `video_id` (the offline row) tried to follow its
///   path to `attempted_path`, but `colliding_video_id` (an online row)
///   already occupies that exact path.
/// - A rehash matches an offline row (`reconcile_known_path`'s `NeedsRehash` arm):
///   `colliding_video_id` (an online row, rehashed in place) happens to now
///   share quick_hash+file_size with `video_id` (an unrelated offline row);
///   `attempted_path` is `colliding_video_id`'s own current path, not a path
///   `video_id` tried to claim.
///
/// Re-detecting the exact same pair (e.g. a repeat scan/poll before the user
/// resolves it) only refreshes `detected_at`/`attempted_path`, mirroring
/// `upsert_skipped_file`'s "re-detection just bumps the timestamp" pattern --
/// it must not pile up duplicate rows for the same standing collision.
pub fn record_path_collision(
    conn: &Connection,
    video_id: &str,
    colliding_video_id: &str,
    attempted_path: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO path_collisions (video_id, colliding_video_id, attempted_path)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(video_id, colliding_video_id)
         DO UPDATE SET detected_at = CURRENT_TIMESTAMP, attempted_path = excluded.attempted_path",
        params![video_id, colliding_video_id, attempted_path],
    )?;
    Ok(())
}

pub fn list_path_collisions(
    pool: &r2d2::Pool<SqliteConnectionManager>,
) -> anyhow::Result<Vec<PathCollisionRow>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT video_id, colliding_video_id, attempted_path
         FROM path_collisions ORDER BY detected_at DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(PathCollisionRow {
                video_id: r.get(0)?,
                colliding_video_id: r.get(1)?,
                attempted_path: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
