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

#[derive(Serialize, Clone)]
pub struct AssignTagResultDto {
    pub tag: TagDto,
    pub newly_tagged: u32,
    pub already_tagged: u32,
    pub missing: u32,
}

/// Assigns one tag to many videos at once. Videos that already have it or no
/// longer exist are reported in the counts rather than failing the batch.
// async: a large batch holds the writer lock for the whole transaction, so
// keep it off the main thread.
#[tauri::command(async)]
pub fn assign_tag_to_videos(
    app: tauri::AppHandle,
    db: State<Db>,
    video_ids: Vec<String>,
    tag_name: String,
) -> Result<AssignTagResultDto, String> {
    let mut conn = db.writer.lock().unwrap();
    let outcome = queries::assign_tag_to_videos(&mut conn, &video_ids, &tag_name)
        .map_err(|e| e.to_string())?;
    drop(conn);
    TauriCatalogNotifier::new(app).notify_changed();
    Ok(AssignTagResultDto {
        tag: outcome.tag.into(),
        newly_tagged: outcome.newly_tagged,
        already_tagged: outcome.already_tagged,
        missing: outcome.missing,
    })
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

fn set_tag_order_impl(db: &Db, tag_ids: &[i64]) -> Result<(), String> {
    let mut conn = db.writer.lock().unwrap();
    queries::set_tag_order(&mut conn, tag_ids).map_err(|e| e.to_string())
}

/// Stores the order of all tags as the user arranged them. Not a catalog
/// change (no video row is touched), so no catalog notification is sent; the
/// caller reloads the tag list itself.
#[tauri::command]
pub fn set_tag_order(db: State<Db>, tag_ids: Vec<i64>) -> Result<(), String> {
    set_tag_order_impl(&db, &tag_ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::init_temp_db;

    #[test]
    fn set_tag_order_impl_persists_the_order_for_list_all_tags() {
        let (_dir, db) = init_temp_db();
        {
            let conn = db.writer.lock().unwrap();
            conn.execute_batch("INSERT INTO tags (id, name) VALUES (1, 'a'), (2, 'b'), (3, 'c');")
                .unwrap();
        }
        set_tag_order_impl(&db, &[3, 1, 2]).unwrap();
        let names: Vec<String> = queries::list_all_tags(&db.read_pool)
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, ["c", "a", "b"]);
    }
}
