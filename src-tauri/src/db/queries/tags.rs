//! Tags and the `video_tags` assignments, including the transactional create/assign/delete paths.

use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, OptionalExtension, Transaction};

pub struct TagRow {
    pub id: i64,
    pub name: String,
}

/// Errors from `assign_tag_to_video`. Deliberately hand-rolled (no
/// `thiserror`) rather than pulling in a new `src-tauri` dependency for a
/// single error enum -- `thiserror` is already a `gb-core`-only dependency,
/// and adding it here would touch `Cargo.toml`/`Cargo.lock` for no real
/// benefit.
#[derive(Debug)]
pub enum TagMutationError {
    InvalidName(gb_core::tags::TagNameError),
    VideoNotFound { video_id: String },
    Sqlite(rusqlite::Error),
}

impl std::fmt::Display for TagMutationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TagMutationError::InvalidName(e) => write!(f, "invalid tag name: {e}"),
            TagMutationError::VideoNotFound { video_id } => {
                write!(f, "video {video_id} does not exist")
            }
            TagMutationError::Sqlite(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TagMutationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TagMutationError::InvalidName(e) => Some(e),
            TagMutationError::Sqlite(e) => Some(e),
            TagMutationError::VideoNotFound { .. } => None,
        }
    }
}

impl From<gb_core::tags::TagNameError> for TagMutationError {
    fn from(e: gb_core::tags::TagNameError) -> Self {
        TagMutationError::InvalidName(e)
    }
}

impl From<rusqlite::Error> for TagMutationError {
    fn from(e: rusqlite::Error) -> Self {
        TagMutationError::Sqlite(e)
    }
}

