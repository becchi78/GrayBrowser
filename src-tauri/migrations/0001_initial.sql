-- GrayBrowser initial schema: 6 tables + 6 indexes.
--
-- video_tags declares REFERENCES to videos(id)/tags(id) per the schema's
-- prose ("外部キー"), but PRAGMA foreign_keys=ON is deliberately NOT set
-- anywhere in this codebase -- referential integrity for this table is
-- enforced in the application layer instead of by SQLite itself.
--
-- All statements use IF NOT EXISTS: schema_version itself is bootstrapped
-- separately before this migration runs (so its own CREATE TABLE here must
-- not fail), and the other tables/indexes follow the same style defensively
-- since the whole batch executes inside a single transaction.

CREATE TABLE IF NOT EXISTS videos (
  id TEXT PRIMARY KEY,
  file_path TEXT UNIQUE NOT NULL,
  file_name TEXT NOT NULL,
  file_size INTEGER NOT NULL,
  duration INTEGER,
  quick_hash TEXT NOT NULL,
  full_hash TEXT,
  status TEXT NOT NULL,
  rating INTEGER DEFAULT 0,
  created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS tags (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT UNIQUE NOT NULL
);

CREATE TABLE IF NOT EXISTS video_tags (
  video_id TEXT NOT NULL REFERENCES videos(id),
  tag_id INTEGER NOT NULL REFERENCES tags(id),
  PRIMARY KEY (video_id, tag_id)
);

CREATE TABLE IF NOT EXISTS schema_version (
  version INTEGER PRIMARY KEY,
  applied_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS skipped_files (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  file_path TEXT UNIQUE NOT NULL,
  file_name TEXT NOT NULL,
  reason TEXT NOT NULL,
  detected_char TEXT,
  detected_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL,
  updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_videos_path ON videos(file_path);
CREATE INDEX IF NOT EXISTS idx_videos_name ON videos(file_name);
CREATE INDEX IF NOT EXISTS idx_videos_status ON videos(status);
CREATE INDEX IF NOT EXISTS idx_videos_quick_hash ON videos(quick_hash);
CREATE INDEX IF NOT EXISTS idx_videos_full_hash ON videos(full_hash);
CREATE INDEX IF NOT EXISTS idx_video_tags_tag ON video_tags(tag_id);
