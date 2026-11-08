//! Helpers shared by more than one topic: the import harness, row
//! builders, fake adapters and the readers of what an import left behind.

use super::*;

/// Import without registering watch folders, with every other port faked
/// out -- the behavior most tests exercise.
///
/// It shares its name with the production `import_all`, and which one a
/// bare `import_all(..)` reaches depends on the topic file's imports: a file
/// that names it in `use super::common::{import_all, ..}` gets this wrapper,
/// because an explicit import beats the `use super::*` glob; a file that
/// does not gets the production one. To call the production one alongside
/// this wrapper, write `super::import_all(..)`. Mixing them up fails to
/// compile, since the two take different arguments.
pub(super) fn import_all<S, F, N, C>(
    db: &Db,
    paths: &WbImportPaths,
    wb_source: &S,
    ffmpeg: &F,
    notifier: &N,
    catalog_notifier: &C,
) where
    S: WbSourceAdapter,
    F: FfmpegAdapter,
    N: WbImportNotifier,
    C: CatalogNotifier,
{
    super::import_all(
        db,
        paths,
        wb_source,
        ffmpeg,
        notifier,
        catalog_notifier,
        None::<&FakeWatchReconfigurer>,
        &FakeDirLister::default(),
        &FakeFolderProbe::default(),
        &FakeVolumeLabelResolver::default(),
        &FakeWbImportClock::default(),
        &FakeWbImportLog::default(),
    );
}

/// The subdirectory `link_thumbnails`/`video_thumbnail_dir` resolve
/// every video in these tests to, since none of them register any
/// `watch_folders`.
pub(super) fn unassigned_dir(thumbnails_root: &Path) -> PathBuf {
    thumbnails_root.join(gb_core::paths::THUMBNAIL_UNASSIGNED_SUBDIR)
}

pub(super) fn row(overrides: impl FnOnce(&mut WbMovieRow)) -> WbMovieRow {
    let mut row = WbMovieRow {
        movie_id: 1,
        movie_name: "movie.mp4".to_string(),
        movie_path: "T:\\videos\\movie.mp4".to_string(),
        tag: String::new(),
        score: 0,
        hash: String::new(),
        kana: String::new(),
        roma: String::new(),
        file_date: "2011-05-04 12:00:00".to_string(),
        regist_date: "2011-05-04 12:00:00".to_string(),
        last_date: "2011-05-04 12:00:00".to_string(),
    };
    overrides(&mut row);
    row
}

pub(super) fn no_op_ffmpeg() -> FakeFfmpegAdapter {
    FakeFfmpegAdapter {
        convert_result: Box::new(|_src| Ok(())),
        ..Default::default()
    }
}

pub(super) struct Harness {
    _db_dir: tempfile::TempDir,
    pub(super) db: Db,
    _thumb_folder_dir: tempfile::TempDir,
    _thumbs_out_dir: tempfile::TempDir,
    pub(super) paths: WbImportPaths,
}

pub(super) fn harness() -> Harness {
    let (db_dir, db) = init_temp_db();
    let thumb_folder_dir = tempfile::tempdir().unwrap();
    let thumbs_out_dir = tempfile::tempdir().unwrap();
    let paths = WbImportPaths {
        wb_path: PathBuf::from("test.wb"),
        thumbnail_folder: thumb_folder_dir.path().to_path_buf(),
        thumbnails_dir: thumbs_out_dir.path().to_path_buf(),
    };
    Harness {
        _db_dir: db_dir,
        db,
        _thumb_folder_dir: thumb_folder_dir,
        _thumbs_out_dir: thumbs_out_dir,
        paths,
    }
}

pub(super) fn recorded_unmatched(h: &Harness) -> Vec<queries::UnmatchedThumbnailRow> {
    let conn = h.db.writer.lock().unwrap();
    queries::list_unmatched_thumbnails(&conn).unwrap()
}

/// A real directory holding one real (online) video file.
pub(super) struct OnlineDir {
    pub(super) dir: tempfile::TempDir,
}

impl OnlineDir {
    pub(super) fn new(file_name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(file_name), b"video bytes").unwrap();
        Self { dir }
    }

    pub(super) fn folder(&self) -> String {
        self.dir.path().to_string_lossy().to_string()
    }

    pub(super) fn file(&self, name: &str) -> String {
        self.dir.path().join(name).to_string_lossy().to_string()
    }
}

