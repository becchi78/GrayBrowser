//! CRUD against the `videos` / `skipped_files` / `settings` / `tags` /
//! `video_tags` tables. Writes (`insert_video`, `upsert_skipped_file`,
//! `set_watch_folders`) take the single writer `Connection`; reads
//! (`list_videos`, `list_skipped_files`) take a pooled read connection
//! (single-writer/many-readers rule).
//!
//! `video_tags` referential integrity: since
//! `PRAGMA foreign_keys` is deliberately OFF (see `db::disable_foreign_keys`),
//! nothing in SQLite itself stops an orphan `video_tags` row. The multi-
//! statement functions in `tags` and `folders` (`assign_tag_to_video`,
//! `delete_tag`, `delete_videos_cascade`) take `&mut Connection` specifically so they can
//! open a real `conn.transaction()` and enforce that integrity in the
//! application layer, mirroring `db::migrations::run_migrations`'s
//! transaction pattern: multiple `tx.execute()`/`tx.query_row()` calls, then
//! one `tx.commit()`, with an automatic rollback on any earlier `Err` (the
//! `Transaction` is simply dropped uncommitted).

mod attempts;
mod duplicates;
mod folders;
mod offline;
mod settings;
mod skipped;
mod tags;
mod unmatched;
mod videos;
mod wb_import;

pub use attempts::*;
pub use duplicates::*;
pub use folders::*;
pub use offline::*;
pub use settings::*;
pub use skipped::*;
pub use tags::*;
pub use unmatched::*;
pub use videos::*;
pub use wb_import::*;
