-- Lets the user remove an unregistered-file row whose file no longer exists.
-- The row is hidden rather than deleted: a delete would be undone the next
-- time a .wb import records the same path again, whereas a hidden row keeps
-- `file_path` UNIQUE-occupied so that import's upsert only refreshes
-- `detected_at` and leaves `dismissed` alone.
--
-- `DEFAULT 0` keeps every pre-existing row visible. A regular scan that finds
-- the file on disk again sets it back to 0, so a file that exists is never
-- left hidden.
ALTER TABLE skipped_files ADD COLUMN dismissed INTEGER NOT NULL DEFAULT 0;

-- The rating, tags and readings of the .wb row a record came from, as JSON.
-- Kept so a record whose file is gone can still be registered as an offline
-- video with them. NULL for records found by a regular scan.
ALTER TABLE skipped_files ADD COLUMN wb_payload TEXT;
