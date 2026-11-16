-- Drops three indexes the query planner has no use for, so every insert and
-- update stops maintaining them. Each is covered by an index SQLite already
-- keeps, or is not used by any query:
--
-- * idx_videos_path duplicates the automatic index behind `file_path TEXT
--   UNIQUE`. Equality lookups and covering scans of file_path use that index
--   instead. `file_path LIKE ... ESCAPE` cannot be answered by either one:
--   LIKE is case-insensitive while both indexes use BINARY collation.
-- * idx_videos_full_hash: no query filters on full_hash. Duplicate groups are
--   built in Rust from rows already read by quick_hash/size.
-- * idx_path_collisions_video duplicates the leading column of the automatic
--   index behind UNIQUE(video_id, colliding_video_id).
--
-- idx_videos_status and idx_videos_name stay: the planner picks them for the
-- online/offline listings and for the file-name sort respectively.
DROP INDEX IF EXISTS idx_videos_path;
DROP INDEX IF EXISTS idx_videos_full_hash;
DROP INDEX IF EXISTS idx_path_collisions_video;
