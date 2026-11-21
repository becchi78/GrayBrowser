//! Tag CRUD commands. Thin passthroughs to `db::queries`' already
//! app-layer-integrity-guaranteed tag functions -- no additional
//! business logic lives here beyond DTO mapping and error stringification.

use serde::Serialize;
use tauri::State;

use crate::db::{queries, Db};
use crate::events::{CatalogNotifier, TauriCatalogNotifier};

#[derive(Serialize, Clone)]
pub struct TagDto {
    pub id: i64,
    pub name: String,
}

impl From<queries::TagRow> for TagDto {
    fn from(row: queries::TagRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
        }
    }
}

/// Assigns a tag (by raw, user-typed name) to a video. Errors (invalid name,
/// unknown `video_id`) are surfaced as strings for the frontend to display,
/// not swallowed -- e.g. tagging a video that went offline/was removed
/// between the grid loading and the user acting on it.
#[tauri::command]
pub fn assign_tag(
    app: tauri::AppHandle,
    db: State<Db>,
    video_id: String,
    tag_name: String,
) -> Result<TagDto, String> {
    let mut conn = db.writer.lock().unwrap();
    let tag = queries::assign_tag_to_video(&mut conn, &video_id, &tag_name)
        .map(TagDto::from)
        .map_err(|e| e.to_string())?;
    TauriCatalogNotifier::new(app).notify_changed();
    Ok(tag)
}

/// Assigns several tags to many videos at once, all or nothing. Videos that
/// already have a tag or no longer exist are skipped rather than failing the
/// batch; an invalid name fails the whole call before anything is written.
// async: a large batch holds the writer lock for the whole transaction, so
// keep it off the main thread.
#[tauri::command(async)]
pub fn assign_tags_to_videos(
    app: tauri::AppHandle,
    db: State<Db>,
    video_ids: Vec<String>,
    tag_names: Vec<String>,
) -> Result<(), String> {
    let mut conn = db.writer.lock().unwrap();
    queries::assign_tags_to_videos(&mut conn, &video_ids, &tag_names).map_err(|e| e.to_string())?;
    drop(conn);
    TauriCatalogNotifier::new(app).notify_changed();
    Ok(())
}

/// Un-assigns several tags from many videos in one transaction. Does not
/// delete the tags themselves; a failure leaves every video as it was.
// async: a large batch holds the writer lock for the whole transaction, so
// keep it off the main thread.
#[tauri::command(async)]
pub fn remove_tags_from_videos(
    app: tauri::AppHandle,
    db: State<Db>,
    video_ids: Vec<String>,
    tag_ids: Vec<i64>,
) -> Result<(), String> {
    let mut conn = db.writer.lock().unwrap();
    queries::remove_tags_from_videos(&mut conn, &video_ids, &tag_ids).map_err(|e| e.to_string())?;
    drop(conn);
    TauriCatalogNotifier::new(app).notify_changed();
    Ok(())
}

/// The tags carried by the given videos, each with how many of those videos
/// carry it (`video_count`), so the UI can tell a tag on every selected video
/// from one on only some.
#[tauri::command(async)]
pub fn list_tag_usage_for_videos(
    db: State<Db>,
    video_ids: Vec<String>,
) -> Result<Vec<TagWithCountDto>, String> {
    queries::list_tag_usage_for_videos(&db.read_pool, &video_ids)
        .map(|rows| rows.into_iter().map(TagWithCountDto::from).collect())
        .map_err(|e| e.to_string())
}

/// Un-assigns a tag from a video. Does not delete the tag itself (see
/// `queries::remove_tag_from_video`'s doc comment).
#[tauri::command]
pub fn remove_tag(
    app: tauri::AppHandle,
    db: State<Db>,
    video_id: String,
    tag_id: i64,
) -> Result<(), String> {
    let conn = db.writer.lock().unwrap();
    queries::remove_tag_from_video(&conn, &video_id, tag_id).map_err(|e| e.to_string())?;
    TauriCatalogNotifier::new(app).notify_changed();
    Ok(())
}

#[tauri::command]
pub fn list_tags_for_video(db: State<Db>, video_id: String) -> Result<Vec<TagDto>, String> {
    queries::list_tags_for_video(&db.read_pool, &video_id)
        .map(|rows| rows.into_iter().map(TagDto::from).collect())
        .map_err(|e| e.to_string())
}

/// Every known tag, for a simple existing-tag suggestion list while typing
/// (not an incremental tag search/management screen).
#[tauri::command]
pub fn list_all_tags(db: State<Db>) -> Result<Vec<TagDto>, String> {
    queries::list_all_tags(&db.read_pool)
        .map(|rows| rows.into_iter().map(TagDto::from).collect())
        .map_err(|e| e.to_string())
}

#[derive(Serialize, Clone)]
pub struct TagWithCountDto {
    pub id: i64,
    pub name: String,
    pub video_count: i64,
}

impl From<queries::TagWithCountRow> for TagWithCountDto {
    fn from(row: queries::TagWithCountRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            video_count: row.video_count,
        }
    }
}

