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

/// Every tag with the number of videos carrying it, in the user's tag order.
#[tauri::command]
pub fn list_tags_with_video_count(db: State<Db>) -> Result<Vec<TagWithCountDto>, String> {
    queries::list_all_tags_with_video_count(&db.read_pool)
        .map(|rows| rows.into_iter().map(TagWithCountDto::from).collect())
        .map_err(|e| e.to_string())
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

/// Stores the order of all tags as the user arranged them. Not a catalog
/// change (no video row is touched), so no catalog notification is sent; the
/// caller reloads the tag list itself.
#[tauri::command]
pub fn set_tag_order(db: State<Db>, tag_ids: Vec<i64>) -> Result<(), String> {
    let mut conn = db.writer.lock().unwrap();
    queries::set_tag_order(&mut conn, &tag_ids).map_err(|e| e.to_string())
}
