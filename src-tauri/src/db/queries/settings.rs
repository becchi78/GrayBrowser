//! Key/value `settings` table: watch folders, polling interval and UI preferences.

use rusqlite::{params, Connection, OptionalExtension};

fn get_setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .optional()
}

fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
        params![key, value],
    )?;
    Ok(())
}

const WATCH_FOLDERS_KEY: &str = "watch_folders";

pub fn get_watch_folders(conn: &Connection) -> anyhow::Result<Vec<String>> {
    match get_setting(conn, WATCH_FOLDERS_KEY)? {
        Some(json) => Ok(serde_json::from_str(&json)?),
        None => Ok(Vec::new()),
    }
}

/// Why `read_watch_folders` failed. Kept as two variants because callers
/// that log (rather than propagate) the failure word the two causes
/// differently.
#[derive(Debug)]
pub enum WatchFoldersReadError {
    Connection(r2d2::Error),
    Query(anyhow::Error),
}

impl std::fmt::Display for WatchFoldersReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connection(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for WatchFoldersReadError {}

/// Takes a read connection from `pool` and reads the watch folders.
pub fn read_watch_folders(
    pool: &r2d2::Pool<r2d2_sqlite::SqliteConnectionManager>,
) -> Result<Vec<String>, WatchFoldersReadError> {
    let conn = pool.get().map_err(WatchFoldersReadError::Connection)?;
    get_watch_folders(&conn).map_err(WatchFoldersReadError::Query)
}

pub fn set_watch_folders(conn: &Connection, folders: &[String]) -> anyhow::Result<()> {
    let json = serde_json::to_string(folders)?;
    set_setting(conn, WATCH_FOLDERS_KEY, &json)?;
    Ok(())
}

/// Folders whose update check (realtime watch, startup diff scan, NAS
/// polling) is switched off. Absent key means every folder is checked.
const WATCH_CHECK_DISABLED_KEY: &str = "watch_check_disabled_folders";

pub fn get_watch_check_disabled(conn: &Connection) -> anyhow::Result<Vec<String>> {
    match get_setting(conn, WATCH_CHECK_DISABLED_KEY)? {
        Some(json) => Ok(serde_json::from_str(&json)?),
        None => Ok(Vec::new()),
    }
}

pub fn set_watch_check_disabled(conn: &Connection, folders: &[String]) -> anyhow::Result<()> {
    let json = serde_json::to_string(folders)?;
    set_setting(conn, WATCH_CHECK_DISABLED_KEY, &json)?;
    Ok(())
}

const NAS_POLL_INTERVAL_KEY: &str = "nas_polling_interval_sec";

/// Default NAS polling interval: periodic (e.g. every 10 minutes),
/// configurable by the user.
const DEFAULT_NAS_POLL_INTERVAL_SECS: i64 = gb_core::nas_poll_interval::DEFAULT_SECS;

pub fn get_nas_poll_interval_secs(conn: &Connection) -> anyhow::Result<i64> {
    match get_setting(conn, NAS_POLL_INTERVAL_KEY)? {
        Some(value) => Ok(value.parse()?),
        None => Ok(DEFAULT_NAS_POLL_INTERVAL_SECS),
    }
}

pub fn set_nas_poll_interval_secs(conn: &Connection, secs: i64) -> anyhow::Result<()> {
    set_setting(conn, NAS_POLL_INTERVAL_KEY, &secs.to_string())?;
    Ok(())
}

const INFO_PANE_COLLAPSED_KEY: &str = "info_pane_collapsed";

/// Unset and unrecognized values both read as `false` (expanded): a corrupt
/// or hand-edited value must never leave the info column hidden with no
/// obvious cause.
pub fn get_info_pane_collapsed(conn: &Connection) -> anyhow::Result<bool> {
    Ok(get_setting(conn, INFO_PANE_COLLAPSED_KEY)?.as_deref() == Some("1"))
}

pub fn set_info_pane_collapsed(conn: &Connection, collapsed: bool) -> anyhow::Result<()> {
    set_setting(
        conn,
        INFO_PANE_COLLAPSED_KEY,
        if collapsed { "1" } else { "0" },
    )?;
    Ok(())
}

const UPDATE_CHECK_ON_STARTUP_KEY: &str = "update_check_on_startup";

/// Unset and unrecognized values both read as `true`: the check is opt-out, and
/// a corrupt or hand-edited value must not silently stop users from hearing
/// about security fixes.
pub fn get_update_check_on_startup(conn: &Connection) -> anyhow::Result<bool> {
    Ok(get_setting(conn, UPDATE_CHECK_ON_STARTUP_KEY)?.as_deref() != Some("0"))
}

pub fn set_update_check_on_startup(conn: &Connection, enabled: bool) -> anyhow::Result<()> {
    set_setting(
        conn,
        UPDATE_CHECK_ON_STARTUP_KEY,
        if enabled { "1" } else { "0" },
    )?;
    Ok(())
}

const SIDEBAR_SECTIONS_COLLAPSED_KEY: &str = "sidebar_sections_collapsed";

/// Collapsed flags of the left-pane sections. `true` means collapsed; every
/// section defaults to expanded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SidebarSectionsCollapsed {
    pub folders: bool,
    pub rating: bool,
    pub tags: bool,
}