/// Deletes a tag and detaches it from every video. Notifies a catalog change
/// because the video list shows each video's tags.
#[tauri::command]
pub fn delete_tag(app: tauri::AppHandle, db: State<Db>, tag_id: i64) -> Result<(), String> {
    {
        let mut conn = db.writer.lock().unwrap();
        queries::delete_tag(&mut conn, tag_id).map_err(|e| e.to_string())?;
    }
    TauriCatalogNotifier::new(app).notify_changed();
    Ok(())
}

/// A top-level entry of the left-pane tag layout.
#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TagLayoutItemDto {
    Tag(TagWithCountDto),
    Group {
        id: i64,
        name: String,
        tags: Vec<TagWithCountDto>,
    },
}

impl From<queries::LayoutItemRow> for TagLayoutItemDto {
    fn from(row: queries::LayoutItemRow) -> Self {
        match row {
            queries::LayoutItemRow::Tag(t) => TagLayoutItemDto::Tag(t.into()),
            queries::LayoutItemRow::Group { id, name, tags } => TagLayoutItemDto::Group {
                id,
                name,
                tags: tags.into_iter().map(TagWithCountDto::from).collect(),
            },
        }
    }
}

fn layout_dto(rows: Vec<queries::LayoutItemRow>) -> Vec<TagLayoutItemDto> {
    rows.into_iter().map(TagLayoutItemDto::from).collect()
}

/// Ungrouped tags and tag groups in display order, tags with their video
/// counts.
#[tauri::command]
pub fn get_tag_layout(db: State<Db>) -> Result<Vec<TagLayoutItemDto>, String> {
    queries::get_tag_layout(&db.read_pool)
        .map(layout_dto)
        .map_err(|e| e.to_string())
}

/// Moves a tag (`kind` = "tag") or group (`kind` = "group") one step up or
/// down in the visible order, and returns the resulting layout. Not a catalog
/// change, so no notification is sent.
#[tauri::command]
pub fn move_tag_layout_item(
    db: State<Db>,
    kind: String,
    id: i64,
    direction: String,
) -> Result<Vec<TagLayoutItemDto>, String> {
    let item = gb_core::tag_layout::ItemRef::parse(&kind, id)
        .ok_or_else(|| format!("unknown kind: {kind}"))?;
    let direction = gb_core::tag_layout::Direction::parse(&direction)
        .ok_or_else(|| format!("unknown direction: {direction}"))?;
    let mut conn = db.writer.lock().unwrap();
    queries::move_tag_layout_item(&mut conn, item, direction)
        .map(layout_dto)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_tag_group(db: State<Db>, name: String) -> Result<Vec<TagLayoutItemDto>, String> {
    let mut conn = db.writer.lock().unwrap();
    queries::create_tag_group(&mut conn, &name)
        .map(layout_dto)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn rename_tag_group(
    db: State<Db>,
    id: i64,
    name: String,
) -> Result<Vec<TagLayoutItemDto>, String> {
    let mut conn = db.writer.lock().unwrap();
    queries::rename_tag_group(&mut conn, id, &name)
        .map(layout_dto)
        .map_err(|e| e.to_string())
}

/// Deletes a group; its tags stay at the top level. No video's tags change, so
/// no catalog notification is sent.
#[tauri::command]
pub fn delete_tag_group(db: State<Db>, id: i64) -> Result<Vec<TagLayoutItemDto>, String> {
    let mut conn = db.writer.lock().unwrap();
    queries::delete_tag_group(&mut conn, id)
        .map(layout_dto)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_collapsed_tag_groups(db: State<Db>) -> Result<Vec<i64>, String> {
    let conn = db.read_pool.get().map_err(|e| e.to_string())?;
    queries::get_collapsed_tag_groups(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_collapsed_tag_groups(db: State<Db>, ids: Vec<i64>) -> Result<(), String> {
    let conn = db.writer.lock().unwrap();
    queries::set_collapsed_tag_groups(&conn, &ids).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(id: i64, name: &str, video_count: i64) -> queries::TagWithCountRow {
        queries::TagWithCountRow {
            id,
            name: name.to_string(),
            video_count,
        }
    }

    /// The frontend types mirror this exact shape.
    #[test]
    fn layout_dto_json_shape_is_pinned() {
        let dto = layout_dto(vec![
            queries::LayoutItemRow::Tag(count(1, "a", 3)),
            queries::LayoutItemRow::Group {
                id: 7,
                name: "g".into(),
                tags: vec![count(2, "b", 0), count(3, "c", 5)],
            },
            queries::LayoutItemRow::Group {
                id: 8,
                name: "empty".into(),
                tags: vec![],
            },
        ]);
        assert_eq!(
            serde_json::to_value(dto).unwrap(),
            serde_json::json!([
                {"kind": "tag", "id": 1, "name": "a", "video_count": 3},
                {"kind": "group", "id": 7, "name": "g", "tags": [
                    {"id": 2, "name": "b", "video_count": 0},
                    {"id": 3, "name": "c", "video_count": 5},
                ]},
                {"kind": "group", "id": 8, "name": "empty", "tags": []},
            ])
        );
    }
}
