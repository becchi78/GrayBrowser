//! Restoring the last search/folder/sort/filter state at launch. The
//! `#[tauri::command]` wrappers are thin; the logic lives in plain functions
//! over `&Db` so it can be tested without a Tauri runtime.

use gb_core::view_state::{parse_saved, sanitize, SavedViewState};
use tauri::State;

use crate::db::{queries, Db};

/// Returns the saved state cleaned against the current folders and tags, or
/// `None` when restoring is off.
fn load(db: &Db) -> anyhow::Result<Option<SavedViewState>> {
    let conn = db.read_pool.get()?;
    if !queries::get_restore_view_state_enabled(&conn)? {
        return Ok(None);
    }
    let raw = queries::get_last_view_state(&conn)?;
    let watch_folders = queries::get_watch_folders(&conn)?;
    drop(conn);
    let tag_ids: Vec<i64> = queries::list_all_tags(&db.read_pool)?
        .into_iter()
        .map(|t| t.id)
        .collect();
    Ok(Some(sanitize(
        parse_saved(raw.as_deref()),
        &watch_folders,
        &tag_ids,
    )))
}

/// The enabled check and the write happen under one writer lock, so a save
/// racing with "turn off" can never leave a value behind after the delete.
fn save(db: &Db, state: &SavedViewState) -> anyhow::Result<()> {
    let json = serde_json::to_string(state)?;
    let conn = db.writer.lock().unwrap();
    if !queries::get_restore_view_state_enabled(&conn)? {
        return Ok(());
    }
    queries::set_last_view_state(&conn, &json)
}

/// Turning it off also drops the stored value, so turning it on again later
/// never revives a stale state. One transaction keeps flag and value in step.
fn set_enabled(db: &Db, enabled: bool) -> anyhow::Result<()> {
    let conn = db.writer.lock().unwrap();
    let tx = conn.unchecked_transaction()?;
    queries::set_restore_view_state_enabled(&tx, enabled)?;
    if !enabled {
        queries::delete_last_view_state(&tx)?;
    }
    tx.commit()?;
    Ok(())
}

#[tauri::command]
pub fn get_restore_view_state_enabled(db: State<Db>) -> Result<bool, String> {
    let conn = db.read_pool.get().map_err(|e| e.to_string())?;
    queries::get_restore_view_state_enabled(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_restore_view_state_enabled(db: State<Db>, enabled: bool) -> Result<(), String> {
    set_enabled(&db, enabled).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn load_view_state(db: State<Db>) -> Result<Option<SavedViewState>, String> {
    load(&db).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_view_state(db: State<Db>, state: SavedViewState) -> Result<(), String> {
    save(&db, &state).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SavedViewState {
        SavedViewState {
            search: "abc".into(),
            folder_path: None,
            sort_field: "rating".into(),
            sort_direction: "desc".into(),
            tag_ids: vec![],
            ratings: vec![4],
        }
    }

    #[test]
    fn enabled_by_default_and_unknown_value_reads_as_enabled() {
        let (_d, db) = crate::db::test_support::init_temp_db();
        let conn = db.writer.lock().unwrap();
        assert!(queries::get_restore_view_state_enabled(&conn).unwrap());
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('restore_view_state_enabled', 'garbage')",
            [],
        )
        .unwrap();
        assert!(queries::get_restore_view_state_enabled(&conn).unwrap());
    }

    #[test]
    fn saved_state_round_trips_when_enabled() {
        let (_d, db) = crate::db::test_support::init_temp_db();
        assert_eq!(load(&db).unwrap(), Some(SavedViewState::default()));
        save(&db, &sample()).unwrap();
        assert_eq!(load(&db).unwrap(), Some(sample()));
    }

    #[test]
    fn disabled_loads_none_and_does_not_save() {
        let (_d, db) = crate::db::test_support::init_temp_db();
        set_enabled(&db, false).unwrap();
        save(&db, &sample()).unwrap();
        assert_eq!(load(&db).unwrap(), None);
        let conn = db.writer.lock().unwrap();
        assert_eq!(queries::get_last_view_state(&conn).unwrap(), None);
    }

    #[test]
    fn disabling_deletes_the_stored_value_and_reenabling_does_not_revive_it() {
        let (_d, db) = crate::db::test_support::init_temp_db();
        save(&db, &sample()).unwrap();
        set_enabled(&db, false).unwrap();
        set_enabled(&db, true).unwrap();
        assert_eq!(load(&db).unwrap(), Some(SavedViewState::default()));
    }

    #[test]
    fn corrupt_stored_value_loads_as_default() {
        let (_d, db) = crate::db::test_support::init_temp_db();
        {
            let conn = db.writer.lock().unwrap();
            queries::set_last_view_state(&conn, "{broken").unwrap();
        }
        assert_eq!(load(&db).unwrap(), Some(SavedViewState::default()));
    }

    #[test]
    fn missing_folder_and_deleted_tag_fall_back() {
        let (_d, db) = crate::db::test_support::init_temp_db();
        {
            let conn = db.writer.lock().unwrap();
            queries::set_watch_folders(&conn, &["C:\\kept".to_string()]).unwrap();
        }
        let mut s = sample();
        s.folder_path = Some("C:\\removed".into());
        s.tag_ids = vec![9999];
        save(&db, &s).unwrap();
        let loaded = load(&db).unwrap().unwrap();
        assert_eq!(loaded.folder_path, None);
        assert!(loaded.tag_ids.is_empty());
        assert_eq!(loaded.search, "abc");

        {
            let conn = db.writer.lock().unwrap();
            queries::set_watch_folders(&conn, &["C:\\removed".to_string()]).unwrap();
        }
        assert_eq!(
            load(&db).unwrap().unwrap().folder_path.as_deref(),
            Some("C:\\removed")
        );
    }
}
