//! Operations that apply to every video under a watch folder (cascade delete, rename, counts).

use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use super::duplicates::record_path_collision;

/// Deletes a video row and every `video_tags` row referencing it, in one
/// transaction.
///
/// Provides the orphan-prevention guarantee video deletion needs; used by
/// `commands::dedup_cmds::delete_duplicate_video`. Does not touch
/// `thumbnails/[id].webp` -- callers that also want to remove the cached
/// thumbnail files must do that separately.
pub fn delete_video_cascade(conn: &mut Connection, video_id: &str) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM video_tags WHERE video_id = ?1",
        params![video_id],
    )?;
    // path_collisions rows reference video_id from either side (the
    // offline row that attempted the move, or the online row that already
    // occupied the path) -- both must go, same app-layer orphan prevention
    // as the video_tags delete above.
    tx.execute(
        "DELETE FROM path_collisions WHERE video_id = ?1 OR colliding_video_id = ?1",
        params![video_id],
    )?;
    tx.execute("DELETE FROM videos WHERE id = ?1", params![video_id])?;
    tx.commit()
}

/// Batch form of `delete_video_cascade`: deletes every listed video row plus
/// the `video_tags`/`path_collisions` rows referencing any of them, all in one
/// transaction so a failure part-way leaves no video half-removed. Ids with no
/// matching row are ignored; returns how many `videos` rows were deleted.
/// Does not touch thumbnail files.
pub fn delete_videos_cascade(conn: &mut Connection, ids: &[String]) -> rusqlite::Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let tx = conn.transaction()?;
    let mut deleted = 0;
    for id in ids {
        tx.execute("DELETE FROM video_tags WHERE video_id = ?1", params![id])?;
        // Both sides of a collision pair, same as `delete_video_cascade`.
        tx.execute(
            "DELETE FROM path_collisions WHERE video_id = ?1 OR colliding_video_id = ?1",
            params![id],
        )?;
        deleted += tx.execute("DELETE FROM videos WHERE id = ?1", params![id])?;
    }
    tx.commit()?;
    Ok(deleted)
}

/// Counts `videos` rows currently under `folder_path` (same folder-boundary-
/// safe `LIKE` matching as `list_videos_filtered`'s `folder_path` argument,
/// via `gb_core::paths::folder_like_prefix`), for the folder-management
/// dialog's delete confirmation (shows the user how many videos' tags,
/// ratings, and registration dates under this folder will be lost).
/// Read-only -- pairs with `delete_videos_under_folder_cascade` below,
/// which performs the actual deletion once the user confirms.
pub fn count_videos_under_folder(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    folder_path: &str,
) -> anyhow::Result<u32> {
    let conn = pool.get()?;
    let pattern = format!("{}%", gb_core::paths::folder_like_prefix(folder_path));
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM videos WHERE file_path LIKE ?1 ESCAPE '\\'",
        params![pattern],
        |r| r.get(0),
    )?;
    Ok(count as u32)
}