/// Unset and unparseable values both read as all-expanded: a corrupt or
/// hand-edited value must never leave a section hidden with no obvious cause.
pub fn get_sidebar_sections_collapsed(
    conn: &Connection,
) -> anyhow::Result<SidebarSectionsCollapsed> {
    Ok(get_setting(conn, SIDEBAR_SECTIONS_COLLAPSED_KEY)?
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default())
}

pub fn set_sidebar_sections_collapsed(
    conn: &Connection,
    state: SidebarSectionsCollapsed,
) -> anyhow::Result<()> {
    set_setting(
        conn,
        SIDEBAR_SECTIONS_COLLAPSED_KEY,
        &serde_json::to_string(&state)?,
    )?;
    Ok(())
}

const TAG_GROUPS_COLLAPSED_KEY: &str = "tag_groups_collapsed";

/// Ids of the tag groups the user has collapsed. Unset and unparseable values
/// both read as empty (every group open): a corrupt value must not leave tags
/// hidden with no obvious cause. Ids of deleted groups may linger; they match
/// nothing and ids are never reused (AUTOINCREMENT).
pub fn get_collapsed_tag_groups(conn: &Connection) -> anyhow::Result<Vec<i64>> {
    Ok(get_setting(conn, TAG_GROUPS_COLLAPSED_KEY)?
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default())
}

pub fn set_collapsed_tag_groups(conn: &Connection, ids: &[i64]) -> anyhow::Result<()> {
    set_setting(
        conn,
        TAG_GROUPS_COLLAPSED_KEY,
        &serde_json::to_string(&gb_core::tags::dedup_keep_first(ids))?,
    )?;
    Ok(())
}

const RESTORE_VIEW_STATE_ENABLED_KEY: &str = "restore_view_state_enabled";

const LAST_VIEW_STATE_KEY: &str = "last_view_state";

/// Unset and unrecognized values both read as `true`: restoring is opt-out, and
/// a corrupt value must not silently turn the feature off.
pub fn get_restore_view_state_enabled(conn: &Connection) -> anyhow::Result<bool> {
    Ok(get_setting(conn, RESTORE_VIEW_STATE_ENABLED_KEY)?.as_deref() != Some("0"))
}

pub fn set_restore_view_state_enabled(conn: &Connection, enabled: bool) -> anyhow::Result<()> {
    set_setting(
        conn,
        RESTORE_VIEW_STATE_ENABLED_KEY,
        if enabled { "1" } else { "0" },
    )?;
    Ok(())
}

pub fn get_last_view_state(conn: &Connection) -> anyhow::Result<Option<String>> {
    Ok(get_setting(conn, LAST_VIEW_STATE_KEY)?)
}

