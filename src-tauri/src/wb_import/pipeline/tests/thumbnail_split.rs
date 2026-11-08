//! Splitting a legacy thumbnail into one frame per slot by the `.wb`'s
//! `thum_size`, and importing it whole whenever that cannot be done.

use super::common::{
    harness, image_size, import_all, jpeg_bytes, legacy_movies, link_with_layout, row,
    six_by_one_layout, split_ffmpeg, unassigned_dir, Harness,
};
use super::*;

// ---- legacy thumbnail splitting ----

fn crops(ffmpeg: &FakeFfmpegAdapter) -> Vec<CropRect> {
    ffmpeg
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter_map(|c| match c {
            FakeCall::SplitImageToWebp { outputs, .. } => Some(outputs.iter().map(|o| o.crop)),
            _ => None,
        })
        .flatten()
        .collect()
}

fn split_calls(ffmpeg: &FakeFfmpegAdapter) -> usize {
    ffmpeg
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| matches!(c, FakeCall::SplitImageToWebp { .. }))
        .count()
}

fn converts(ffmpeg: &FakeFfmpegAdapter) -> usize {
    ffmpeg
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| matches!(c, FakeCall::ConvertImageToWebp { .. }))
        .count()
}

struct SplitRun {
    summary: WbImportSummary,
    video_dir: PathBuf,
    video_id: String,
    ready: i64,
}

impl SplitRun {
    fn slot(&self, i: usize) -> PathBuf {
        self.video_dir.join(format!("{}_{i}.webp", self.video_id))
    }

    fn slot_tmp(&self, i: usize) -> PathBuf {
        self.video_dir
            .join(format!("{}_{i}.webp.tmp", self.video_id))
    }
}

/// Imports one offline row with a matching legacy JPG.
fn run_split_import(
    h: &Harness,
    profile_values: Result<Vec<String>, WbSourceError>,
    ffmpeg: &FakeFfmpegAdapter,
) -> SplitRun {
    let wb_source = FakeWbSourceAdapter {
        movies: Ok(vec![row(|r| {
            r.movie_path = "Z:\\split.mp4".to_string();
            r.hash = "1e5e0fbf".to_string();
        })]),
        profile_values,
        ..Default::default()
    };
    std::fs::write(
        h.paths.thumbnail_folder.join("MyVideo.mp4.#1e5e0fbf.jpg"),
        b"fake jpg",
    )
    .unwrap();
    let notifier = FakeWbImportNotifier::default();
    import_all(
        &h.db,
        &h.paths,
        &wb_source,
        ffmpeg,
        &notifier,
        &FakeCatalogNotifier::default(),
    );
    let summary = notifier.complete_calls.lock().unwrap()[0].clone();
    let conn = h.db.writer.lock().unwrap();
    let (video_id, ready): (String, i64) = conn
        .query_row(
            "SELECT id, thumbnail_ready FROM videos WHERE file_path = 'Z:\\split.mp4'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    SplitRun {
        summary,
        video_dir: unassigned_dir(&h.paths.thumbnails_dir),
        video_id,
        ready,
    }
}

fn six_by_one() -> Result<Vec<String>, WbSourceError> {
    Ok(vec!["200x150x6x1".to_string()])
}

