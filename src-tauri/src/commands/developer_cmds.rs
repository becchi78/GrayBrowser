//! Commands that only developer mode may use. The menu hides them when the mode
//! is off, but the check is repeated here so a request that skips the menu is
//! refused too.

use rusqlite::Connection;
use tauri::{State, Webview};

use crate::db::{queries, Db};

pub(crate) const DEVELOPER_MODE_REQUIRED: &str = "開発者モードがオフのため、実行できません。";

/// `Err` unless developer mode is on. A failure to read the setting also
/// refuses: an unknown state must not unlock developer tools.
pub(crate) fn ensure_developer_mode(conn: &Connection) -> Result<(), String> {
    match queries::get_developer_mode(conn) {
        Ok(true) => Ok(()),
        Ok(false) => Err(DEVELOPER_MODE_REQUIRED.to_string()),
        Err(e) => Err(e.to_string()),
    }
}

/// Opens the web inspector for the calling window. With the `devtools` feature
/// Tauri injects a Ctrl+Shift+I handler that calls the internal toggle
/// command; that command is denied in the capability file, and the window is
/// configured with the inspector disabled (F12, context menu), so this
/// developer-mode-checked command is meant to be the way in.
#[tauri::command]
pub fn open_devtools(webview: Webview, db: State<Db>) -> Result<(), String> {
    {
        let conn = db.read_pool.get().map_err(|e| e.to_string())?;
        ensure_developer_mode(&conn)?;
    }
    webview.open_devtools();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::init_temp_db;

    #[test]
    fn refused_by_default_and_after_turning_the_mode_off() {
        let (_d, db) = init_temp_db();
        let conn = db.writer.lock().unwrap();
        assert_eq!(
            ensure_developer_mode(&conn),
            Err(DEVELOPER_MODE_REQUIRED.to_string())
        );
        queries::set_developer_mode(&conn, true).unwrap();
        assert_eq!(ensure_developer_mode(&conn), Ok(()));
        queries::set_developer_mode(&conn, false).unwrap();
        assert!(ensure_developer_mode(&conn).is_err());
    }
}
