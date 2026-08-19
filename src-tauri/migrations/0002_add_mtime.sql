-- Adds videos.mtime, the cheap change-detection
-- signal used alongside file_size to decide whether a rescanned/repolled file
-- needs its quick_hash recomputed. Nullable: rows inserted before this column
-- existed (or before the scan pipeline started plumbing mtime through) simply
-- have NULL here, and gb_core::reconcile::classify_discovered_file treats a
-- missing known mtime as "must rehash", never as "unchanged" -- a NULL can
-- never silently cause a real change to be skipped.
ALTER TABLE videos ADD COLUMN mtime INTEGER;
