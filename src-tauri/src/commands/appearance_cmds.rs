//! Theme and color-mode persistence commands. The `#[tauri::command]` wrappers
//! are thin; the logic lives in plain functions over `&Db` so it can be tested
//! without a Tauri runtime.

use gb_core::appearance::{Mode, Theme};
use serde::Serialize;
use tauri::State;

use crate::db::{queries, Db};

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct AppearanceDto {
    pub theme: String,
    pub mode: String,
}

fn load(db: &Db) -> anyhow::Result<AppearanceDto> {
    let conn = db.read_pool.get()?;
    Ok(AppearanceDto {
        theme: queries::get_theme(&conn)?.as_str().to_string(),
        mode: queries::get_appearance_mode(&conn)?.as_str().to_string(),
    })
}

/// An unknown value is rejected rather than stored: the stored string is what
/// the frontend writes into `data-theme`, so only known values may reach it.
fn save_theme(db: &Db, theme: &str) -> Result<(), String> {
    let theme = Theme::parse(theme).ok_or_else(|| format!("未対応のテーマです: \"{theme}\""))?;
    let conn = db.writer.lock().unwrap();
    queries::set_theme(&conn, theme).map_err(|e| e.to_string())
}

fn save_mode(db: &Db, mode: &str) -> Result<(), String> {
    let mode = Mode::parse(mode).ok_or_else(|| format!("未対応の外観です: \"{mode}\""))?;
    let conn = db.writer.lock().unwrap();
    queries::set_appearance_mode(&conn, mode).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_appearance(db: State<Db>) -> Result<AppearanceDto, String> {
    load(&db).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_theme(db: State<Db>, theme: String) -> Result<(), String> {
    save_theme(&db, &theme)
}

#[tauri::command]
pub fn set_appearance_mode(db: State<Db>, mode: String) -> Result<(), String> {
    save_mode(&db, &mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::init_temp_db;

    #[test]
    fn a_fresh_database_reports_the_defaults() {
        let (_d, db) = init_temp_db();
        assert_eq!(
            load(&db).unwrap(),
            AppearanceDto {
                theme: "graphite".into(),
                mode: "system".into()
            }
        );
    }

    #[test]
    fn saved_values_are_read_back() {
        let (_d, db) = init_temp_db();
        save_theme(&db, "fluent").unwrap();
        save_mode(&db, "dark").unwrap();
        assert_eq!(
            load(&db).unwrap(),
            AppearanceDto {
                theme: "fluent".into(),
                mode: "dark".into()
            }
        );
    }

    #[test]
    fn an_unknown_value_is_rejected_and_the_stored_one_stays() {
        let (_d, db) = init_temp_db();
        save_theme(&db, "cinema").unwrap();
        save_mode(&db, "light").unwrap();

        assert!(save_theme(&db, "Cinema").is_err());
        assert!(save_theme(&db, "").is_err());
        assert!(save_mode(&db, "auto").is_err());

        let loaded = load(&db).unwrap();
        assert_eq!(loaded.theme, "cinema");
        assert_eq!(loaded.mode, "light");
    }
}
