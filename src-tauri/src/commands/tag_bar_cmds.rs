//! Tag bar pinned-tags persistence commands. Thin passthroughs to
//! `db::queries`' tag-bar functions -- no additional business logic lives
//! here beyond error stringification.

use tauri::State;

use crate::db::{queries, Db};

/// Returns the persisted tag-bar pin list, self-healing (pruning ids of
/// tags that no longer exist) as a side effect. Uses `db.writer` rather than
/// `db.read_pool` because self-healing can write the pruned list back to the
/// `settings` table, and this app's single-writer-lock convention requires
/// all writes to go through `db.writer`.
#[tauri::command]
pub fn get_tag_bar_pinned_tag_ids(db: State<Db>) -> Result<Vec<i64>, String> {
    let conn = db.writer.lock().unwrap();
    queries::get_tag_bar_pinned_tag_ids_self_healing(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_tag_bar_pinned_tag_ids(db: State<Db>, tag_ids: Vec<i64>) -> Result<(), String> {
    let conn = db.writer.lock().unwrap();
    queries::set_tag_bar_pinned_tag_ids(&conn, &tag_ids).map_err(|e| e.to_string())
}
