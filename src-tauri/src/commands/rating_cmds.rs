//! Rating command. "Clear rating" is just `set_rating(id, 0)` --
//! there is no separate clear command, matching 0's meaning as "unrated".

use tauri::State;

use crate::db::{queries, Db};
use crate::events::{CatalogNotifier, TauriCatalogNotifier};

#[tauri::command]
pub fn set_rating(
    app: tauri::AppHandle,
    db: State<Db>,
    video_id: String,
    rating: u8,
) -> Result<(), String> {
    let validated = gb_core::rating::validate_rating(rating)
        .map_err(|e| format!("rating {} is out of range (must be 0-5)", e.value))?;
    let conn = db.writer.lock().unwrap();
    queries::set_rating(&conn, &video_id, validated).map_err(|e| e.to_string())?;
    TauriCatalogNotifier::new(app).notify_changed();
    Ok(())
}