pub fn set_last_view_state(conn: &Connection, json: &str) -> anyhow::Result<()> {
    set_setting(conn, LAST_VIEW_STATE_KEY, json)?;
    Ok(())
}

pub fn delete_last_view_state(conn: &Connection) -> anyhow::Result<()> {
    conn.execute("DELETE FROM settings WHERE key = ?1", [LAST_VIEW_STATE_KEY])?;
    Ok(())
}

#[cfg(test)]
mod watch_check_disabled_tests {
    use super::*;
    use crate::db::test_support::init_temp_db;

    #[test]
    fn defaults_to_empty_when_the_key_is_absent() {
        let (_dir, db) = init_temp_db();
        let conn = db.writer.lock().unwrap();
        assert!(get_watch_check_disabled(&conn).unwrap().is_empty());
    }

    #[test]
    fn round_trips_in_order_and_is_independent_of_watch_folders() {
        let (_dir, db) = init_temp_db();
        let conn = db.writer.lock().unwrap();
        let disabled = vec![r"D:\Movies".to_string(), r"C:\Videos".to_string()];
        set_watch_check_disabled(&conn, &disabled).unwrap();
        assert_eq!(get_watch_check_disabled(&conn).unwrap(), disabled);
        assert!(get_watch_folders(&conn).unwrap().is_empty());

        set_watch_check_disabled(&conn, &[]).unwrap();
        assert!(get_watch_check_disabled(&conn).unwrap().is_empty());
    }
}

#[cfg(test)]
mod read_watch_folders_tests {
    use super::*;
    use crate::db::test_support::init_temp_db;

    #[test]
    fn reads_the_saved_folders_through_the_pool() {
        let (_dir, db) = init_temp_db();
        {
            let conn = db.writer.lock().unwrap();
            set_watch_folders(&conn, &["A".to_string(), "B".to_string()]).unwrap();
        }
        assert_eq!(
            read_watch_folders(&db.read_pool).unwrap(),
            vec!["A".to_string(), "B".to_string()]
        );
    }

    #[test]
    fn a_broken_value_surfaces_as_a_query_error_with_the_same_text() {
        let (_dir, db) = init_temp_db();
        let expected = {
            let conn = db.writer.lock().unwrap();
            set_setting(&conn, WATCH_FOLDERS_KEY, "not json").unwrap();
            get_watch_folders(&conn).unwrap_err().to_string()
        };
        let err = read_watch_folders(&db.read_pool).unwrap_err();
        assert!(matches!(err, WatchFoldersReadError::Query(_)));
        assert_eq!(err.to_string(), expected);
    }
}

#[cfg(test)]
mod boolean_flag_tests {
    use super::*;
    use crate::db::test_support::init_temp_db;

    #[test]
    fn update_check_on_startup_defaults_to_on_and_round_trips() {
        let (_d, db) = init_temp_db();
        let conn = db.writer.lock().unwrap();
        assert!(get_update_check_on_startup(&conn).unwrap());
        set_update_check_on_startup(&conn, false).unwrap();
        assert!(!get_update_check_on_startup(&conn).unwrap());
        set_update_check_on_startup(&conn, true).unwrap();
        assert!(get_update_check_on_startup(&conn).unwrap());
    }

    #[test]
    fn an_unrecognized_stored_update_check_value_reads_as_on() {
        let (_d, db) = init_temp_db();
        let conn = db.writer.lock().unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('update_check_on_startup', 'garbage')",
            [],
        )
        .unwrap();
        assert!(get_update_check_on_startup(&conn).unwrap());
    }

    #[test]
    fn restore_view_state_is_on_by_default_and_an_unrecognized_value_reads_as_on() {
        let (_d, db) = init_temp_db();
        let conn = db.writer.lock().unwrap();
        assert!(get_restore_view_state_enabled(&conn).unwrap());
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('restore_view_state_enabled', 'garbage')",
            [],
        )
        .unwrap();
        assert!(get_restore_view_state_enabled(&conn).unwrap());
    }
}
