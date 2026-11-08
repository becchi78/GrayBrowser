//! The import log: rows, linking notes, summary and stage times.

use super::common::{
    count_of, harness, import_with_log, lines_starting_with, named_row, no_op_ffmpeg, replaced_row,
    row, Harness, OnlineDir, DASH,
};
use super::*;

// ---- import log ----

/// One row of every outcome: the log must account for each of them, state
/// what happened to its thumbnail, and close with a summary whose
/// equations hold.
#[test]
fn the_log_records_every_kind_of_row_and_the_summary() {
    let h = harness();
    let online_dir = tempfile::tempdir().unwrap();
    let online_path = online_dir.path().join("online.mp4");
    std::fs::write(&online_path, b"online bytes").unwrap();
    let online_path = online_path.to_string_lossy().to_string();

    std::fs::write(
        h.paths.thumbnail_folder.join("Linked.mp4.#1e5e0fbf.jpg"),
        b"jpg",
    )
    .unwrap();
    std::fs::write(
        h.paths.thumbnail_folder.join("Broken.mp4.#aaaaaaaa.jpg"),
        b"jpg",
    )
    .unwrap();
    std::fs::write(h.paths.thumbnail_folder.join("stray_file.jpg"), b"jpg").unwrap();
    {
        let conn = h.db.writer.lock().unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_one_insert BEFORE INSERT ON videos \
             WHEN NEW.file_path = 'Z:\\db_fail.mp4' \
             BEGIN SELECT RAISE(ABORT, 'forced failure'); END;",
        )
        .unwrap();
    }
    let ffmpeg = FakeFfmpegAdapter {
        convert_result: Box::new(|src| {
            if src.to_string_lossy().contains("aaaaaaaa") {
                Err(FfmpegError::NonZeroExit {
                    status: 1,
                    stderr: "simulated".into(),
                })
            } else {
                Ok(())
            }
        }),
        ..Default::default()
    };
    let rows = vec![
        row(|r| {
            r.movie_id = 1;
            r.movie_name = "online.mp4".into();
            r.movie_path = online_path.clone();
            r.tag = "a\nb".into();
        }),
        row(|r| {
            r.movie_id = 2;
            r.movie_name = "linked.mp4".into();
            r.movie_path = "Z:\\gone\\linked.mp4".into();
            r.hash = "1e5e0fbf".into();
        }),
        row(|r| {
            r.movie_id = 3;
            r.movie_name = "broken.mp4".into();
            r.movie_path = "Z:\\gone\\broken.mp4".into();
            r.hash = "aaaaaaaa".into();
        }),
        row(|r| {
            r.movie_id = 4;
            r.movie_name = "bad\u{2460}.mp4".into();
            r.movie_path = "Z:\\gone\\bad\u{2460}.mp4".into();
        }),
        row(|r| {
            r.movie_id = 5;
            r.movie_name = "date.mp4".into();
            r.movie_path = "Z:\\gone\\date.mp4".into();
            r.last_date = "not-a-date".into();
        }),
        row(|r| {
            r.movie_id = 6;
            r.movie_name = "db_fail.mp4".into();
            r.movie_path = "Z:\\db_fail.mp4".into();
        }),
        row(|r| {
            r.movie_id = 7;
            r.movie_name = "online.mp4".into();
            r.movie_path = online_path.clone();
        }),
        row(|r| {
            r.movie_id = 8;
            r.movie_name = "nohash.mp4".into();
            r.movie_path = "Z:\\gone\\nohash.mp4".into();
        }),
    ];
    let import_log = FakeWbImportLog::default();

    let (_, summary) = import_with_log(
        &h,
        Ok(rows),
        &ffmpeg,
        &FakeDirLister::default(),
        &import_log,
    );

    let summary = summary.expect("the import completes");
    assert_eq!(summary.total, 8);
    let text = import_log.text();
    let row_lines: Vec<&str> = text.lines().filter(|l| l.contains("movie_id=")).collect();
    assert_eq!(row_lines.len(), 8, "one line per row:\n{text}");
    let line = |id: i64| {
        *row_lines
            .iter()
            .find(|l| l.contains(&format!("movie_id={id} ")))
            .unwrap_or_else(|| panic!("no line for {id}:\n{text}"))
    };

    assert!(line(1).starts_with("[登録]"));
    assert!(line(1).contains("状態=オンライン") && line(1).contains("タグ数=2"));
    assert!(line(1).contains("紐付け=動画から生成"));
    assert!(line(2).contains("状態=オフライン") && line(2).contains("旧サムネイル=1e5e0fbf"));
    assert!(line(2).contains("紐付け=紐付け成功"));
    assert!(line(3).contains("紐付け=変換失敗(WebPへの変換に失敗"));
    assert!(line(4).starts_with("[未登録]") && line(4).contains("状態=判定前"));
    assert!(line(4).contains("機種依存文字のため未登録(検出した文字: \u{2460})"));
    assert!(line(5).starts_with("[失敗]") && line(5).contains("取り込めなかった(日付が不正"));
    assert!(line(6).starts_with("[失敗]") && line(6).contains("データベースへの書き込みに失敗"));
    assert!(line(7).starts_with("[スキップ]") && line(7).contains("登録済みのためスキップ"));
    assert!(line(8).contains("旧サムネイル=なし") && line(8).contains("紐付け=ハッシュなし"));

    assert!(text.contains("stray_file.jpg"), "unmatched JPG is listed");
    assert!(text.contains("== サマリー =="));
    assert!(text.contains("総数: 8"));
    assert!(text.contains("(合計 8: 一致)"), "{text}");
    assert!(text.contains("監視フォルダへ追加したパス: なし"));
    let at = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("{needle}"));
    assert!(
        at("== 読み込み") < at("== 動画の登録")
            && at("== 動画の登録") < at("== 旧サムネイルの紐付け")
    );
    assert!(at("== 旧サムネイルの紐付け") < at("監視フォルダへ追加したパス"));
    assert!(at("監視フォルダへ追加したパス") < at("== サマリー =="));
}