/// `(id, file_path)` of every video regardless of `status`. Offline videos are
/// included on purpose: a legacy-imported offline video keeps its linked
/// thumbnail under the folder its path resolves to, so it has to follow when
/// that resolution changes.
pub fn list_video_ids_and_paths(conn: &Connection) -> anyhow::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare("SELECT id, file_path FROM videos")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Deletes every `videos` row under `folder_path` (same folder-boundary-safe
/// `LIKE` matching as `count_videos_under_folder`/`list_videos_filtered`),
/// plus every `video_tags`/`path_collisions` row referencing one of them, in
/// a **single transaction** -- the app-layer orphan-
/// prevention guarantee, extended from `delete_video_cascade`'s one-video
/// case to "every video under a folder" without looping that function call
/// by call (which would split the work across many transactions and risk
/// leaving a partially-deleted folder behind on a mid-loop failure).
///
/// The three `DELETE`s below intentionally re-run the same `videos`-matching
/// `LIKE` subquery/condition (rather than collecting ids into a bound `IN
/// (...)` list first) so `video_tags`/`path_collisions` are always resolved
/// against the *same* still-intact `videos` rows the final `DELETE FROM
/// videos` will remove -- the `videos` rows themselves are only ever touched
/// by that last statement, so every earlier statement's subquery still sees
/// the full untouched set.
///
/// Returns the deleted video ids, mainly for confirmation in tests.
/// `commands::settings_cmds::remove_watch_folder` ignores them: it removes the
/// whole cached-thumbnail subdirectory via
/// `thumbnail::paths::remove_folder_thumbnail_dir`. This function itself
/// never touches the filesystem.
///
/// `skipped_files` rows under `folder_path` are deleted in the same
/// transaction, with the same prefix predicate. Once the folder is no longer
/// watched there is nowhere left to act on such a row (the file is never
/// rescanned, so it can never be registered or cleared by a later scan), and
/// leaving it would accumulate unactionable entries in the skipped list.
pub fn delete_videos_under_folder_cascade(
    conn: &mut Connection,
    folder_path: &str,
) -> rusqlite::Result<Vec<String>> {
    let pattern = format!("{}%", gb_core::paths::folder_like_prefix(folder_path));
    let tx = conn.transaction()?;

    let ids: Vec<String> = {
        let mut stmt = tx.prepare("SELECT id FROM videos WHERE file_path LIKE ?1 ESCAPE '\\'")?;
        let rows = stmt
            .query_map(params![pattern], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    tx.execute(
        "DELETE FROM video_tags WHERE video_id IN (
             SELECT id FROM videos WHERE file_path LIKE ?1 ESCAPE '\\'
         )",
        params![pattern],
    )?;
    tx.execute(
        "DELETE FROM path_collisions WHERE video_id IN (
             SELECT id FROM videos WHERE file_path LIKE ?1 ESCAPE '\\'
         ) OR colliding_video_id IN (
             SELECT id FROM videos WHERE file_path LIKE ?1 ESCAPE '\\'
         )",
        params![pattern],
    )?;
    tx.execute(
        "DELETE FROM videos WHERE file_path LIKE ?1 ESCAPE '\\'",
        params![pattern],
    )?;
    tx.execute(
        "DELETE FROM skipped_files WHERE file_path LIKE ?1 ESCAPE '\\'",
        params![pattern],
    )?;

    tx.commit()?;
    Ok(ids)
}

/// Result of `rename_watch_folder_videos`: how many rows under the folder
/// were rewritten to the new path vs. left untouched because the new path
/// would collide with another video's `file_path`, plus the full identity/
/// path pair for every row that was *actually* rewritten
/// (`renamed_videos`).
///
/// `renamed_videos` exists specifically so `commands::settings_cmds::
/// rename_watch_folder` can move each renamed video's cached thumbnails
/// individually, one video at a time -- never as a single whole-
/// subdirectory rename of the old folder's thumbnail subdirectory to the
/// new one. A collision-skipped row's `file_path` never actually changed,
/// so its resolved thumbnail subdirectory (`gb_core::paths::
/// resolve_thumbnail_subdir`) hasn't changed either; a caller that moved
/// the *whole* subdirectory instead would incorrectly drag a collision-
/// skipped video's thumbnails along with it, leaving them physically
/// relocated to a folder the video's own DB row no longer (or never did)
/// belong to.
pub struct RenameWatchFolderOutcome {
    pub renamed_count: u32,
    pub collision_skipped_count: u32,
    pub renamed_videos: Vec<RenamedVideo>,
}

/// One row actually rewritten by `rename_watch_folder_videos` (i.e. not
/// collision-skipped) -- its identity plus its `file_path` before and after
/// the rename.
pub struct RenamedVideo {
    pub video_id: String,
    pub old_file_path: String,
    pub new_file_path: String,
}

/// Checks whether `file_path` currently belongs to any *other* video row,
/// regardless of `status`. Broader than `is_path_used_by_online_video`'s
/// online-only check: `videos.file_path` is `UNIQUE` across *every* row
/// regardless of status (an offline row's `file_path` is never cleared, only
/// its `status` flips), so `rename_watch_folder_videos`'s batch `UPDATE`
/// must also treat an offline occupant as a collision -- otherwise the
/// `UPDATE` could hit the same `UNIQUE` violation `update_video_path_and_
/// status`'s doc comment warns about, but this time inside a transaction
/// whose atomicity guarantee we cannot afford to break.
fn find_colliding_video_id(
    tx: &Transaction,
    file_path: &str,
    excluding_id: &str,
) -> rusqlite::Result<Option<String>> {
    tx.query_row(
        "SELECT id FROM videos WHERE file_path = ?1 AND id != ?2",
        params![file_path, excluding_id],
        |r| r.get(0),
    )
    .optional()
}

/// The folder-management dialog's ✎ path edit: rewrites every
/// `videos.file_path` under `old_folder_path` to fall under
/// `new_folder_path` instead, preserving `id`/`file_name`/tags/
/// rating/`created_at` untouched -- only `file_path` and (per-row) `status`
/// change. Same folder-boundary-safe `LIKE` matching as
/// `delete_videos_under_folder_cascade`/`count_videos_under_folder`, and the
/// same "snapshot the affected rows before mutating anything" shape.
///
/// For each matched row:
/// - the new path is computed via `gb_core::paths::replace_folder_prefix`,
///   which only rewrites the folder portion -- `file_name` never changes, so
///   this function does not touch that column at all;
/// - if the new path would collide with another row's `file_path`
///   (`find_colliding_video_id`, any status), the row is left completely
///   untouched (old `file_path`/`status` preserved) and the collision is
///   recorded via `record_path_collision` for the duplicate-candidates UI to
///   surface later;
/// - otherwise `file_path` is rewritten and `status` is set to `"online"` or
///   `"offline"` based on a fresh `std::fs` existence check against the new
///   path -- no separate rescan is needed to reflect the move.
///
/// `skipped_files` rows under the folder are rewritten the same way (same
/// prefix match, same `replace_folder_prefix`), so a stale "unregistered" row
/// pointing at the old path does not linger and then duplicate the row a
/// rescan creates under the new path. Unlike videos, a row whose new path is
/// already taken by another `skipped_files` row (compared ASCII-caselessly,
/// like the prefix match; the `UNIQUE` constraint itself is case-sensitive) is
/// *deleted* rather than kept: the existing row came from a rescan of the new
/// location and is the authoritative one. Skipped rows are not counted in the
/// outcome.
///
/// All of the above happens inside a single transaction: either
/// every non-colliding row is rewritten, or (on an `Err`) none of them are.
pub fn rename_watch_folder_videos(
    conn: &mut Connection,
    old_folder_path: &str,
    new_folder_path: &str,
) -> rusqlite::Result<RenameWatchFolderOutcome> {
    let pattern = format!("{}%", gb_core::paths::folder_like_prefix(old_folder_path));
    let tx = conn.transaction()?;

    let rows: Vec<(String, String)> = {
        let mut stmt =
            tx.prepare("SELECT id, file_path FROM videos WHERE file_path LIKE ?1 ESCAPE '\\'")?;
        let rows = stmt
            .query_map(params![pattern], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    let mut renamed_count = 0u32;
    let mut collision_skipped_count = 0u32;
    let mut renamed_videos = Vec::new();

    for (video_id, old_path) in rows {
        let Some(new_path) =
            gb_core::paths::replace_folder_prefix(&old_path, old_folder_path, new_folder_path)
        else {
            // Defensive only: every row here was already selected by the
            // same-shaped `LIKE` prefix match above, so this should be
            // unreachable. Leave the row untouched rather than guess.
            log::warn!(
                "rename_watch_folder_videos: {old_path} matched the folder LIKE prefix but \
                 replace_folder_prefix could not rewrite it under {old_folder_path} -- leaving \
                 it untouched"
            );
            collision_skipped_count += 1;
            continue;
        };

        if let Some(colliding_id) = find_colliding_video_id(&tx, &new_path, &video_id)? {
            record_path_collision(&tx, &video_id, &colliding_id, &new_path)?;
            collision_skipped_count += 1;
            continue;
        }

        let status = if std::path::Path::new(&new_path).exists() {
            "online"
        } else {
            "offline"
        };
        tx.execute(
            "UPDATE videos SET file_path = ?1, status = ?2 WHERE id = ?3",
            params![new_path, status, video_id],
        )?;
        renamed_count += 1;
        renamed_videos.push(RenamedVideo {
            video_id,
            old_file_path: old_path,
            new_file_path: new_path,
        });
    }

    rename_skipped_files_under_folder(&tx, &pattern, old_folder_path, new_folder_path)?;

    tx.commit()?;
    Ok(RenameWatchFolderOutcome {
        renamed_count,
        collision_skipped_count,
        renamed_videos,
    })
}

/// Rewrites `skipped_files.file_path` under `old_folder_path` to the new
/// folder, deleting a row instead when another row already occupies the
/// rewritten path. `file_path` is the only path-derived column (`file_name`
/// is unchanged by a folder rename). Runs inside the caller's transaction.
fn rename_skipped_files_under_folder(
    tx: &Transaction,
    pattern: &str,
    old_folder_path: &str,
    new_folder_path: &str,
) -> rusqlite::Result<()> {
    let rows: Vec<(i64, String)> = {
        let mut stmt = tx.prepare(
            "SELECT id, file_path FROM skipped_files WHERE file_path LIKE ?1 ESCAPE '\\'",
        )?;
        let rows = stmt
            .query_map(params![pattern], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    for (id, old_path) in rows {
        let Some(new_path) =
            gb_core::paths::replace_folder_prefix(&old_path, old_folder_path, new_folder_path)
        else {
            continue;
        };
        let occupied: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM skipped_files
             WHERE lower(file_path) = lower(?1) AND id != ?2)",
            params![new_path, id],
            |r| r.get(0),
        )?;
        if occupied {
            tx.execute("DELETE FROM skipped_files WHERE id = ?1", params![id])?;
        } else {
            tx.execute(
                "UPDATE skipped_files SET file_path = ?1 WHERE id = ?2",
                params![new_path, id],
            )?;
        }
    }
    Ok(())
}
