-- Legacy thumbnail files that a .wb import could not tie to a video, kept so
-- the user can link them by hand later. One row per file; a re-import of the
-- same folder rebuilds that folder's rows. thum_layout is the original
-- `thum_size` text (NULL when the .wb did not state one unambiguously), kept
-- so a manual link cuts the image into frames exactly as an automatic one.
-- No reference to videos: the file is not (yet) anyone's thumbnail.
CREATE TABLE IF NOT EXISTS wb_unmatched_thumbnails (
  folder_path TEXT NOT NULL,
  file_name   TEXT NOT NULL,
  kind        TEXT NOT NULL,
  thum_layout TEXT,
  PRIMARY KEY (folder_path, file_name)
);