/// Looks up or creates a tag by its already-normalized name in one round
/// trip. `ON CONFLICT ... RETURNING` makes this atomic and single-statement
/// whether the tag already existed or not -- under this app's single-writer
/// lock there's no real race to protect against, but this keeps the SQL
/// itself provably correct independent of that (no separate
/// SELECT-then-INSERT window at all).
pub(super) fn get_or_create_tag(
    tx: &Transaction,
    normalized_name: &str,
) -> rusqlite::Result<TagRow> {
    tx.query_row(
        "INSERT INTO tags (name) VALUES (?1)
         ON CONFLICT(name) DO UPDATE SET name = excluded.name
         RETURNING id, name",
        params![normalized_name],
        |r| {
            Ok(TagRow {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        },
    )
}

/// Assigns a tag (by raw, user-typed name) to a video, creating the tag if
/// it doesn't already exist. The app-layer referential-integrity guarantee
/// for `video_tags`: both sides of the pair are
/// checked/created to exist *before* the `video_tags` row is inserted, all
/// inside one transaction --
///
/// 1. `gb_core::tags::normalize_tag_name` -- rejects empty/whitespace-only
///    names before any DB access at all.
/// 2. `SELECT 1 FROM videos WHERE id = ?1` -- the `videos.id` side of the
///    check. Returns `VideoNotFound` (and rolls back, touching nothing) if
///    absent, so a rejected `video_id` never leaves behind an unreferenced
///    (if harmless) new `tags` row.
/// 3. `get_or_create_tag` -- the `tags.id` side: the row is guaranteed to
///    exist within this same transaction by the time we reach step 4.
/// 4. `INSERT INTO video_tags ... ON CONFLICT DO NOTHING` -- idempotent:
///    re-adding an already-present tag is a silent no-op (same convention as
///    `insert_video`'s `ON CONFLICT(file_path) DO NOTHING`).
pub fn assign_tag_to_video(
    conn: &mut Connection,
    video_id: &str,
    raw_tag_name: &str,
) -> Result<TagRow, TagMutationError> {
    let normalized_name = gb_core::tags::normalize_tag_name(raw_tag_name)?;

    let tx = conn.transaction()?;

    let video_exists = tx
        .query_row(
            "SELECT 1 FROM videos WHERE id = ?1",
            params![video_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !video_exists {
        return Err(TagMutationError::VideoNotFound {
            video_id: video_id.to_string(),
        });
    }

    let tag = get_or_create_tag(&tx, &normalized_name)?;

    tx.execute(
        "INSERT INTO video_tags (video_id, tag_id) VALUES (?1, ?2)
         ON CONFLICT(video_id, tag_id) DO NOTHING",
        params![video_id, tag.id],
    )?;

    tx.commit()?;
    Ok(tag)
}

/// Result of `assign_tag_to_videos`: how the requested videos split between
/// newly tagged, already carrying the tag, and no longer in the catalog.
pub struct AssignTagOutcome {
    pub tag: TagRow,
    pub newly_tagged: u32,
    pub already_tagged: u32,
    pub missing: u32,
}

/// Assigns one tag (by raw, user-typed name) to many videos in a single
/// transaction. The name is normalized once up front so an invalid name
/// changes nothing. Unlike the single-video variant, an unknown id is not an
/// error: a video can be removed or moved between the selection and the
/// action, and one stale id must not block tagging the rest, so it is counted
/// in `missing` instead. Duplicate ids are counted once. The tag row is
/// created even when no listed video exists, since the caller gets the tag
/// back either way and the single-video path already leaves reusable tags.
pub fn assign_tag_to_videos(
    conn: &mut Connection,
    video_ids: &[String],
    raw_tag_name: &str,
) -> Result<AssignTagOutcome, TagMutationError> {
    let normalized_name = gb_core::tags::normalize_tag_name(raw_tag_name)?;

    let tx = conn.transaction()?;
    let tag = get_or_create_tag(&tx, &normalized_name)?;

    let mut seen = std::collections::HashSet::new();
    let (mut newly_tagged, mut already_tagged, mut missing) = (0u32, 0u32, 0u32);
    for video_id in video_ids {
        if !seen.insert(video_id.as_str()) {
            continue;
        }
        let video_exists = tx
            .query_row(
                "SELECT 1 FROM videos WHERE id = ?1",
                params![video_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !video_exists {
            missing += 1;
            continue;
        }
        let inserted = tx.execute(
            "INSERT INTO video_tags (video_id, tag_id) VALUES (?1, ?2)
             ON CONFLICT(video_id, tag_id) DO NOTHING",
            params![video_id, tag.id],
        )?;
        if inserted > 0 {
            newly_tagged += 1;
        } else {
            already_tagged += 1;
        }
    }

    tx.commit()?;
    Ok(AssignTagOutcome {
        tag,
        newly_tagged,
        already_tagged,
        missing,
    })
}

/// Un-assigns a tag from a video. Deletes only the `video_tags` row --
/// **the `tags` master row is deliberately left in place** even if this was
/// its last reference, so it stays available for reuse without the user
/// having to retype it. A tag
/// disappears from the `tags` table only via an explicit `delete_tag` call.
/// Single-statement delete: removing a `video_tags` row can never create an
/// orphan (it only ever shrinks the set), so no transaction is needed beyond
/// SQLite's own per-statement atomicity.
pub fn remove_tag_from_video(
    conn: &Connection,
    video_id: &str,
    tag_id: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM video_tags WHERE video_id = ?1 AND tag_id = ?2",
        params![video_id, tag_id],
    )?;
    Ok(())
}

pub fn list_tags_for_video(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    video_id: &str,
) -> anyhow::Result<Vec<TagRow>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT tags.id, tags.name FROM tags
         JOIN video_tags ON video_tags.tag_id = tags.id
         WHERE video_tags.video_id = ?1
         ORDER BY tags.name",
    )?;
    let rows = stmt
        .query_map(params![video_id], |r| {
            Ok(TagRow {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Every tag in the `tags` master table, for a tag picker/autocomplete UI --
/// independent of whether it's currently assigned to any video. Ordered by the
/// user-defined `sort_order`; tags without one follow, by name.
pub fn list_all_tags(pool: &r2d2::Pool<SqliteConnectionManager>) -> anyhow::Result<Vec<TagRow>> {
    let conn = pool.get()?;
    let mut stmt =
        conn.prepare("SELECT id, name FROM tags ORDER BY (sort_order IS NULL), sort_order, name")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(TagRow {
                id: r.get(0)?,
                name: r.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub struct TagWithCountRow {
    pub id: i64,
    pub name: String,
    pub video_count: i64,
}

/// Same tags and order as `list_all_tags`, plus how many videos carry each
/// one. LEFT JOIN keeps tags with no videos (count 0), which a plain join
/// would drop.
pub fn list_all_tags_with_video_count(
    pool: &r2d2::Pool<SqliteConnectionManager>,
) -> anyhow::Result<Vec<TagWithCountRow>> {
    let conn = pool.get()?;
    let mut stmt = conn.prepare(
        "SELECT t.id, t.name, COUNT(vt.video_id) \
         FROM tags t LEFT JOIN video_tags vt ON vt.tag_id = t.id \
         GROUP BY t.id \
         ORDER BY (t.sort_order IS NULL), t.sort_order, t.name",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(TagWithCountRow {
                id: r.get(0)?,
                name: r.get(1)?,
                video_count: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Stores the user's tag order: the first id gets 0, the next 1, and so on.
/// Ids with no tag are skipped without leaving a gap, and a repeated id keeps
/// its first position. Every tag not in the list is reset to NULL (placed
/// after the listed ones, by name) rather than left as is: a stale number
/// could equal one just assigned and make the order between the two
/// undefined. One transaction, so a failure leaves the previous order intact.
pub fn set_tag_order(conn: &mut Connection, tag_ids: &[i64]) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute("UPDATE tags SET sort_order = NULL", [])?;
    let mut next = 0_i64;
    {
        let mut stmt = tx.prepare("UPDATE tags SET sort_order = ?1 WHERE id = ?2")?;
        for id in gb_core::tags::dedup_keep_first(tag_ids) {
            if stmt.execute(params![next, id])? > 0 {
                next += 1;
            }
        }
    }
    tx.commit()
}

/// Deletes a tag from the `tags` master table and every `video_tags` row
/// referencing it, in one transaction -- the orphan-prevention cascade
/// required for tag deletion. Order (children before
/// parent) doesn't affect correctness given FK enforcement is off, but
/// matches the semantics that would still hold if it were ever turned back
/// on.
pub fn delete_tag(conn: &mut Connection, tag_id: i64) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM video_tags WHERE tag_id = ?1", params![tag_id])?;
    tx.execute("DELETE FROM tags WHERE id = ?1", params![tag_id])?;
    tx.commit()
}

#[cfg(test)]
mod delete_tag_tests {
    use super::*;
    use crate::db::test_support::init_temp_db;

    #[test]
    fn deleting_an_unknown_tag_succeeds_and_changes_nothing() {
        let (_dir, db) = init_temp_db();
        let mut conn = db.writer.lock().unwrap();
        conn.execute("INSERT INTO tags (id, name) VALUES (1, 'a')", [])
            .unwrap();

        delete_tag(&mut conn, 999).unwrap();

        let tags: i64 = conn
            .query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tags, 1);
    }
}
