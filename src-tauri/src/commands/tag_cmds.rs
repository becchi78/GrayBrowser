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

fn delete_tag_impl(db: &Db, tag_id: i64) -> Result<(), String> {
    let mut conn = db.writer.lock().unwrap();
    queries::delete_tag(&mut conn, tag_id).map_err(|e| e.to_string())
}

/// Deletes a tag and detaches it from every video. Notifies a catalog change
/// because the video list shows each video's tags.
#[tauri::command]
pub fn delete_tag(app: tauri::AppHandle, db: State<Db>, tag_id: i64) -> Result<(), String> {
    delete_tag_impl(&db, tag_id)?;
    TauriCatalogNotifier::new(app).notify_changed();
    Ok(())
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

    #[test]
    fn delete_tag_impl_removes_the_tag_and_its_video_tags_only() {
        let (_dir, db) = init_temp_db();
        {
            let conn = db.writer.lock().unwrap();
            conn.execute_batch(
                "INSERT INTO tags (id, name, sort_order) VALUES (1, 'a', 0), (2, 'b', 1), (3, 'c', 2);
                 INSERT INTO videos (id, file_path, file_name, file_size, quick_hash, status)
                   VALUES ('v1', 'D:\\a.mp4', 'a.mp4', 1024, 'h1', 'online'),
                          ('v2', 'D:\\b.mp4', 'b.mp4', 1024, 'h2', 'online');
                 INSERT INTO video_tags (video_id, tag_id)
                   VALUES ('v1', 1), ('v2', 1), ('v1', 2), ('v2', 3);",
            )
            .unwrap();
        }
        delete_tag_impl(&db, 1).unwrap();

        let tags = queries::list_all_tags_with_video_count(&db.read_pool).unwrap();
        let summary: Vec<(i64, i64)> = tags.iter().map(|t| (t.id, t.video_count)).collect();
        assert_eq!(summary, [(2, 1), (3, 1)]);
        let conn = db.writer.lock().unwrap();
        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM video_tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 2);
        let on_deleted: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM video_tags WHERE tag_id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(on_deleted, 0);
    }

    #[test]
    fn delete_tag_impl_succeeds_for_an_unknown_tag() {
        let (_dir, db) = init_temp_db();
        delete_tag_impl(&db, 999).unwrap();
    }
}