#[test]
fn the_log_shows_the_restored_path_of_a_question_mark_row() {
    let h = harness();
    let real = format!("a{DASH}.mp4");
    let dir = OnlineDir::new(&real);
    let lister = FakeDirLister::default().with_dir(&dir.folder(), &[&real]);
    let import_log = FakeWbImportLog::default();

    import_with_log(
        &h,
        Ok(vec![replaced_row(1, &dir.folder(), "a")]),
        &no_op_ffmpeg(),
        &lister,
        &import_log,
    );

    let text = import_log.text();
    let line = lines_starting_with(&text, "[登録]")[0].to_string();
    assert!(line.contains(&format!(".wb内パス=\"{}\\a?.mp4\"", dir.folder())));
    assert!(line.contains(&format!("復元後パス=\"{}\"", dir.file(&real))));
    assert!(line.contains("状態=オンライン"));
}

#[test]
fn the_log_lists_folders_added_to_the_watch_list() {
    let h = harness();
    let dir = OnlineDir::new("w.mp4");
    let notifier = FakeWbImportNotifier::default();
    let import_log = FakeWbImportLog::default();
    let reconfigurer = FakeWatchReconfigurer::default();

    super::import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies: Ok(vec![named_row(1, &dir.folder(), "w.mp4")]),
            ..Default::default()
        },
        &no_op_ffmpeg(),
        &notifier,
        &FakeCatalogNotifier::default(),
        Some(&reconfigurer),
        &FakeDirLister::default(),
        &FakeFolderProbe::default(),
        &FakeVolumeLabelResolver::default(),
        &FakeWbImportClock::default(),
        &import_log,
    );

    let text = import_log.text();
    assert!(text.contains("監視フォルダへの追加: する"));
    assert!(text.contains("監視フォルダへ追加したパス: 1 件"), "{text}");
    assert!(text.contains(&dir.folder()));
}

#[test]
fn an_unreadable_thumbnail_folder_is_noted_in_the_log() {
    let mut h = harness();
    h.paths.thumbnail_folder = h.paths.thumbnail_folder.join("missing");
    let import_log = FakeWbImportLog::default();

    import_with_log(
        &h,
        Ok(vec![row(|r| {
            r.movie_path = "Z:\\gone\\x.mp4".into();
            r.hash = "1e5e0fbf".into();
        })]),
        &no_op_ffmpeg(),
        &FakeDirLister::default(),
        &import_log,
    );

    let text = import_log.text();
    assert!(
        text.contains("旧サムネイルフォルダを読めなかった"),
        "{text}"
    );
    assert!(text.contains("紐付け=対応する旧サムネイルなし"));
}

#[test]
fn a_failed_read_leaves_a_log_with_the_reason() {
    let h = harness();
    let import_log = FakeWbImportLog::default();

    let (notifier, summary) = import_with_log(
        &h,
        Err(gb_core::ports::wb_source::WbSourceError::Query(
            "no such table: movie".into(),
        )),
        &no_op_ffmpeg(),
        &FakeDirLister::default(),
        &import_log,
    );

    assert!(summary.is_none());
    let failed = notifier.failed_calls.lock().unwrap()[0].clone();
    let text = import_log.text();
    assert!(text.contains("取り込みを中止しました"));
    assert!(text.contains(&failed), "the screen's reason is in the log");
    assert!(text.contains("no such table: movie"));
    assert!(text.contains("test.wb"), "header names the .wb");
    assert!(!text.contains("== サマリー =="));
}

