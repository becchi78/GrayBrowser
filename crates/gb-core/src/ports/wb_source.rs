//! `.wb` (legacy WhiteBrowser SQLite database) read port. The trait is
//! SQLite-independent; the real implementation (in
//! `src-tauri::adapters::wb_source`) opens the file with `rusqlite` in
//! read-only mode.

pub trait WbSourceAdapter: Send + Sync {
    /// Reads every row of the `movie` table. `view_count` is intentionally
    /// not part of `WbMovieRow` -- requirements exclude it from migration.
    fn read_movies(&self) -> Result<Vec<WbMovieRow>, WbSourceError>;

    /// Reads every text-column cell across all tables, for the
    /// anonymization tool's coverage scan. Not used by the importer itself.
    fn read_all_text_cells(&self) -> Result<Vec<WbTextCell>, WbSourceError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WbMovieRow {
    pub movie_id: i64,
    pub movie_name: String,
    pub movie_path: String,
    pub tag: String,
    pub score: i64,
    pub hash: String,
    pub kana: String,
    pub roma: String,
    pub file_date: String,
    pub regist_date: String,
    pub last_date: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WbTextCell {
    pub table_name: String,
    pub column_name: String,
    pub row_id: i64,
    pub value: String,
}

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum WbSourceError {
    #[error("failed to open .wb database: {0}")]
    Open(String),
    #[error("query failed: {0}")]
    Query(String),
}
