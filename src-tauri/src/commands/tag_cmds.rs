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
