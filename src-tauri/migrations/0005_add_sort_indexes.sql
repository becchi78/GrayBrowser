-- Indexes for the existing/new sort orders.
-- list_videos has ordered by created_at DESC with no supporting
-- index until now; rating backs the new rating-sort. Both verified via
-- EXPLAIN QUERY PLAN against a 2000-row synthetic dataset to actually avoid a
-- temp-b-tree sort (see src-tauri/tests/sort_index_usage.rs).
--
-- A third candidate, idx_videos_mtime (for the "更新日" sort, which maps to
-- filesystem mtime rather than a new updated_at column), was tested and
-- deliberately NOT added here: the required NULL-last ordering (`ORDER BY
-- mtime IS NULL, mtime DESC`, since unreconciled rows leave mtime NULL) is a
-- compound expression the planner does not use a plain column index for --
-- it still falls back to a full scan + temp b-tree sort with or without the
-- index present, so adding it would be dead weight.
CREATE INDEX IF NOT EXISTS idx_videos_created_at ON videos(created_at);
CREATE INDEX IF NOT EXISTS idx_videos_rating ON videos(rating);
