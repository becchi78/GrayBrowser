//! Empty-hash legacy thumbnails linked to videos by name.

use super::common::{
    harness, import_and_summarize, import_with_log, no_op_ffmpeg, recorded_unmatched, row, Harness,
};
use super::*;

// ---- empty-hash thumbnails linked by name ----

fn name_link_row(id: i64, path: &str, hash: &str) -> WbMovieRow {
    row(|r| {
        r.movie_id = id;
        r.movie_name = path.rsplit('\\').next().unwrap().to_string();
        r.movie_path = path.to_string();
        r.hash = hash.to_string();
    })
}

fn video_ready(h: &Harness, file_name: &str) -> Vec<bool> {
    let conn = h.db.writer.lock().unwrap();
    let mut stmt = conn
        .prepare("SELECT thumbnail_ready FROM videos WHERE file_name = ?1")
        .unwrap();
    stmt.query_map([file_name], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn recorded_names(h: &Harness) -> Vec<String> {
    recorded_unmatched(h)
        .into_iter()
        .map(|r| r.file_name)
        .collect()
}

#[test]
fn only_unambiguous_name_matches_to_videos_without_thumbnails_are_linked() {
    let h = harness();
    for name in [
        "Alpha.#.jpg",
        "dup.#.jpg",
        "done.#1e5e0fbf.jpg",
        "done.#.jpg",
        "nomatch.#.jpg",
        "orphan.#deadbeef.jpg",
    ] {
        std::fs::write(h.paths.thumbnail_folder.join(name), b"jpg").unwrap();
    }
    let rows = vec![
        name_link_row(1, "Z:\\v\\alpha", ""),
        name_link_row(2, "Z:\\a\\dup", ""),
        name_link_row(3, "Z:\\b\\dup", ""),
        name_link_row(4, "Z:\\v\\done", "1e5e0fbf"),
    ];
    let import_log = FakeWbImportLog::default();

    let (_, summary) = import_with_log(
        &h,
        Ok(rows),
        &no_op_ffmpeg(),
        &FakeDirLister::default(),
        &import_log,
    );

    let summary = summary.expect("completes");
    assert_eq!(summary.thumbnails_linked_by_name, 1);
    assert_eq!(summary.thumbnails_linked, 2, "done by hash + alpha by name");
    assert_eq!(summary.thumbnails_unmatched, 4);
    assert_eq!(summary.thumbnails_unmatched_empty_hash, 3);
    assert_eq!(video_ready(&h, "alpha"), vec![true]);
    assert_eq!(video_ready(&h, "dup"), vec![false, false]);
    let mut names = recorded_names(&h);
    names.sort();
    assert_eq!(
        names,
        [
            "done.#.jpg",
            "dup.#.jpg",
            "nomatch.#.jpg",
            "orphan.#deadbeef.jpg"
        ]
    );
    let text = import_log.text();
    assert!(text.contains("\"Alpha.#.jpg\" -> \"alpha\""), "{text}");
    assert!(text.contains("自動で紐付けた旧サムネイル: 1 件"));
    assert!(text.contains("自動対応できなかった旧サムネイル: 4 件"));
    assert!(text.contains("紐付け 2 (うち名前の一致で自動 1)"));
}

#[test]
fn a_failed_name_link_leaves_the_row_and_does_not_fail_the_import() {
    let h = harness();
    for name in ["good.#.jpg", "bad.#.jpg"] {
        std::fs::write(h.paths.thumbnail_folder.join(name), b"jpg").unwrap();
    }
    let ffmpeg = FakeFfmpegAdapter {
        convert_result: Box::new(|src| {
            if src
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("bad")
            {
                Err(FfmpegError::Spawn("boom".into()))
            } else {
                Ok(())
            }
        }),
        ..Default::default()
    };

    let (_, summary) = import_with_log(
        &h,
        Ok(vec![
            name_link_row(1, "Z:\\v\\good", ""),
            name_link_row(2, "Z:\\v\\bad", ""),
        ]),
        &ffmpeg,
        &FakeDirLister::default(),
        &FakeWbImportLog::default(),
    );

    let summary = summary.expect("the import still completes");
    assert_eq!(summary.thumbnails_linked_by_name, 1);
    assert_eq!(summary.thumbnails_linked, 1);
    assert_eq!(summary.thumbnails_unmatched, 1);
    assert_eq!(summary.thumbnails_unmatched_empty_hash, 1);
    assert_eq!(video_ready(&h, "good"), vec![true]);
    assert_eq!(video_ready(&h, "bad"), vec![false]);
    assert_eq!(recorded_names(&h), ["bad.#.jpg"]);
}

#[test]
fn only_empty_hash_rows_are_linked_by_name() {
    let h = harness();
    // Its name equals a video's, but it carries a hash that belongs to no
    // video, so it is a "no matching video" row and stays for a person.
    std::fs::write(h.paths.thumbnail_folder.join("alpha.#deadbeef.jpg"), b"jpg").unwrap();

    let summary = import_and_summarize(&h, vec![name_link_row(1, r"Z:\v\alpha", "")]);

    assert_eq!(summary.thumbnails_linked_by_name, 0);
    assert_eq!(summary.thumbnails_unmatched, 1);
    assert_eq!(video_ready(&h, "alpha"), vec![false]);
    assert_eq!(recorded_names(&h), ["alpha.#deadbeef.jpg"]);
}

#[test]
fn rows_recorded_for_another_folder_are_not_linked_by_an_import() {
    let h = harness();
    // The file exists, so only the folder check keeps it from being linked.
    let other = tempfile::tempdir().unwrap();
    std::fs::write(other.path().join("alpha.#.jpg"), b"jpg").unwrap();
    let other_folder = other.path().to_string_lossy().to_string();
    {
        let mut conn = h.db.writer.lock().unwrap();
        queries::replace_unmatched_thumbnails(
            &mut conn,
            &other_folder,
            &[queries::NewUnmatchedThumbnail {
                file_name: "alpha.#.jpg".to_string(),
                kind: "empty_hash".to_string(),
                thum_layout: None,
            }],
        )
        .unwrap();
    }

    let summary = import_and_summarize(&h, vec![name_link_row(1, r"Z:\v\alpha", "")]);

    assert_eq!(summary.thumbnails_linked_by_name, 0);
    assert_eq!(video_ready(&h, "alpha"), vec![false]);
    assert_eq!(recorded_names(&h), ["alpha.#.jpg"]);
}

#[test]
fn nothing_is_linked_by_name_when_the_list_could_not_be_saved() {
    let h = harness();
    std::fs::write(h.paths.thumbnail_folder.join("alpha.#.jpg"), b"jpg").unwrap();
    let folder = h.paths.thumbnail_folder.to_string_lossy().to_string();
    {
        let mut conn = h.db.writer.lock().unwrap();
        // A row left by an earlier import, which a stale read would use.
        queries::replace_unmatched_thumbnails(
            &mut conn,
            &folder,
            &[queries::NewUnmatchedThumbnail {
                file_name: "alpha.#.jpg".to_string(),
                kind: "empty_hash".to_string(),
                thum_layout: None,
            }],
        )
        .unwrap();
        conn.execute_batch(
            "CREATE TRIGGER reject BEFORE INSERT ON wb_unmatched_thumbnails                  BEGIN SELECT RAISE(ABORT, 'no'); END;",
        )
        .unwrap();
    }

    let summary = import_and_summarize(&h, vec![name_link_row(1, r"Z:\v\alpha", "")]);

    assert_eq!(summary.thumbnails_linked_by_name, 0);
    assert_eq!(video_ready(&h, "alpha"), vec![false]);
}

#[test]
fn a_reimport_does_not_count_a_name_link_twice() {
    let h = harness();
    std::fs::write(h.paths.thumbnail_folder.join("alpha.#.jpg"), b"jpg").unwrap();
    let rows = vec![name_link_row(1, "Z:\\v\\alpha", "")];

    let first = import_and_summarize(&h, rows.clone());
    let second = import_and_summarize(&h, rows);

    assert_eq!(first.thumbnails_linked_by_name, 1);
    assert_eq!(first.thumbnails_unmatched, 0);
    // The file is still in the folder, but its video has thumbnails now,
    // so it waits for a person instead of being applied again.
    assert_eq!(second.thumbnails_linked_by_name, 0);
    assert_eq!(second.thumbnails_linked, 0);
    assert_eq!(second.thumbnails_unmatched, 1);
    assert_eq!(second.thumbnails_unmatched_empty_hash, 1);
    assert_eq!(video_ready(&h, "alpha"), vec![true]);
    assert_eq!(recorded_names(&h), ["alpha.#.jpg"]);
}
