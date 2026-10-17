//! Info-pane collapsed-state persistence commands. Thin passthroughs to
//! `db::queries`' info-pane functions.

use tauri::State;

use crate::db::{queries, Db};

#[tauri::command]
pub fn get_info_pane_collapsed(db: State<Db>) -> Result<bool, String> {
    let conn = db.read_pool.get().map_err(|e| e.to_string())?;
    queries::get_info_pane_collapsed(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_info_pane_collapsed(db: State<Db>, collapsed: bool) -> Result<(), String> {
    let conn = db.writer.lock().unwrap();
    queries::set_info_pane_collapsed(&conn, collapsed).map_err(|e| e.to_string())
}
