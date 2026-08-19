//! Standalone read-only `.wb` reader for this tool. Deliberately does not
//! depend on `graybrowser_lib` (which would pull in `tauri`) -- this is a
//! small dev CLI, not part of the shipped app. Mirrors the query shapes of
//! `src-tauri/src/adapters/wb_source.rs`'s `RealWbSourceAdapter` (both read
//! the same real schema), but is self-contained.

use std::path::Path;

use gb_core::ports::wb_source::{WbMovieRow, WbTextCell};
use rusqlite::{Connection, OpenFlags};

pub fn open_readonly(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
}

pub fn read_movies(conn: &Connection) -> rusqlite::Result<Vec<WbMovieRow>> {
    let mut stmt = conn.prepare(
        "SELECT movie_id, movie_name, movie_path, tag, score, hash, kana, roma, \
         file_date, regist_date, last_date FROM movie",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(WbMovieRow {
            movie_id: row.get(0)?,
            movie_name: row.get(1)?,
            movie_path: row.get(2)?,
            tag: row.get(3)?,
            score: row.get(4)?,
            hash: row.get(5)?,
            kana: row.get(6)?,
            roma: row.get(7)?,
            file_date: row.get(8)?,
            regist_date: row.get(9)?,
            last_date: row.get(10)?,
        })
    })?;
    rows.collect()
}

/// Enumerates every table's TEXT-typed columns dynamically (via
/// `sqlite_master` + `PRAGMA table_info`, no hardcoded table list) and
/// reads every non-empty cell.
pub fn read_all_text_cells(conn: &Connection) -> rusqlite::Result<Vec<WbTextCell>> {
    let mut cells = Vec::new();
    for table_name in text_bearing_table_names(conn)? {
        for column_name in text_column_names(conn, &table_name)? {
            cells.extend(read_text_column(conn, &table_name, &column_name)?);
        }
    }
    Ok(cells)
}

fn text_bearing_table_names(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")?;
    let rows: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<_, _>>()?;
    Ok(rows.into_iter().filter(|n| is_safe_identifier(n)).collect())
}

fn text_column_names(conn: &Connection, table_name: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table_name})"))?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(1)?, row.get(2)?)))?
        .collect::<Result<_, _>>()?;
    Ok(rows
        .into_iter()
        .filter(|(name, col_type)| {
            col_type.to_uppercase().contains("TEXT") && is_safe_identifier(name)
        })
        .map(|(name, _)| name)
        .collect())
}

fn read_text_column(
    conn: &Connection,
    table_name: &str,
    column_name: &str,
) -> rusqlite::Result<Vec<WbTextCell>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT rowid, {column_name} FROM {table_name} WHERE {column_name} IS NOT NULL AND {column_name} != ''"
    ))?;
    let cells = stmt
        .query_map([], |row| {
            Ok(WbTextCell {
                table_name: table_name.to_string(),
                column_name: column_name.to_string(),
                row_id: row.get(0)?,
                value: row.get(1)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(cells)
}

fn is_safe_identifier(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}
