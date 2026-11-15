//! Rating command. "Clear rating" is just `set_rating(id, 0)` --
//! there is no separate clear command, matching 0's meaning as "unrated".

use tauri::State;

use crate::db::{queries, Db};
use crate::events::{CatalogNotifier, TauriCatalogNotifier};

/// Validates a rating coming in over IPC, with the message the frontend shows.
pub(crate) fn validate_rating_arg(rating: u8) -> Result<u8, String> {
    gb_core::rating::validate_rating(rating)
        .map_err(|e| format!("rating {} is out of range (must be 0-5)", e.value))
}

#[tauri::command]
pub fn set_rating(
    app: tauri::AppHandle,
    db: State<Db>,
    video_id: String,
    rating: u8,
) -> Result<(), String> {
    let validated = validate_rating_arg(rating)?;
    let conn = db.writer.lock().unwrap();
    queries::set_rating(&conn, &video_id, validated).map_err(|e| e.to_string())?;
    TauriCatalogNotifier::new(app).notify_changed();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_boundaries_of_the_range() {
        assert_eq!(validate_rating_arg(0), Ok(0));
        assert_eq!(validate_rating_arg(5), Ok(5));
    }

    #[test]
    fn rejects_a_rating_above_the_range_with_the_shown_message() {
        assert_eq!(
            validate_rating_arg(6),
            Err("rating 6 is out of range (must be 0-5)".to_string())
        );
    }
}
