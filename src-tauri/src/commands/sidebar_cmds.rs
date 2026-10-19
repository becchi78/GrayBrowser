//! Left-pane section collapsed-state persistence commands. Thin passthroughs
//! to `db::queries`' sidebar-section functions.

use tauri::State;

use crate::db::queries::SidebarSectionsCollapsed;
use crate::db::{queries, Db};

#[tauri::command]
pub fn get_sidebar_sections_collapsed(db: State<Db>) -> Result<SidebarSectionsCollapsed, String> {
    let conn = db.read_pool.get().map_err(|e| e.to_string())?;
    queries::get_sidebar_sections_collapsed(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_sidebar_sections_collapsed(
    db: State<Db>,
    state: SidebarSectionsCollapsed,
) -> Result<(), String> {
    let conn = db.writer.lock().unwrap();
    queries::set_sidebar_sections_collapsed(&conn, state).map_err(|e| e.to_string())
}
