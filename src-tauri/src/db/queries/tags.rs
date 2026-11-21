//! Tags and the `video_tags` assignments, including the transactional create/assign/delete paths.

use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, OptionalExtension, Transaction};

pub struct TagRow {
    pub id: i64,
    pub name: String,
}

/// Errors from `assign_tag_to_video` and `assign_tags_to_videos`. Deliberately hand-rolled (no
/// `thiserror`) rather than pulling in a new `src-tauri` dependency for a
/// single error enum -- `thiserror` is already a `gb-core`-only dependency,
/// and adding it here would touch `Cargo.toml`/`Cargo.lock` for no real
/// benefit.
#[derive(Debug)]
pub enum TagMutationError {
    InvalidName(gb_core::tags::TagNameError),
    VideoNotFound { video_id: String },
    EmptyGroupName,
    DuplicateGroupName(String),
    GroupNotFound { group_id: i64 },
    Sqlite(rusqlite::Error),
}

impl std::fmt::Display for TagMutationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TagMutationError::InvalidName(e) => write!(f, "invalid tag name: {e}"),
            TagMutationError::VideoNotFound { video_id } => {
                write!(f, "video {video_id} does not exist")
            }
            TagMutationError::EmptyGroupName => write!(f, "グループ名を入力してください"),
            TagMutationError::DuplicateGroupName(name) => {
                write!(f, "同じ名前のタググループが既にあります: {name}")
            }
            TagMutationError::GroupNotFound { group_id } => {
                write!(f, "タググループ {group_id} が見つかりません")
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
            TagMutationError::VideoNotFound { .. }
            | TagMutationError::EmptyGroupName
            | TagMutationError::DuplicateGroupName(_)
            | TagMutationError::GroupNotFound { .. } => None,
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

/// Assigns several tags (by raw, user-typed names) to many videos in a single
/// transaction, so a failure part-way leaves no tag created and no video
/// tagged. Every name is normalized up front, so one invalid name changes
/// nothing; names that normalize to the same string are applied once. Unlike
/// the single-video variant, an unknown video id is not an error: a video can
/// be removed or moved between the selection and the action, and one stale id
/// must not block tagging the rest. Duplicate ids are applied once. The tag
/// rows are created even when no listed video exists, since the single-video
/// path already leaves reusable tags. Returns the tags in first-seen order.
pub fn assign_tags_to_videos(
    conn: &mut Connection,
    video_ids: &[String],
    raw_tag_names: &[String],
) -> Result<Vec<TagRow>, TagMutationError> {
    let mut names = Vec::with_capacity(raw_tag_names.len());
    for raw in raw_tag_names {
        let normalized = gb_core::tags::normalize_tag_name(raw)?;
        if !names.contains(&normalized) {
            names.push(normalized);
        }
    }

    let tx = conn.transaction()?;
    let tags = names
        .iter()
        .map(|name| get_or_create_tag(&tx, name))
        .collect::<rusqlite::Result<Vec<_>>>()?;

    {
        let mut exists = tx.prepare("SELECT 1 FROM videos WHERE id = ?1")?;
        let mut insert = tx.prepare(
            "INSERT INTO video_tags (video_id, tag_id) VALUES (?1, ?2)
             ON CONFLICT(video_id, tag_id) DO NOTHING",
        )?;
        for video_id in dedup_ids(video_ids) {
            if !exists.exists(params![video_id])? {
                continue;
            }
            for tag in &tags {
                insert.execute(params![video_id, tag.id])?;
            }
        }
    }

    tx.commit()?;
    Ok(tags)
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

/// How many ids go into one `IN (...)` list. Kept well under SQLite's
/// bound-variable limit (999 on older builds) so a large selection never
/// fails on the statement size.
const VIDEO_ID_CHUNK: usize = 500;

/// The ids with duplicates dropped, first occurrence kept. A repeated id must
/// not be counted twice in the per-tag usage.
fn dedup_ids(video_ids: &[String]) -> Vec<&str> {
    let mut seen = std::collections::HashSet::new();
    video_ids
        .iter()
        .map(String::as_str)
        .filter(|id| seen.insert(*id))
        .collect()
}

fn placeholders(n: usize) -> String {
    vec!["?"; n].join(",")
}

/// Un-assigns several tags from many videos in a single transaction, so a
/// failure part-way leaves every video as it was. Only `video_tags` rows go:
/// the `tags` rows stay (as in `remove_tag_from_video`), videos outside the
/// list and tags outside `tag_ids` are untouched, and an unknown video id or
/// tag id simply matches nothing and is not an error. Duplicate ids on either
/// side are harmless.
pub fn remove_tags_from_videos(
    conn: &mut Connection,
    video_ids: &[String],
    tag_ids: &[i64],
) -> rusqlite::Result<()> {
    remove_tags_from_videos_chunked(conn, video_ids, tag_ids, VIDEO_ID_CHUNK)
}

fn remove_tags_from_videos_chunked(
    conn: &mut Connection,
    video_ids: &[String],
    tag_ids: &[i64],
    chunk: usize,
) -> rusqlite::Result<()> {
    let ids = dedup_ids(video_ids);
    let tag_ids = gb_core::tags::dedup_keep_first(tag_ids);
    let tx = conn.transaction()?;
    for tag_id in tag_ids {
        for part in ids.chunks(chunk) {
            let sql = format!(
                "DELETE FROM video_tags WHERE tag_id = ? AND video_id IN ({})",
                placeholders(part.len())
            );
            let params = std::iter::once(rusqlite::types::Value::Integer(tag_id)).chain(
                part.iter()
                    .map(|id| rusqlite::types::Value::Text((*id).to_string())),
            );
            tx.execute(&sql, rusqlite::params_from_iter(params))?;
        }
    }
    tx.commit()
}

/// The tags carried by at least one of the given videos, with how many of
/// them carry each (`video_count`), in the user's tag order. Ids that match no
/// video are not counted, and a repeated id counts once. The chunked counts are
/// read in one transaction so they come from a single snapshot.
pub fn list_tag_usage_for_videos(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    video_ids: &[String],
) -> anyhow::Result<Vec<TagWithCountRow>> {
    list_tag_usage_for_videos_chunked(pool, video_ids, VIDEO_ID_CHUNK)
}

fn list_tag_usage_for_videos_chunked(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    video_ids: &[String],
    chunk: usize,
) -> anyhow::Result<Vec<TagWithCountRow>> {
    let ids = dedup_ids(video_ids);
    let mut conn = pool.get()?;
    let tx = conn.transaction()?;

    let mut counts: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    for part in ids.chunks(chunk) {
        let sql = format!(
            "SELECT vt.tag_id, COUNT(*) FROM video_tags vt
             JOIN videos v ON v.id = vt.video_id
             WHERE vt.video_id IN ({}) GROUP BY vt.tag_id",
            placeholders(part.len())
        );
        let mut stmt = tx.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(part.iter()), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (tag_id, n) = row?;
            *counts.entry(tag_id).or_insert(0) += n;
        }
    }
    if counts.is_empty() {
        return Ok(Vec::new());
    }

    let layout = load_layout(&tx)?;
    Ok(gb_core::tag_layout::flatten(&layout)
        .into_iter()
        .filter_map(|t| {
            counts.get(&t.id).map(|&video_count| TagWithCountRow {
                id: t.id,
                name: t.name.clone(),
                video_count,
            })
        })
        .collect())
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

/// Reads every group and tag and builds the display layout. Takes a plain
/// `&Connection` (a `Transaction` derefs to it) so writers can read inside
/// their own transaction and compute from the same snapshot they then write.
fn load_layout(conn: &Connection) -> rusqlite::Result<gb_core::tag_layout::TagLayout> {
    let groups = conn
        .prepare("SELECT id, name, sort_order FROM tag_groups")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let tags = conn
        .prepare("SELECT id, name, sort_order, group_id FROM tags")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(gb_core::tag_layout::build_layout(&groups, &tags))
}

/// Writes the whole layout back. Every tag and group is renumbered (and every
/// tag's `group_id` rewritten) rather than only the moved ones, so stale
/// numbers or dangling `group_id`s left by earlier edits cannot tie with the
/// fresh ones. The caller owns the transaction.
fn save_layout(tx: &Transaction, layout: &gb_core::tag_layout::TagLayout) -> rusqlite::Result<()> {
    let a = gb_core::tag_layout::assignments(layout);
    let mut tag_stmt =
        tx.prepare("UPDATE tags SET group_id = ?1, sort_order = ?2 WHERE id = ?3")?;
    for t in &a.tags {
        tag_stmt.execute(params![t.group_id, t.sort_order, t.id])?;
    }
    let mut group_stmt = tx.prepare("UPDATE tag_groups SET sort_order = ?1 WHERE id = ?2")?;
    for g in &a.groups {
        group_stmt.execute(params![g.sort_order, g.id])?;
    }
    Ok(())
}

/// Every tag in the `tags` master table, for a tag picker/autocomplete UI --
/// independent of whether it's currently assigned to any video. In the
/// left-pane display order, with each group's tags listed where the group is.
pub fn list_all_tags(pool: &r2d2::Pool<SqliteConnectionManager>) -> anyhow::Result<Vec<TagRow>> {
    let conn = pool.get()?;
    let layout = load_layout(&conn)?;
    Ok(gb_core::tag_layout::flatten(&layout)
        .into_iter()
        .map(|t| TagRow {
            id: t.id,
            name: t.name.clone(),
        })
        .collect())
}

#[derive(Debug)]
pub struct TagWithCountRow {
    pub id: i64,
    pub name: String,
    pub video_count: i64,
}

/// A top-level entry of the tag layout, with usage counts on the tags.
#[derive(Debug)]
pub enum LayoutItemRow {
    Tag(TagWithCountRow),
    Group {
        id: i64,
        name: String,
        tags: Vec<TagWithCountRow>,
    },
}

/// Attaches how many videos carry each tag. Tags with no videos get 0.
fn with_counts(
    conn: &Connection,
    layout: &gb_core::tag_layout::TagLayout,
) -> rusqlite::Result<Vec<LayoutItemRow>> {
    use gb_core::tag_layout::{LayoutItem, TagEntry};
    let counts: std::collections::HashMap<i64, i64> = conn
        .prepare("SELECT tag_id, COUNT(*) FROM video_tags GROUP BY tag_id")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let row = |t: &TagEntry| TagWithCountRow {
        id: t.id,
        name: t.name.clone(),
        video_count: counts.get(&t.id).copied().unwrap_or(0),
    };
    Ok(layout
        .items
        .iter()
        .map(|item| match item {
            LayoutItem::Tag(t) => LayoutItemRow::Tag(row(t)),
            LayoutItem::Group(g) => LayoutItemRow::Group {
                id: g.id,
                name: g.name.clone(),
                tags: g.tags.iter().map(row).collect(),
            },
        })
        .collect())
}

/// The left-pane layout: ungrouped tags and groups in display order.
pub fn get_tag_layout(
    pool: &r2d2::Pool<SqliteConnectionManager>,
) -> anyhow::Result<Vec<LayoutItemRow>> {
    let mut conn = pool.get()?;
    // One transaction so the tags and their counts come from a single snapshot.
    let tx = conn.transaction()?;
    let layout = load_layout(&tx)?;
    Ok(with_counts(&tx, &layout)?)
}

/// Runs `edit` on the current layout and stores the result, all in one
/// transaction; an `Err` from `edit` or any statement rolls everything back.
/// `edit` may also change `tag_groups` rows (create/delete) through `tx`.
/// Returns the stored layout with counts.
fn edit_layout(
    conn: &mut Connection,
    edit: impl FnOnce(
        &Transaction,
        gb_core::tag_layout::TagLayout,
    ) -> Result<Option<gb_core::tag_layout::TagLayout>, TagMutationError>,
) -> Result<Vec<LayoutItemRow>, TagMutationError> {
    let tx = conn.transaction()?;
    let current = load_layout(&tx)?;
    let layout = match edit(&tx, current.clone())? {
        Some(next) => {
            save_layout(&tx, &next)?;
            next
        }
        // Nothing to change (e.g. an edge move): skip the write entirely.
        None => current,
    };
    let rows = with_counts(&tx, &layout)?;
    tx.commit()?;
    Ok(rows)
}

/// Moves a tag or group one step up or down in the visible order. At an edge
/// (or for an unknown id) nothing changes and the current layout is returned.
pub fn move_tag_layout_item(
    conn: &mut Connection,
    item: gb_core::tag_layout::ItemRef,
    direction: gb_core::tag_layout::Direction,
) -> Result<Vec<LayoutItemRow>, TagMutationError> {
    edit_layout(conn, |_, layout| {
        Ok(gb_core::tag_layout::move_item(&layout, item, direction))
    })
}

/// Creates an empty group at the top of the list. The name is normalized like
/// a tag name and must be unique.
pub fn create_tag_group(
    conn: &mut Connection,
    raw_name: &str,
) -> Result<Vec<LayoutItemRow>, TagMutationError> {
    let name = normalize_group_name(raw_name)?;
    edit_layout(conn, |tx, layout| {
        if group_id_by_name(tx, &name)?.is_some() {
            return Err(TagMutationError::DuplicateGroupName(name.clone()));
        }
        tx.execute("INSERT INTO tag_groups (name) VALUES (?1)", params![name])?;
        let id = tx.last_insert_rowid();
        Ok(Some(gb_core::tag_layout::insert_group_at_top(
            &layout, id, &name,
        )))
    })
}

/// Renames a group. Renaming to its current name is a no-op; a name another
/// group already uses is an error.
pub fn rename_tag_group(
    conn: &mut Connection,
    group_id: i64,
    raw_name: &str,
) -> Result<Vec<LayoutItemRow>, TagMutationError> {
    let name = normalize_group_name(raw_name)?;
    edit_layout(conn, |tx, _| {
        let exists = tx
            .query_row(
                "SELECT 1 FROM tag_groups WHERE id = ?1",
                params![group_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Err(TagMutationError::GroupNotFound { group_id });
        }
        match group_id_by_name(tx, &name)? {
            Some(id) if id == group_id => return Ok(None),
            Some(_) => return Err(TagMutationError::DuplicateGroupName(name.clone())),
            None => {}
        }
        tx.execute(
            "UPDATE tag_groups SET name = ?1 WHERE id = ?2",
            params![name, group_id],
        )?;
        // The layout is read before the rename; reload so the saved and
        // returned layout carry the new name.
        Ok(Some(load_layout(tx)?))
    })
}

/// Deletes a group. Its tags stay, at the top level where the group was, and
/// no `video_tags` row is touched.
pub fn delete_tag_group(
    conn: &mut Connection,
    group_id: i64,
) -> Result<Vec<LayoutItemRow>, TagMutationError> {
    edit_layout(conn, |tx, layout| {
        tx.execute("DELETE FROM tag_groups WHERE id = ?1", params![group_id])?;
        Ok(Some(gb_core::tag_layout::remove_group(&layout, group_id)))
    })
}

/// Group names follow the tag-name rules, but an empty name is reported in
/// terms of groups since the message is shown to the user as is.
fn normalize_group_name(raw: &str) -> Result<String, TagMutationError> {
    gb_core::tags::normalize_tag_name(raw).map_err(|_| TagMutationError::EmptyGroupName)
}

fn group_id_by_name(tx: &Transaction, name: &str) -> rusqlite::Result<Option<i64>> {
    tx.query_row(
        "SELECT id FROM tag_groups WHERE name = ?1",
        params![name],
        |r| r.get(0),
    )
    .optional()
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

#[cfg(test)]
mod chunked_bulk_tests {
    use super::*;
    use crate::db::test_support::{init_temp_db, insert_test_video};

    fn seed(db: &crate::db::Db, n: usize) -> Vec<String> {
        let names: Vec<String> = (0..n).map(|i| format!("v{i}")).collect();
        for name in &names {
            insert_test_video(db, name, &format!("D:\\v\\{name}.mp4"));
        }
        let conn = db.writer.lock().unwrap();
        conn.execute("INSERT INTO tags (id, name) VALUES (1, 'a')", [])
            .unwrap();
        for name in &names {
            conn.execute(
                "INSERT INTO video_tags (video_id, tag_id) VALUES (?1, 1)",
                params![name],
            )
            .unwrap();
        }
        names
    }

    #[test]
    fn usage_counts_are_merged_across_chunks() {
        let (_dir, db) = init_temp_db();
        let names = seed(&db, 7);
        // Request a duplicate and an unknown id too; neither may be counted.
        let mut request = names.clone();
        request.push("v0".into());
        request.push("ghost".into());

        let usage = list_tag_usage_for_videos_chunked(&db.read_pool, &request, 2).unwrap();

        assert_eq!(usage.len(), 1);
        assert_eq!((usage[0].id, usage[0].video_count), (1, 7));
    }

    #[test]
    fn removal_spans_chunks_and_spares_unlisted_videos() {
        let (_dir, db) = init_temp_db();
        let names = seed(&db, 7);
        let mut conn = db.writer.lock().unwrap();

        remove_tags_from_videos_chunked(&mut conn, &names[..6], &[1], 2).unwrap();

        let left: Vec<String> = conn
            .prepare("SELECT video_id FROM video_tags")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(left, vec!["v6".to_string()]);
    }

    #[test]
    fn multi_tag_removal_spans_chunks_and_keeps_unselected_tags() {
        let (_dir, db) = init_temp_db();
        let names = seed(&db, 7);
        let mut conn = db.writer.lock().unwrap();
        conn.execute("INSERT INTO tags (id, name) VALUES (2, 'b')", [])
            .unwrap();
        conn.execute("INSERT INTO tags (id, name) VALUES (3, 'c')", [])
            .unwrap();
        for name in &names {
            for tag in [2, 3] {
                conn.execute(
                    "INSERT INTO video_tags (video_id, tag_id) VALUES (?1, ?2)",
                    params![name, tag],
                )
                .unwrap();
            }
        }

        // Tag 1 is listed twice; tag 3 is not listed at all.
        remove_tags_from_videos_chunked(&mut conn, &names[..5], &[1, 2, 1], 2).unwrap();

        let left: Vec<(String, i64)> = conn
            .prepare("SELECT video_id, tag_id FROM video_tags WHERE tag_id IN (1, 2) ORDER BY video_id, tag_id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            left,
            vec![
                ("v5".to_string(), 1),
                ("v5".to_string(), 2),
                ("v6".to_string(), 1),
                ("v6".to_string(), 2),
            ]
        );
        let kept: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM video_tags WHERE tag_id = 3",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, 7);
    }

    #[test]
    fn failure_in_a_later_chunk_rolls_back_earlier_chunks() {
        let (_dir, db) = init_temp_db();
        let names = seed(&db, 6);
        let mut conn = db.writer.lock().unwrap();
        conn.execute(
            "CREATE TRIGGER fail_on_v5 BEFORE DELETE ON video_tags
             WHEN OLD.video_id = 'v5'
             BEGIN SELECT RAISE(ABORT, 'simulated failure'); END;",
            [],
        )
        .unwrap();

        assert!(remove_tags_from_videos_chunked(&mut conn, &names, &[1], 2).is_err());

        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM video_tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 6);
    }
}