#[test]
fn an_import_succeeds_even_when_the_log_cannot_be_written() {
    let h = harness();
    let import_log = FakeWbImportLog::failing();

    let (_, summary) = import_with_log(
        &h,
        Ok(vec![row(|r| r.movie_path = "Z:\\gone\\ok.mp4".into())]),
        &no_op_ffmpeg(),
        &FakeDirLister::default(),
        &import_log,
    );

    assert_eq!(summary.unwrap().registered, 1);
    assert_eq!(count_of(&h, "SELECT COUNT(*) FROM videos"), 1);
}

/// Records the order of log writes relative to the failure notification.
struct OrderRecorder {
    events: Arc<Mutex<Vec<String>>>,
}

impl WbImportLog for OrderRecorder {
    fn append(&self, text: &str) {
        self.events.lock().unwrap().push(format!("log:{text}"));
    }
    fn location(&self) -> String {
        String::new()
    }
}

impl WbImportNotifier for OrderRecorder {
    fn notify_progress(&self, _: WbImportStage, _: u32, _: Option<u32>) {}
    fn notify_complete(&self, _: &WbImportSummary) {}
    fn notify_failed(&self, _: &str) {
        self.events.lock().unwrap().push("failed".to_string());
    }
}

#[test]
fn the_failure_reason_is_logged_before_the_failure_is_announced() {
    let h = harness();
    let events = Arc::new(Mutex::new(Vec::new()));
    let recorder = OrderRecorder {
        events: events.clone(),
    };

    super::import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter::default(),
        &no_op_ffmpeg(),
        &recorder,
        &FakeCatalogNotifier::default(),
        None::<&FakeWatchReconfigurer>,
        &FakeDirLister::default(),
        &FakeFolderProbe::default(),
        &FakeVolumeLabelResolver::default(),
        &FakeWbImportClock::default(),
        &recorder,
    );

    let events = events.lock().unwrap();
    let failure_log = events
        .iter()
        .position(|e| e.starts_with("log:") && e.contains("取り込みを中止しました"))
        .expect("failure reason logged");
    let announced = events.iter().position(|e| e == "failed").unwrap();
    assert!(failure_log < announced, "{events:?}");
}

#[test]
fn the_summary_states_the_thumbnail_counts() {
    let h = harness();
    std::fs::write(h.paths.thumbnail_folder.join("A.mp4.#1e5e0fbf.jpg"), b"j").unwrap();
    std::fs::write(h.paths.thumbnail_folder.join("B.mp4.#aaaaaaaa.jpg"), b"j").unwrap();
    std::fs::write(h.paths.thumbnail_folder.join("stray.jpg"), b"j").unwrap();
    let ffmpeg = FakeFfmpegAdapter {
        convert_result: Box::new(|src| {
            if src.to_string_lossy().contains("aaaaaaaa") {
                Err(FfmpegError::NonZeroExit {
                    status: 1,
                    stderr: "simulated".into(),
                })
            } else {
                Ok(())
            }
        }),
        ..Default::default()
    };
    let import_log = FakeWbImportLog::default();

    import_with_log(
        &h,
        Ok(vec![
            row(|r| {
                r.movie_id = 1;
                r.movie_path = r"Z:\gone\a.mp4".into();
                r.hash = "1e5e0fbf".into();
            }),
            row(|r| {
                r.movie_id = 2;
                r.movie_path = r"Z:\gone\b.mp4".into();
                r.hash = "aaaaaaaa".into();
            }),
        ]),
        &ffmpeg,
        &FakeDirLister::default(),
        &import_log,
    );

    assert!(import_log.text().contains(
        "旧サムネイル: 紐付け 1 (うち名前の一致で自動 0) / 失敗 1 / 対応なし 1 (ハッシュが空 0 / 対応する動画なし 1) / "
    ));
}

fn run_with_clock(
    h: &Harness,
    movies: Result<Vec<WbMovieRow>, gb_core::ports::wb_source::WbSourceError>,
    clock: &FakeWbImportClock,
    import_log: &FakeWbImportLog,
) -> Option<WbImportSummary> {
    let notifier = FakeWbImportNotifier::default();
    super::import_all(
        &h.db,
        &h.paths,
        &FakeWbSourceAdapter {
            movies,
            ..Default::default()
        },
        &no_op_ffmpeg(),
        &notifier,
        &FakeCatalogNotifier::default(),
        None::<&FakeWatchReconfigurer>,
        &FakeDirLister::default(),
        &FakeFolderProbe::default(),
        &FakeVolumeLabelResolver::default(),
        clock,
        import_log,
    );
    let summary = notifier.complete_calls.lock().unwrap().first().cloned();
    summary
}

