-- Persists the route-X UNIQUE collision the path-follow rewrite
-- (update_video_path_and_status) can hit when the new path is already
-- occupied by a different online video. The two files should be treated as
-- duplicate candidates -- this migration adds a table to make that
-- persistence concrete rather than losing the collision the moment it's
-- detected. (The same table is also reused, unmodified, for route-Y's
-- coincidental-rehash-match collisions -- see src/scan/mod.rs's
-- `reconcile_known_path`.)
--
-- video_id: the offline-side row that attempted (and failed) to follow the
-- path. colliding_video_id: the online-side row that already owns
-- attempted_path. Same REFERENCES-as-documentation style as video_tags
-- (0001_initial.sql) -- PRAGMA foreign_keys stays OFF, so referential
-- integrity for this table is enforced in the application layer
-- (queries::delete_video_cascade), not by SQLite itself.
--
-- UNIQUE(video_id, colliding_video_id) backs the same "re-detecting the same
-- collision only refreshes detected_at" upsert pattern skipped_files already
-- uses (0001_initial.sql / queries::upsert_skipped_file), so a repeat
-- scan/poll hitting the same collision does not pile up duplicate rows.
CREATE TABLE IF NOT EXISTS path_collisions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  video_id TEXT NOT NULL REFERENCES videos(id),
  colliding_video_id TEXT NOT NULL REFERENCES videos(id),
  attempted_path TEXT NOT NULL,
  detected_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  UNIQUE(video_id, colliding_video_id)
);

CREATE INDEX IF NOT EXISTS idx_path_collisions_video ON path_collisions(video_id);
CREATE INDEX IF NOT EXISTS idx_path_collisions_colliding ON path_collisions(colliding_video_id);