#[test]
fn a_matching_size_splits_the_jpg_into_one_frame_per_slot() {
    let h = harness();
    let ffmpeg = split_ffmpeg(image_size(1200, 150), None);

    let run = run_split_import(&h, six_by_one(), &ffmpeg);

    assert_eq!(run.summary.thumbnails_linked, 1);
    assert_eq!(run.summary.thumbnails_failed, 0);
    assert_eq!(run.ready, 1);
    let rects = crops(&ffmpeg);
    assert_eq!(
        rects.iter().map(|r| r.x).collect::<Vec<_>>(),
        [0, 200, 400, 600, 800, 1000]
    );
    assert!(rects.iter().all(|r| r.y == 0 && r.w == 200 && r.h == 150));
    assert_eq!(converts(&ffmpeg), 0, "the whole-image path must not run");
    for i in 0..THUMBNAILS_PER_VIDEO {
        assert!(run.slot(i).exists(), "slot {i}");
        assert!(!run.slot_tmp(i).exists(), "tmp {i}");
    }
    // All six frames come from one ffmpeg call, each written through its
    // own slot's scratch path.
    assert_eq!(split_calls(&ffmpeg), 1);
    let outs: Vec<PathBuf> = ffmpeg
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter_map(|c| match c {
            FakeCall::SplitImageToWebp { outputs, .. } => {
                Some(outputs.iter().map(|o| o.tmp_path.clone()))
            }
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(outs.len(), THUMBNAILS_PER_VIDEO);
    for (i, out) in outs.iter().enumerate() {
        assert_eq!(out, &run.slot_tmp(i));
    }
}

fn assert_imported_whole(run: &SplitRun, ffmpeg: &FakeFfmpegAdapter) {
    assert_eq!(crops(ffmpeg).len(), 0, "no frame may be cut");
    assert_eq!(converts(ffmpeg), 1, "the whole image is converted once");
    assert_eq!(run.summary.thumbnails_linked, 1);
    assert_eq!(run.ready, 1);
    for i in 0..THUMBNAILS_PER_VIDEO {
        assert!(run.slot(i).exists(), "slot {i}");
    }
}

#[test]
fn a_size_mismatch_imports_the_jpg_whole() {
    let h = harness();
    let ffmpeg = split_ffmpeg(image_size(1200, 151), None);
    let run = run_split_import(&h, six_by_one(), &ffmpeg);
    assert_imported_whole(&run, &ffmpeg);
}

#[test]
fn a_failed_size_probe_imports_the_jpg_whole() {
    let h = harness();
    let ffmpeg = split_ffmpeg(
        Err(FfmpegError::NonZeroExit {
            status: 1,
            stderr: "bad jpg".into(),
        }),
        None,
    );
    let run = run_split_import(&h, six_by_one(), &ffmpeg);
    assert_imported_whole(&run, &ffmpeg);
}

#[test]
fn a_probe_without_a_size_imports_the_jpg_whole() {
    let h = harness();
    let ffmpeg = split_ffmpeg(Ok(VideoMetadata::default()), None);
    let run = run_split_import(&h, six_by_one(), &ffmpeg);
    assert_imported_whole(&run, &ffmpeg);
}

#[test]
fn a_missing_thum_size_imports_the_jpg_whole() {
    let h = harness();
    let ffmpeg = split_ffmpeg(image_size(1200, 150), None);
    let run = run_split_import(&h, Ok(vec![]), &ffmpeg);
    assert_imported_whole(&run, &ffmpeg);
}

#[test]
fn an_unreadable_thum_size_imports_the_jpg_whole() {
    let h = harness();
    let ffmpeg = split_ffmpeg(image_size(1200, 150), None);
    let err = WbSourceError::Query("no profile table".into());
    let run = run_split_import(&h, Err(err), &ffmpeg);
    assert_imported_whole(&run, &ffmpeg);
}

#[test]
fn an_unparsable_thum_size_imports_the_jpg_whole() {
    let h = harness();
    let ffmpeg = split_ffmpeg(image_size(1200, 150), None);
    let run = run_split_import(&h, Ok(vec!["wide".to_string()]), &ffmpeg);
    assert_imported_whole(&run, &ffmpeg);
}

#[test]
fn skins_that_disagree_on_thum_size_import_the_jpg_whole() {
    let h = harness();
    let ffmpeg = split_ffmpeg(image_size(1200, 150), None);
    let values = Ok(vec!["200x150x6x1".to_string(), "100x75x6x1".to_string()]);
    let run = run_split_import(&h, values, &ffmpeg);
    assert_imported_whole(&run, &ffmpeg);
}

#[test]
fn skins_that_agree_on_thum_size_still_split() {
    let h = harness();
    let ffmpeg = split_ffmpeg(image_size(1200, 150), None);
    let values = Ok(vec!["200x150x6x1".to_string(), "200x150x6x1".to_string()]);
    let run = run_split_import(&h, values, &ffmpeg);
    assert_eq!(crops(&ffmpeg).len(), 6);
    assert_eq!(run.summary.thumbnails_linked, 1);
}

#[test]
fn a_layout_that_is_not_six_frames_imports_the_jpg_whole() {
    let h = harness();
    let ffmpeg = split_ffmpeg(image_size(800, 150), None);
    let run = run_split_import(&h, Ok(vec!["200x150x4x1".to_string()]), &ffmpeg);
    assert_imported_whole(&run, &ffmpeg);
}

#[test]
fn a_split_failure_removes_every_slot_and_counts_as_failed() {
    let h = harness();
    let ffmpeg = split_ffmpeg(image_size(1200, 150), Some(600));

    let run = run_split_import(&h, six_by_one(), &ffmpeg);

    assert_eq!(run.summary.thumbnails_failed, 1);
    assert_eq!(run.summary.thumbnails_linked, 0);
    assert_eq!(run.ready, 0);
    assert_eq!(split_calls(&ffmpeg), 1, "the split is not retried");
    assert_eq!(converts(&ffmpeg), 0, "a failed split does not fall back");
    for i in 0..THUMBNAILS_PER_VIDEO {
        assert!(!run.slot(i).exists(), "slot {i}");
        assert!(!run.slot_tmp(i).exists(), "tmp {i}");
    }
}

#[test]
fn a_rename_failure_partway_removes_every_slot() {
    let h = harness();
    let video_id = "fixed-video-id-for-rename-failure-test";
    let video_dir = unassigned_dir(&h.paths.thumbnails_dir);
    std::fs::create_dir_all(&video_dir).unwrap();
    std::fs::write(
        h.paths.thumbnail_folder.join("MyVideo.mp4.#deadbeef.jpg"),
        b"fake jpg",
    )
    .unwrap();
    // A directory at slot 2's destination makes only that rename fail.
    std::fs::create_dir(video_dir.join(format!("{video_id}_2.webp"))).unwrap();
    let ffmpeg = split_ffmpeg(image_size(1200, 150), None);
    let layout = gb_core::wb_thumbnail_split::parse_thum_size("200x150x6x1").unwrap();

    let outcome = link_thumbnails(
        &h.paths.thumbnail_folder,
        &h.paths.thumbnails_dir,
        &[],
        &[(
            video_id.to_string(),
            "deadbeef".to_string(),
            "Z:\\rename-failure.mp4".to_string(),
        )],
        &[],
        &HashSet::new(),
        &HashSet::new(),
        &ffmpeg,
        Some(&layout),
        |_, _| {},
    );

    assert_eq!(outcome.failed, 1);
    assert_eq!(outcome.linked, 0);
    assert!(outcome.linked_video_ids.is_empty());
    assert_eq!(split_calls(&ffmpeg), 1);
    for i in [0, 1, 3, 4, 5] {
        assert!(!video_dir.join(format!("{video_id}_{i}.webp")).exists());
    }
    for i in 0..THUMBNAILS_PER_VIDEO {
        assert!(!video_dir.join(format!("{video_id}_{i}.webp.tmp")).exists());
    }
    assert!(video_dir.join(format!("{video_id}_2.webp")).is_dir());
}

fn probes(ffmpeg: &FakeFfmpegAdapter) -> usize {
    ffmpeg
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| matches!(c, FakeCall::ProbeMetadata(_)))
        .count()
}

#[test]
fn a_readable_jpeg_header_splits_without_asking_ffprobe() {
    let h = harness();
    let movies = legacy_movies(&h, 3, |_| jpeg_bytes(1200, 150));
    // A probe would fail, so success proves the size came from the header.
    let ffmpeg = split_ffmpeg(
        Err(FfmpegError::NonZeroExit {
            status: 1,
            stderr: "must not be probed".into(),
        }),
        None,
    );

    let outcome = link_with_layout(&h, &movies, &ffmpeg, Some(&six_by_one_layout()));

    assert_eq!((outcome.linked, outcome.failed), (3, 0));
    assert_eq!(probes(&ffmpeg), 0);
    assert_eq!(crops(&ffmpeg).len(), 3 * THUMBNAILS_PER_VIDEO);
    assert_eq!(converts(&ffmpeg), 0);
}

#[test]
fn a_jpeg_header_of_another_size_imports_whole_without_asking_ffprobe() {
    let h = harness();
    let movies = legacy_movies(&h, 1, |_| jpeg_bytes(1200, 151));
    let ffmpeg = split_ffmpeg(image_size(1200, 150), None);

    let outcome = link_with_layout(&h, &movies, &ffmpeg, Some(&six_by_one_layout()));

    assert_eq!(outcome.linked, 1);
    assert_eq!(probes(&ffmpeg), 0);
    assert_eq!(crops(&ffmpeg).len(), 0);
    assert_eq!(converts(&ffmpeg), 1);
}

#[test]
fn a_non_jpeg_or_damaged_file_falls_back_to_ffprobe() {
    let h = harness();
    let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13].to_vec();
    let mut truncated = jpeg_bytes(1200, 150);
    truncated.truncate(100);
    let movies = legacy_movies(&h, 3, |i| match i {
        0 => png.clone(),
        1 => truncated.clone(),
        _ => b"fake jpg".to_vec(),
    });
    let ffmpeg = split_ffmpeg(image_size(1200, 150), None);

    let outcome = link_with_layout(&h, &movies, &ffmpeg, Some(&six_by_one_layout()));

    assert_eq!((outcome.linked, outcome.failed), (3, 0));
    assert_eq!(probes(&ffmpeg), 3);
    assert_eq!(crops(&ffmpeg).len(), 3 * THUMBNAILS_PER_VIDEO);
}

#[test]
fn an_unreadable_size_imports_whole_as_before() {
    let h = harness();
    let movies = legacy_movies(&h, 2, |_| b"fake jpg".to_vec());
    let ffmpeg = split_ffmpeg(
        Err(FfmpegError::NonZeroExit {
            status: 1,
            stderr: "bad jpg".into(),
        }),
        None,
    );

    let outcome = link_with_layout(&h, &movies, &ffmpeg, Some(&six_by_one_layout()));

    assert_eq!((outcome.linked, outcome.failed), (2, 0));
    assert_eq!(probes(&ffmpeg), 2);
    assert_eq!(crops(&ffmpeg).len(), 0);
    assert_eq!(converts(&ffmpeg), 2);
}