#[test]
fn the_log_stamps_each_stage_and_ends_with_the_durations() {
    let h = harness();
    let import_log = FakeWbImportLog::default();
    // Start, registering begins, linking begins, import ends.
    let clock = FakeWbImportClock::at_offsets(&[0, 2, 5, 70]);

    run_with_clock(&h, Ok(vec![row(|_| {})]), &clock, &import_log).expect("completes");

    let text = import_log.text();
    assert!(text.contains("\n開始時刻: 2026-11-02 12:00:00\n"), "{text}");
    for heading in [
        "== 読み込み (reading) == 2026-11-02 12:00:00\n",
        "== 動画の登録 (registering) == 2026-11-02 12:00:02\n",
        "== 旧サムネイルの紐付け (linking) == 2026-11-02 12:00:05\n",
    ] {
        assert!(text.contains(heading), "{heading}\n{text}");
    }
    assert!(
        text.ends_with(
            "\n終了時刻: 2026-11-02 12:01:10\n\
             所要時間: 全体 1分10秒 / 読み込み 2秒 / 動画の登録 3秒 / 旧サムネイルの紐付け 1分5秒\n"
        ),
        "{text}"
    );
    assert_eq!(clock.calls(), 4);
    let at = |needle: &str| text.find(needle).unwrap();
    assert!(at("== サマリー ==") < at("終了時刻:"));
}

#[test]
fn a_failed_read_still_reports_the_time_it_took() {
    let h = harness();
    let import_log = FakeWbImportLog::default();
    let clock = FakeWbImportClock::at_offsets(&[0, 3]);

    let summary = run_with_clock(
        &h,
        Err(gb_core::ports::wb_source::WbSourceError::Query(
            "boom".into(),
        )),
        &clock,
        &import_log,
    );

    assert!(summary.is_none());
    let text = import_log.text();
    assert!(text.contains("== 読み込み (reading) == 2026-11-02 12:00:00\n"));
    assert!(!text.contains("== 動画の登録"));
    assert!(text.ends_with("\n終了時刻: 2026-11-02 12:00:03\n所要時間: 全体 3秒 / 読み込み 3秒\n"));
}

#[test]
fn an_unopenable_wb_logs_times_too() {
    let import_log = FakeWbImportLog::default();
    let clock = FakeWbImportClock::at_offsets(&[0, 1]);
    let paths = WbImportPaths {
        wb_path: PathBuf::from("test.wb"),
        thumbnail_folder: PathBuf::from("thumbs"),
        thumbnails_dir: PathBuf::new(),
    };

    record_start_failure(&import_log, &clock, &paths, true, "not a database");

    let text = import_log.text();
    assert!(text.contains("開始時刻: 2026-11-02 12:00:00"));
    assert!(text.contains("== 読み込み (reading) == 2026-11-02 12:00:00"));
    assert!(text.contains("取り込みを中止しました: not a database"));
    assert!(text.ends_with("終了時刻: 2026-11-02 12:00:01\n所要時間: 全体 1秒 / 読み込み 1秒\n"));
}

#[test]
fn unmatched_thumbnails_are_split_into_empty_hash_and_no_matching_video() {
    let h = harness();
    for name in [
        "Gone1.mp4.#.jpg",
        "Gone2.mp4.#.jpg",
        ".#.jpg",
        "Orphan.mp4.#abcdef01.jpg",
    ] {
        std::fs::write(h.paths.thumbnail_folder.join(name), b"jpg").unwrap();
    }
    let import_log = FakeWbImportLog::default();

    let (_, summary) = import_with_log(
        &h,
        Ok(vec![row(|r| r.hash = String::new())]),
        &no_op_ffmpeg(),
        &FakeDirLister::default(),
        &import_log,
    );

    let summary = summary.expect("completes");
    assert_eq!(summary.thumbnails_unmatched, 4);
    assert_eq!(summary.thumbnails_unmatched_empty_hash, 2);
    let text = import_log.text();
    assert!(
        text.contains("自動対応できなかった旧サムネイル: 4 件"),
        "{text}"
    );
    assert!(text.contains(
        "ハッシュが空(自動では紐付けられない): 2 件\n  \"Gone1.mp4.#.jpg\"\n  \"Gone2.mp4.#.jpg\"\n"
    ));
    assert!(text.contains("対応する動画なし: 2 件\n"));
    assert!(text.contains("対応なし 4 (ハッシュが空 2 / 対応する動画なし 2)"));
}
