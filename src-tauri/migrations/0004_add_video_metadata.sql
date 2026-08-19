-- ffprobe-derived metadata, background-filled by the
-- metadata worker (src-tauri/src/metadata/). probed_at IS NULL marks "not yet
-- probed" -- same stateless-resume philosophy as the thumbnail worker:
-- re-scan for rows missing it, no persisted queue table.
ALTER TABLE videos ADD COLUMN width INTEGER;
ALTER TABLE videos ADD COLUMN height INTEGER;
ALTER TABLE videos ADD COLUMN video_codec TEXT;
ALTER TABLE videos ADD COLUMN audio_codec TEXT;
ALTER TABLE videos ADD COLUMN bitrate INTEGER;
ALTER TABLE videos ADD COLUMN fps REAL;
ALTER TABLE videos ADD COLUMN probed_at DATETIME;
