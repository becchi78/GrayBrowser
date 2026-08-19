//! Writer for the committed fixture (`tests/fixtures/wb/sample_small.wb`).
//! Only ever creates a brand-new SQLite file at a caller-supplied output
//! path -- never opens or touches the real input `.wb`.
//!
//! The written `movie` table intentionally has a *reduced* schema: only the
//! 11 columns `wb-anonymize-tool::reader::read_movies` (and, in
//! `src-tauri`, `RealWbSourceAdapter::read_movies`) actually select, not the
//! real data's full 30-column schema. Nothing in the codebase reads the
//! other columns (title/artist/comment1-3/etc., which are 0-filled in the
//! real data anyway), so cloning them would only add
//! unused surface area to a fixture whose whole point is to be minimal.

use std::path::Path;

use gb_core::ports::wb_source::WbMovieRow;
use rusqlite::{params, Connection};

pub fn create_output(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute(
        "CREATE TABLE movie (
            movie_id INTEGER PRIMARY KEY,
            movie_name TEXT NOT NULL DEFAULT '',
            movie_path TEXT NOT NULL DEFAULT '',
            tag TEXT NOT NULL DEFAULT '',
            score INTEGER NOT NULL DEFAULT 0,
            hash TEXT NOT NULL DEFAULT '',
            kana TEXT NOT NULL DEFAULT '',
            roma TEXT NOT NULL DEFAULT '',
            file_date datetime,
            regist_date datetime,
            last_date datetime
        )",
        [],
    )?;
    Ok(conn)
}

pub fn write_movies(conn: &Connection, rows: &[WbMovieRow]) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO movie (movie_id, movie_name, movie_path, tag, score, hash, kana, roma, \
         file_date, regist_date, last_date) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )?;
    for row in rows {
        stmt.execute(params![
            row.movie_id,
            row.movie_name,
            row.movie_path,
            row.tag,
            row.score,
            row.hash,
            row.kana,
            row.roma,
            row.file_date,
            row.regist_date,
            row.last_date,
        ])?;
    }
    Ok(())
}