pub(super) fn import_registering(
    h: &Harness,
    rows: Vec<WbMovieRow>,
    reconfigurer: &FakeWatchReconfigurer,
) -> crate::events::WbImportSummary {
    let wb_source = FakeWbSourceAdapter {
        movies: Ok(rows),
        ..Default::default()
    };
    let notifier = FakeWbImportNotifier::default();
    super::import_all(
        &h.db,
        &h.paths,
        &wb_source,
        &no_op_ffmpeg(),
        &notifier,
        &FakeCatalogNotifier::default(),
        Some(reconfigurer),
        &FakeDirLister::default(),
        &FakeFolderProbe::default(),
        &FakeVolumeLabelResolver::default(),
        &FakeWbImportClock::default(),
        &FakeWbImportLog::default(),
    );
    let summary = notifier.complete_calls.lock().unwrap()[0].clone();
    summary
}

pub(super) fn stored_watch_folders(h: &Harness) -> Vec<String> {
    let conn = h.db.read_pool.get().unwrap();
    queries::get_watch_folders(&conn).unwrap()
}

pub(super) const DASH: &str = "\u{2014}";

pub(super) fn import_with_lister(
    h: &Harness,
    rows: Vec<WbMovieRow>,
    lister: &FakeDirLister,
    reconfigurer: Option<&FakeWatchReconfigurer>,
) -> crate::events::WbImportSummary {
    let notifier = FakeWbImportNotifier::default();
    super::import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies: Ok(rows),
            ..Default::default()
        },
        &no_op_ffmpeg(),
        &notifier,
        &FakeCatalogNotifier::default(),
        reconfigurer,
        lister,
        &FakeFolderProbe::default(),
        &FakeVolumeLabelResolver::default(),
        &FakeWbImportClock::default(),
        &FakeWbImportLog::default(),
    );
    let summary = notifier.complete_calls.lock().unwrap()[0].clone();
    summary
}

/// A row as the old WhiteBrowser stored it: the dash replaced by `?` in
/// both the path and the name.
pub(super) fn replaced_row(id: i64, folder: &str, stem: &str) -> WbMovieRow {
    row(|r| {
        r.movie_id = id;
        r.movie_name = format!("{stem}?.mp4");
        r.movie_path = format!("{folder}\\{stem}?.mp4");
    })
}

pub(super) fn image_size(w: u32, h: u32) -> Result<VideoMetadata, FfmpegError> {
    Ok(VideoMetadata {
        width: Some(w),
        height: Some(h),
        ..Default::default()
    })
}

/// An ffmpeg fake whose JPG probe returns `size`, and whose convert and
/// split calls succeed (split unless a rectangle has x `fail_crop_at_x`).
pub(super) fn split_ffmpeg(
    size: Result<VideoMetadata, FfmpegError>,
    fail_crop_at_x: Option<u32>,
) -> FakeFfmpegAdapter {
    FakeFfmpegAdapter {
        metadata_for: Some(Box::new(move |_| size.clone())),
        convert_result: Box::new(|_| Ok(())),
        split_result: Box::new(move |_, crops| {
            if crops.iter().any(|c| Some(c.x) == fail_crop_at_x) {
                Err(FfmpegError::NonZeroExit {
                    status: 1,
                    stderr: "simulated crop failure".into(),
                })
            } else {
                Ok(())
            }
        }),
        // A failing ffmpeg may already have created some outputs.
        split_partial_writes: if fail_crop_at_x.is_some() { 3 } else { 0 },
        ..Default::default()
    }
}

/// A row for `name` under `folder`, as a normal (non-`?`) stored path.
pub(super) fn named_row(id: i64, folder: &str, name: &str) -> WbMovieRow {
    row(|r| {
        r.movie_id = id;
        r.movie_name = name.to_string();
        r.movie_path = format!("{folder}\\{name}");
    })
}

pub(super) fn count_of(h: &Harness, sql: &str) -> i64 {
    h.db.writer
        .lock()
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}

/// Progress sent for one stage, in order.
pub(super) fn stage_progress(
    notifier: &FakeWbImportNotifier,
    stage: WbImportStage,
) -> Vec<(u32, Option<u32>)> {
    notifier
        .progress_calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(s, _, _)| *s == stage)
        .map(|&(_, processed, total)| (processed, total))
        .collect()
}

pub(super) fn registering_progress(notifier: &FakeWbImportNotifier) -> Vec<(u32, Option<u32>)> {
    stage_progress(notifier, WbImportStage::Registering)
}

