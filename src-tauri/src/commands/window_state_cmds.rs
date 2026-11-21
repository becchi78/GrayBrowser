//! Remembering the main window size across launches. The
//! `#[tauri::command]` wrappers are thin; the logic lives in plain functions
//! over `&Db` so it can be tested without a Tauri runtime.

use gb_core::window_state::{next_state, parse_saved, SavedWindowState};
use tauri::State;

use crate::db::{queries, Db};

/// The stored size, or `None` when remembering is off or nothing usable is
/// stored.
pub(crate) fn load(db: &Db) -> anyhow::Result<Option<SavedWindowState>> {
    let conn = db.read_pool.get()?;
    if !queries::get_remember_window_size_enabled(&conn)? {
        return Ok(None);
    }
    let raw = queries::get_window_state(&conn)?;
    Ok(parse_saved(raw.as_deref()))
}

/// The enabled check, the read of the previous value and the write all happen
/// under one writer lock, so a save racing with "turn off" can never leave a
/// value behind after the delete.
pub(crate) fn save(db: &Db, current: (u32, u32), maximized: bool) -> anyhow::Result<()> {
    let conn = db.writer.lock().unwrap();
    if !queries::get_remember_window_size_enabled(&conn)? {
        return Ok(());
    }
    let prev = parse_saved(queries::get_window_state(&conn)?.as_deref());
    let json = serde_json::to_string(&next_state(prev, current, maximized))?;
    queries::set_window_state(&conn, &json)
}

/// Turning it off also drops the stored value, so turning it on again later
/// never revives a stale size. One transaction keeps flag and value in step.
pub(crate) fn set_enabled(db: &Db, enabled: bool) -> anyhow::Result<()> {
    let conn = db.writer.lock().unwrap();
    let tx = conn.unchecked_transaction()?;
    queries::set_remember_window_size_enabled(&tx, enabled)?;
    if !enabled {
        queries::delete_window_state(&tx)?;
    }
    tx.commit()?;
    Ok(())
}

#[tauri::command]
pub fn get_remember_window_size_enabled(db: State<Db>) -> Result<bool, String> {
    let conn = db.read_pool.get().map_err(|e| e.to_string())?;
    queries::get_remember_window_size_enabled(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_remember_window_size_enabled(db: State<Db>, enabled: bool) -> Result<(), String> {
    set_enabled(&db, enabled).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(width: u32, height: u32, maximized: bool) -> SavedWindowState {
        SavedWindowState {
            width,
            height,
            maximized,
        }
    }

    #[test]
    fn nothing_stored_loads_none() {
        let (_d, db) = crate::db::test_support::init_temp_db();
        assert_eq!(load(&db).unwrap(), None);
    }

    #[test]
    fn saved_size_round_trips_when_enabled() {
        let (_d, db) = crate::db::test_support::init_temp_db();
        save(&db, (1100, 800), false).unwrap();
        assert_eq!(load(&db).unwrap(), Some(st(1100, 800, false)));
    }

    #[test]
    fn maximized_save_keeps_the_previous_normal_size() {
        let (_d, db) = crate::db::test_support::init_temp_db();
        save(&db, (1100, 800), false).unwrap();
        save(&db, (1920, 1040), true).unwrap();
        assert_eq!(load(&db).unwrap(), Some(st(1100, 800, true)));
        save(&db, (1100, 800), false).unwrap();
        assert_eq!(load(&db).unwrap(), Some(st(1100, 800, false)));
    }

    #[test]
    fn disabled_loads_none_and_does_not_save() {
        let (_d, db) = crate::db::test_support::init_temp_db();
        set_enabled(&db, false).unwrap();
        save(&db, (1100, 800), false).unwrap();
        assert_eq!(load(&db).unwrap(), None);
        let conn = db.writer.lock().unwrap();
        assert_eq!(queries::get_window_state(&conn).unwrap(), None);
    }

    #[test]
    fn disabling_deletes_the_stored_value_and_reenabling_does_not_revive_it() {
        let (_d, db) = crate::db::test_support::init_temp_db();
        save(&db, (1100, 800), false).unwrap();
        set_enabled(&db, false).unwrap();
        {
            let conn = db.writer.lock().unwrap();
            assert_eq!(queries::get_window_state(&conn).unwrap(), None);
        }
        set_enabled(&db, true).unwrap();
        assert_eq!(load(&db).unwrap(), None);
    }

    #[test]
    fn corrupt_stored_value_loads_none_and_is_overwritten_by_the_next_save() {
        let (_d, db) = crate::db::test_support::init_temp_db();
        {
            let conn = db.writer.lock().unwrap();
            queries::set_window_state(&conn, "{broken").unwrap();
        }
        assert_eq!(load(&db).unwrap(), None);
        save(&db, (1000, 700), false).unwrap();
        assert_eq!(load(&db).unwrap(), Some(st(1000, 700, false)));
    }
}
