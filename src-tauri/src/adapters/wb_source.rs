//! Real `WbSourceAdapter`: opens a legacy `.wb` (WhiteBrowser SQLite)
//! database read-only via `rusqlite`. Always opened with
//! `SQLITE_OPEN_READ_ONLY` -- a bug elsewhere in the importer must not be
//! able to write to a user's real personal-data file.

use std::path::Path;
use std::sync::Mutex;

use gb_core::ports::wb_source::{WbMovieRow, WbSourceAdapter, WbSourceError, WbTextCell};
use rusqlite::{Connection, OpenFlags};

/// `Connection` isn't `Sync` on its own (it caches prepared statements
/// internally), so it's wrapped the same way `db::Db`'s writer connection is
/// (see `db/mod.rs`) to satisfy `WbSourceAdapter: Send + Sync`.
pub struct RealWbSourceAdapter {
    conn: Mutex<Connection>,
}

impl RealWbSourceAdapter {
    pub fn open(wb_path: &Path) -> Result<Self, WbSourceError> {
        let conn = Connection::open_with_flags(wb_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| WbSourceError::Open(e.to_string()))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn text_bearing_table_names(&self) -> Result<Vec<String>, WbSourceError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .map_err(query_err)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(query_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(query_err)?;
        Ok(rows
            .into_iter()
            .filter(|name| is_safe_identifier(name))
            .collect())
    }

    /// Column names whose declared SQLite type contains `TEXT`, for
    /// `table_name`. `PRAGMA table_info` cannot take a bound parameter, so
    /// `table_name` is interpolated directly -- safe here because callers
    /// only pass names already filtered by `is_safe_identifier`.
    fn text_column_names(&self, table_name: &str) -> Result<Vec<String>, WbSourceError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table_name})"))
            .map_err(query_err)?;
        let rows = stmt
            .query_map([], |row| {
                let name: String = row.get(1)?;
                let col_type: String = row.get(2)?;
                Ok((name, col_type))
            })
            .map_err(query_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(query_err)?;
        Ok(rows
            .into_iter()
            .filter(|(name, col_type)| {
                col_type.to_uppercase().contains("TEXT") && is_safe_identifier(name)
            })
            .map(|(name, _)| name)
            .collect())
    }

    fn read_text_column(
        &self,
        table_name: &str,
        column_name: &str,
    ) -> Result<Vec<WbTextCell>, WbSourceError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(&format!(
                "SELECT rowid, {column_name} FROM {table_name} \
                 WHERE {column_name} IS NOT NULL AND {column_name} != ''"
            ))
            .map_err(query_err)?;
        let cells = stmt
            .query_map([], |row| {
                Ok(WbTextCell {
                    table_name: table_name.to_string(),
                    column_name: column_name.to_string(),
                    row_id: row.get(0)?,
                    value: row.get(1)?,
                })
            })
            .map_err(query_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(query_err)?;
        Ok(cells)
    }
}

impl WbSourceAdapter for RealWbSourceAdapter {
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

    /// Scans every table's text-typed columns, not just the known
    /// personal-info-bearing ones, so the anonymization tool's coverage
    /// stays correct even if a future `.wb` schema version adds
    /// tables/columns.
    fn read_all_text_cells(&self) -> Result<Vec<WbTextCell>, WbSourceError> {
        let mut cells = Vec::new();
        for table_name in self.text_bearing_table_names()? {
            for column_name in self.text_column_names(&table_name)? {
                cells.extend(self.read_text_column(&table_name, &column_name)?);
            }
        }
        Ok(cells)
    }

    /// Reads only the `value` column of the matching rows, as text. `profile`
    /// holds some binary values that are not valid text; reading whole rows
    /// (or those values) would fail, so a row whose value is not text is
    /// skipped instead.
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

/// Guards the dynamic SQL built in this file against a malicious `.wb` file
/// declaring a table/column name containing SQL syntax. `sqlite_master` and
/// `PRAGMA table_info` output is normally safe, but this keeps the
/// interpolation defensive regardless.
fn is_safe_identifier(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn query_err(e: rusqlite::Error) -> WbSourceError {
    WbSourceError::Query(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter_with_profile(
        rows: &[(&str, &str, rusqlite::types::Value)],
    ) -> (tempfile::TempDir, RealWbSourceAdapter) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.wb");
        let conn = Connection::open(&path).unwrap();
        conn.execute("CREATE TABLE profile (skin TEXT, key TEXT, value TEXT)", [])
            .unwrap();
        for (skin, key, value) in rows {
            conn.execute(
                "INSERT INTO profile (skin, key, value) VALUES (?1, ?2, ?3)",
                rusqlite::params![skin, key, value],
            )
            .unwrap();
        }
        drop(conn);
        let adapter = RealWbSourceAdapter::open(&path).unwrap();
        (dir, adapter)
    }

    #[test]
    fn profile_value_returns_every_skins_text_and_skips_non_text_rows() {
        use rusqlite::types::Value;
        let (_dir, adapter) = adapter_with_profile(&[
            ("a", "thum_size", Value::Text("200x150x6x1".into())),
            ("b", "thum_size", Value::Text("100x75x6x1".into())),
            ("a", "thum_size", Value::Blob(vec![0xff, 0xfe, 0x00])),
            ("a", "other", Value::Text("x".into())),
        ]);
        let mut values = adapter.read_profile_value("thum_size").unwrap();
        values.sort();
        assert_eq!(values, ["100x75x6x1", "200x150x6x1"]);
    }

    #[test]
    fn profile_value_is_empty_when_the_key_is_absent() {
        let (_dir, adapter) = adapter_with_profile(&[]);
        assert!(adapter.read_profile_value("thum_size").unwrap().is_empty());
    }

    #[test]
    fn profile_value_errors_when_there_is_no_profile_table() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.wb");
        drop(Connection::open(&path).unwrap());
        let adapter = RealWbSourceAdapter::open(&path).unwrap();
        assert!(adapter.read_profile_value("thum_size").is_err());
    }
}