/// Minimal JPEG bytes: SOI, a long EXIF-like block, then a frame header.
/// Enough for `jpeg_dimensions`; not decodable, which the fakes never need.
pub(super) fn jpeg_bytes(w: u16, h: u16) -> Vec<u8> {
    let mut v = vec![0xFF, 0xD8, 0xFF, 0xE1];
    v.extend_from_slice(&(10_002u16).to_be_bytes());
    v.extend(std::iter::repeat_n(0u8, 10_000));
    v.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8]);
    v.extend_from_slice(&h.to_be_bytes());
    v.extend_from_slice(&w.to_be_bytes());
    v.extend_from_slice(&[1, 1, 0x11, 0]);
    v
}

/// Writes one legacy JPG per video with `content(i)` as its bytes and
/// returns the matching `registered_movies` rows. File names are
/// zero-padded so they sort like the indexes.
pub(super) fn legacy_movies(
    h: &Harness,
    count: usize,
    content: impl Fn(usize) -> Vec<u8>,
) -> Vec<(String, String, String)> {
    (0..count)
        .map(|i| {
            let hash = format!("{i:08x}");
            std::fs::write(
                h.paths
                    .thumbnail_folder
                    .join(format!("m{i:04}.mp4.#{hash}.jpg")),
                content(i),
            )
            .unwrap();
            (format!("video-{i}"), hash, format!("Z:\\gone\\m{i}.mp4"))
        })
        .collect()
}

pub(super) fn six_by_one_layout() -> ThumbLayout {
    ThumbLayout {
        tile_w: 200,
        tile_h: 150,
        cols: 6,
        rows: 1,
    }
}

pub(super) fn link_with_layout(
    h: &Harness,
    movies: &[(String, String, String)],
    ffmpeg: &FakeFfmpegAdapter,
    layout: Option<&ThumbLayout>,
) -> LinkOutcome {
    link_thumbnails(
        &h.paths.thumbnail_folder,
        &h.paths.thumbnails_dir,
        &[],
        movies,
        &[],
        &HashSet::new(),
        &HashSet::new(),
        ffmpeg,
        layout,
        |_, _| {},
    )
}

pub(super) fn import_and_summarize(h: &Harness, rows: Vec<WbMovieRow>) -> WbImportSummary {
    let notifier = FakeWbImportNotifier::default();
    import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies: Ok(rows),
            ..Default::default()
        },
        &no_op_ffmpeg(),
        &notifier,
        &FakeCatalogNotifier::default(),
    );
    let summary = notifier.complete_calls.lock().unwrap()[0].clone();
    summary
}

/// Runs the import against `import_log` and returns the summary, if the
/// import got as far as producing one.
pub(super) fn import_with_log(
    h: &Harness,
    movies: Result<Vec<WbMovieRow>, gb_core::ports::wb_source::WbSourceError>,
    ffmpeg: &FakeFfmpegAdapter,
    lister: &FakeDirLister,
    import_log: &FakeWbImportLog,
) -> (FakeWbImportNotifier, Option<WbImportSummary>) {
    let notifier = FakeWbImportNotifier::default();
    super::import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies,
            ..Default::default()
        },
        ffmpeg,
        &notifier,
        &FakeCatalogNotifier::default(),
        None::<&FakeWatchReconfigurer>,
        lister,
        &FakeFolderProbe::default(),
        &FakeVolumeLabelResolver::default(),
        &FakeWbImportClock::default(),
        import_log,
    );
    let summary = notifier.complete_calls.lock().unwrap().first().cloned();
    (notifier, summary)
}

pub(super) fn lines_starting_with<'a>(text: &'a str, prefix: &str) -> Vec<&'a str> {
    text.lines().filter(|l| l.starts_with(prefix)).collect()
}

pub(super) fn row_at(id: i64, path: &str) -> WbMovieRow {
    row(|r| {
        r.movie_id = id;
        r.movie_name = path.rsplit('\\').next().unwrap().to_string();
        r.movie_path = path.to_string();
    })
}

pub(super) fn status_of(h: &Harness, path: &str) -> (String, String, i64, i64) {
    h.db.writer
        .lock()
        .unwrap()
        .query_row(
            "SELECT status, quick_hash, mtime, file_size FROM videos WHERE file_path = ?1",
            [path],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap()
}
