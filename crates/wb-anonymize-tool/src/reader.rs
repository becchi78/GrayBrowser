//! Standalone read-only `.wb` reader for this tool. Deliberately does not
//! depend on `graybrowser_lib` (which would pull in `tauri`) -- this is a
//! small dev CLI, not part of the shipped app. It implements the same
//! `WbSourceAdapter` port as `src-tauri`'s `RealWbSourceAdapter`, so both
//! read the real schema through one contract; the full-table text scan the
//! anonymization coverage check needs lives here only, since the importer
//! never uses it.

use std::path::Path;
use std::sync::Mutex;

use gb_core::ports::wb_source::{WbMovieRow, WbSourceAdapter, WbSourceError};
use rusqlite::{Connection, OpenFlags};

/// One text-typed cell of the real `.wb`, addressed by table/column/rowid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WbTextCell {
    pub table_name: String,
    pub column_name: String,
    pub row_id: i64,
    pub value: String,
}

/// `Connection` isn't `Sync` on its own, so it is wrapped in a `Mutex` to
/// satisfy `WbSourceAdapter: Send + Sync`.
pub struct ReadOnlyWb {
    conn: Mutex<Connection>,
}

impl ReadOnlyWb {
    /// Always opened `SQLITE_OPEN_READ_ONLY`: the real file holds personal
    /// data and must never be written.
    pub fn open(path: &Path) -> Result<Self, WbSourceError> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| WbSourceError::Open(e.to_string()))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Enumerates every table's TEXT-typed columns dynamically (via
    /// `sqlite_master` + `PRAGMA table_info`, no hardcoded table list) and
    /// reads every non-empty cell, so coverage stays correct even if a
    /// future `.wb` schema version adds tables/columns.
    pub fn read_all_text_cells(&self) -> Result<Vec<WbTextCell>, WbSourceError> {
        let conn = self.conn.lock().unwrap();
        let mut cells = Vec::new();
        for table_name in text_bearing_table_names(&conn).map_err(query_err)? {
            for column_name in text_column_names(&conn, &table_name).map_err(query_err)? {
                cells
                    .extend(read_text_column(&conn, &table_name, &column_name).map_err(query_err)?);
            }
        }
        Ok(cells)
    }
}

impl WbSourceAdapter for ReadOnlyWb {
    fn read_movies(&self) -> Result<Vec<WbMovieRow>, WbSourceError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT movie_id, movie_name, movie_path, tag, score, hash, kana, roma, \
                 file_date, regist_date, last_date FROM movie",
            )
            .map_err(query_err)?;
        let rows = stmt
            .query_map([], |row| {
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
            })
            .map_err(query_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(query_err)?;
        Ok(rows)
    }

    /// Same row-skipping rule as the shipped adapter: `profile` also holds
    /// binary values, so a row whose value is not text is skipped.
    fn read_profile_value(&self, key: &str) -> Result<Vec<String>, WbSourceError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT value FROM profile WHERE key = ?1")
            .map_err(query_err)?;
        let mut rows = stmt.query([key]).map_err(query_err)?;
        let mut values = Vec::new();
        while let Some(row) = rows.next().map_err(query_err)? {
            if let Ok(value) = row.get::<_, String>(0) {
                values.push(value);
            }
        }
        Ok(values)
    }
}

fn query_err(e: rusqlite::Error) -> WbSourceError {
    WbSourceError::Query(e.to_string())
}

fn text_bearing_table_names(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")?;
    let rows: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<_, _>>()?;
    Ok(rows.into_iter().filter(|n| is_safe_identifier(n)).collect())
}

/// `PRAGMA table_info` cannot take a bound parameter, so `table_name` is
/// interpolated directly -- safe because callers only pass names already
/// filtered by `is_safe_identifier`.
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

/// Guards the dynamic SQL above against a malicious `.wb` declaring a
/// table/column name containing SQL syntax.
fn is_safe_identifier(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wb_with(setup: &[&str]) -> (tempfile::TempDir, ReadOnlyWb) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.wb");
        let conn = Connection::open(&path).unwrap();
        for sql in setup {
            conn.execute(sql, []).unwrap();
        }
        drop(conn);
        let wb = ReadOnlyWb::open(&path).unwrap();
        (dir, wb)
    }

    fn cell(table: &str, column: &str, row_id: i64, value: &str) -> WbTextCell {
        WbTextCell {
            table_name: table.to_string(),
            column_name: column.to_string(),
            row_id,
            value: value.to_string(),
        }
    }

    #[test]
    fn text_cells_cover_every_table_and_skip_non_text_empty_and_null() {
        let (_dir, wb) = wb_with(&[
            "CREATE TABLE movie (movie_id INTEGER PRIMARY KEY, movie_name TEXT, score INTEGER, file_date datetime)",
            "INSERT INTO movie VALUES (1, 'a.mp4', 3, '2011-05-04 12:00:00')",
            "INSERT INTO movie VALUES (2, '', 4, '2011-05-04 12:00:00')",
            "INSERT INTO movie VALUES (3, NULL, 5, NULL)",
            "CREATE TABLE findfact (find_text TEXT NOT NULL DEFAULT '')",
            "INSERT INTO findfact VALUES ('term')",
            "CREATE TABLE \"weird-table\" (x TEXT)",
            "INSERT INTO \"weird-table\" VALUES ('must not be read')",
        ]);
        let mut cells = wb.read_all_text_cells().unwrap();
        cells.sort_by(|a, b| (&a.table_name, a.row_id).cmp(&(&b.table_name, b.row_id)));
        assert_eq!(
            cells,
            [
                cell("findfact", "find_text", 1, "term"),
                cell("movie", "movie_name", 1, "a.mp4"),
            ]
        );
    }

    #[test]
    fn read_movies_maps_every_column() {
        let (_dir, wb) = wb_with(&[
            "CREATE TABLE movie (movie_id INTEGER PRIMARY KEY, movie_name TEXT, movie_path TEXT, tag TEXT, score INTEGER, hash TEXT, kana TEXT, roma TEXT, file_date datetime, regist_date datetime, last_date datetime)",
            "INSERT INTO movie VALUES (7, 'n', 'p', 't', 2, 'h', 'k', 'r', 'fd', 'rd', 'ld')",
        ]);
        assert_eq!(
            wb.read_movies().unwrap(),
            [WbMovieRow {
                movie_id: 7,
                movie_name: "n".into(),
                movie_path: "p".into(),
                tag: "t".into(),
                score: 2,
                hash: "h".into(),
                kana: "k".into(),
                roma: "r".into(),
                file_date: "fd".into(),
                regist_date: "rd".into(),
                last_date: "ld".into(),
            }]
        );
    }

    #[test]
    fn profile_value_returns_matching_text_rows_only() {
        let (_dir, wb) = wb_with(&[
            "CREATE TABLE profile (skin TEXT, key TEXT, value TEXT)",
            "INSERT INTO profile VALUES ('a', 'thum_size', '200x150x6x1')",
            "INSERT INTO profile VALUES ('a', 'other', 'x')",
            "INSERT INTO profile VALUES ('b', 'thum_size', X'fffe00')",
        ]);
        assert_eq!(wb.read_profile_value("thum_size").unwrap(), ["200x150x6x1"]);
    }

    #[test]
    fn open_fails_for_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            ReadOnlyWb::open(&dir.path().join("absent.wb")),
            Err(WbSourceError::Open(_))
        ));
    }
}
