//! Text of the per-import log and the rules for naming and pruning its files.
//! Pure formatting and selection only; writing lives behind
//! `ports::wb_import_log`.

use std::collections::{HashMap, HashSet};

use crate::wb_import::{partition_unmatched, WbImportStage};

/// How many import logs are kept; older ones are deleted.
pub const KEEP_LOG_COUNT: usize = 10;

const FILE_PREFIX: &str = "wb-import-";
const FILE_SUFFIX: &str = ".log";

/// Calendar date and time of day, already broken down by the caller so this
/// crate needs no clock or time zone logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTimeParts {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl DateTimeParts {
    /// Breaks a Unix timestamp down in UTC. For callers that have no local
    /// clock to ask.
    pub fn from_unix_utc(secs: u64) -> Self {
        let days = secs / 86_400;
        let rem = secs % 86_400;
        // Civil-from-days (proleptic Gregorian), shifted so the year starts in
        // March and the leap day falls at the end.
        let z = days as i64 + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as u32;
        Self {
            year,
            month,
            day,
            hour: (rem / 3_600) as u32,
            minute: (rem % 3_600 / 60) as u32,
            second: (rem % 60) as u32,
        }
    }
}

/// `wb-import-YYYYMMDD-HHMMSS.log`. `sequence` is 0 for the first log of that
/// second; a later one in the same second gets `-<sequence>` so it never
/// overwrites the earlier file.
pub fn log_file_name(at: &DateTimeParts, sequence: u32) -> String {
    let stamp = format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        at.year, at.month, at.day, at.hour, at.minute, at.second
    );
    if sequence == 0 {
        format!("{FILE_PREFIX}{stamp}{FILE_SUFFIX}")
    } else {
        format!("{FILE_PREFIX}{stamp}-{sequence}{FILE_SUFFIX}")
    }
}

/// Sort key of an import-log file name: (timestamp digits, sequence). `None`
/// for any other name, so unrelated files (`app_r*.log`) are never selected.
fn parse_log_file_name(name: &str) -> Option<(u64, u32)> {
    let body = name.strip_prefix(FILE_PREFIX)?.strip_suffix(FILE_SUFFIX)?;
    let mut parts = body.split('-');
    let date = digits(parts.next()?, 8)?;
    let time = digits(parts.next()?, 6)?;
    let sequence = match parts.next() {
        None => 0,
        Some(seq) => u32::try_from(digits_up_to(seq, 9)?).ok()?,
    };
    if parts.next().is_some() {
        return None;
    }
    // Both halves are fixed width, so the combined number orders by time.
    Some((date * 1_000_000 + time, sequence))
}

fn digits(s: &str, len: usize) -> Option<u64> {
    if s.len() != len {
        return None;
    }
    digits_up_to(s, len)
}

fn digits_up_to(s: &str, max_len: usize) -> Option<u64> {
    if s.is_empty() || s.len() > max_len || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// The newest import log among `names`, if any.
pub fn latest_log_name(names: &[String]) -> Option<String> {
    names
        .iter()
        .filter_map(|n| parse_log_file_name(n).map(|key| (key, n)))
        .max_by_key(|(key, _)| *key)
        .map(|(_, n)| n.clone())
}

/// The import logs to delete so that only the newest `keep` remain. Age is
/// judged by the name's timestamp, so a clock set back can make a fresh log
/// count as old; that is accepted because the worst outcome is losing one
/// diagnostic file, and a second source of truth (file times) would be less
/// predictable on network drives. Names that
/// are not import logs are never returned.
pub fn logs_to_delete(names: &[String], keep: usize) -> Vec<String> {
    let mut logs: Vec<((u64, u32), &String)> = names
        .iter()
        .filter_map(|n| parse_log_file_name(n).map(|key| (key, n)))
        .collect();
    logs.sort_by_key(|(key, _)| std::cmp::Reverse(*key));
    logs.into_iter()
        .skip(keep)
        .map(|(_, n)| n.clone())
        .collect()
}

/// Whether the file was found on this machine when the row was imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Online,
    Offline,
    /// The row ended before availability was checked.
    NotJudged,
}

/// What happened to a `?` in the stored path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathRestoration {
    /// The row ended before the path was examined.
    NotChecked,
    /// Used as stored: no `?`, or the file exists as written, or nothing
    /// could be restored.
    Unchanged,
    Restored(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowOutcome {
    Registered,
    /// The path was already a registered video.
    AlreadyRegistered,
    /// Not registered because the file name has a machine-dependent character.
    InvalidName {
        detected_char: char,
    },
    Failed {
        reason: String,
    },
}

/// One `.wb` row as the import saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowLog {
    pub movie_id: i64,
    pub name: String,
    pub wb_path: String,
    pub restoration: PathRestoration,
    pub availability: Availability,
    pub tag_count: usize,
    pub thumbnail_hash: Option<String>,
    pub outcome: RowOutcome,
    /// Id of the newly created video; only for `Registered`.
    pub video_id: Option<String>,
}

/// What became of the row's legacy thumbnail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkStatus {
    /// Not registered by this import, so nothing was linked.
    NotApplicable,
    /// Online video: a new thumbnail is generated from the file itself.
    GeneratedFromVideo,
    Linked,
    Failed(String),
    /// The `.wb` row has no thumbnail hash.
    NoHash,
    /// No file in the legacy thumbnail folder carries the row's hash.
    NoMatchingFile,
}

/// Decides a row's [`LinkStatus`] from the linking pass.
pub fn resolve_link_status(
    row: &RowLog,
    linked_video_ids: &HashSet<String>,
    failed_video_ids: &HashMap<String, String>,
) -> LinkStatus {
    if row.outcome != RowOutcome::Registered {
        return LinkStatus::NotApplicable;
    }
    if row.availability == Availability::Online {
        return LinkStatus::GeneratedFromVideo;
    }
    if row.thumbnail_hash.is_none() {
        return LinkStatus::NoHash;
    }
    let Some(id) = row.video_id.as_deref() else {
        return LinkStatus::NoMatchingFile;
    };
    if linked_video_ids.contains(id) {
        LinkStatus::Linked
    } else if let Some(reason) = failed_video_ids.get(id) {
        LinkStatus::Failed(reason.clone())
    } else {
        LinkStatus::NoMatchingFile
    }
}

/// Wraps `s` in quotes with control characters made visible, so a name can
/// never break the one-line-per-row layout. Backslashes are left alone to keep
/// Windows paths readable.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One line (with trailing newline) for a row.
pub fn format_row(row: &RowLog, link: &LinkStatus) -> String {
    let (tag, outcome) = match &row.outcome {
        RowOutcome::Registered => ("登録", "登録".to_string()),
        RowOutcome::AlreadyRegistered => ("スキップ", "登録済みのためスキップ".to_string()),
        RowOutcome::InvalidName { detected_char } => (
            "未登録",
            format!("機種依存文字のため未登録(検出した文字: {detected_char})"),
        ),
        RowOutcome::Failed { reason } => ("失敗", format!("取り込めなかった({reason})")),
    };
    let restoration = match &row.restoration {
        PathRestoration::NotChecked => "未判定".to_string(),
        PathRestoration::Unchanged => "変更なし".to_string(),
        PathRestoration::Restored(p) => quote(p),
    };
    let availability = match row.availability {
        Availability::Online => "オンライン",
        Availability::Offline => "オフライン",
        Availability::NotJudged => "判定前",
    };
    let hash = row.thumbnail_hash.as_deref().unwrap_or("なし");
    let link = match link {
        LinkStatus::NotApplicable => "-".to_string(),
        LinkStatus::GeneratedFromVideo => "動画から生成".to_string(),
        LinkStatus::Linked => "紐付け成功".to_string(),
        LinkStatus::Failed(reason) => format!("変換失敗({reason})"),
        LinkStatus::NoHash => "ハッシュなし".to_string(),
        LinkStatus::NoMatchingFile => "対応する旧サムネイルなし".to_string(),
    };
    let video_id = row.video_id.as_deref().unwrap_or("-");
    format!(
        "[{tag}] movie_id={} 名前={} .wb内パス={} 復元後パス={restoration} 状態={availability} \
         タグ数={} 旧サムネイル={hash} 紐付け={link} 結果={outcome} 動画ID={video_id}\n",
        row.movie_id,
        quote(&row.name),
        quote(&row.wb_path),
        row.tag_count,
    )
}

/// `YYYY-MM-DD HH:MM:SS`, the form every time in the log takes.
pub fn format_datetime(at: &DateTimeParts) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        at.year, at.month, at.day, at.hour, at.minute, at.second
    )
}

/// A span in the form people read aloud: `1時間2分3秒`, `1分23秒`, `45秒`. Units
/// that would lead with a zero are dropped, but an inner zero is kept
/// (`1時間0分5秒`) so the digits keep their place. A span under a second is
/// `1秒未満` rather than `0秒`, which would read as "took no time"; only an
/// exactly zero span is `0秒`.
pub fn format_duration(millis: u64) -> String {
    if millis == 0 {
        return "0秒".to_string();
    }
    if millis < 1_000 {
        return "1秒未満".to_string();
    }
    let secs = millis / 1_000;
    let (hours, minutes, seconds) = (secs / 3_600, secs % 3_600 / 60, secs % 60);
    if hours > 0 {
        format!("{hours}時間{minutes}分{seconds}秒")
    } else if minutes > 0 {
        format!("{minutes}分{seconds}秒")
    } else {
        format!("{seconds}秒")
    }
}

/// Heading of a stage with the local time it began.
pub fn format_stage_heading(stage: WbImportStage, started_at: &DateTimeParts) -> String {
    format!(
        "\n== {} ({}) == {}\n",
        stage.label(),
        stage.as_str(),
        format_datetime(started_at)
    )
}

/// Turns the monotonic time each stage began (in order) and the time the import
/// ended into each stage's duration and the total. A stage lasts until the next
/// one begins, the last one until `end_ms`. Saturating, so a counter that
/// somehow reads backwards yields 0 rather than a huge span.
pub fn stage_durations(
    starts: &[(WbImportStage, u64)],
    end_ms: u64,
) -> (Vec<(WbImportStage, u64)>, u64) {
    let stages = starts
        .iter()
        .enumerate()
        .map(|(i, (stage, start))| {
            let next = starts.get(i + 1).map_or(end_ms, |(_, s)| *s);
            (*stage, next.saturating_sub(*start))
        })
        .collect();
    let total = starts
        .first()
        .map_or(0, |(_, first)| end_ms.saturating_sub(*first));
    (stages, total)
}

/// How long the whole import and each stage that began took. A stage that
/// never began (the import stopped earlier) is simply not in `stages`.
pub fn format_durations(stages: &[(WbImportStage, u64)], total_millis: u64) -> String {
    let mut out = format!("所要時間: 全体 {}", format_duration(total_millis));
    for (stage, millis) in stages {
        out.push_str(&format!(
            " / {} {}",
            stage.label(),
            format_duration(*millis)
        ));
    }
    out.push('\n');
    out
}

/// Closing lines: when the import ended and how long it and each stage took.
pub fn format_finish(
    finished_at: &DateTimeParts,
    stages: &[(WbImportStage, u64)],
    total_millis: u64,
) -> String {
    format!(
        "\n終了時刻: {}\n{}",
        format_datetime(finished_at),
        format_durations(stages, total_millis)
    )
}

/// Opening lines: when and what is being imported, and a privacy reminder.
pub fn format_header(
    started_at: &DateTimeParts,
    wb_path: &str,
    thumbnail_folder: &str,
    register_watch_folders: bool,
) -> String {
    format!(
        "GrayBrowser .wb取り込みログ\n\
         開始時刻: {}\n\
         .wbファイル: {}\n\
         旧サムネイルフォルダ: {}\n\
         監視フォルダへの追加: {}\n\
         ※ このログには実際のファイル名・パスが含まれます。不具合報告に添付するときはご注意ください。\n",
        format_datetime(started_at),
        quote(wb_path),
        quote(thumbnail_folder),
        if register_watch_folders {
            "する"
        } else {
            "しない"
        },
    )
}

pub fn format_read_count(total: u32) -> String {
    format!("{total} 件の行を読み込みました。\n")
}

pub fn format_failure(reason: &str) -> String {
    format!("\n取り込みを中止しました: {reason}\n")
}

/// Legacy thumbnail files that could not be linked automatically, grouped by
/// cause. Empty when there are none.
pub fn format_unmatched_filenames(filenames: &[String]) -> String {
    if filenames.is_empty() {
        return String::new();
    }
    let parts = partition_unmatched(filenames);
    let mut out = format!("自動対応できなかった旧サムネイル: {} 件\n", filenames.len());
    for (title, names) in [
        ("ハッシュが空(自動では紐付けられない)", &parts.empty_hash),
        ("対応する動画なし", &parts.no_matching_video),
    ] {
        if names.is_empty() {
            continue;
        }
        out.push_str(&format!("{title}: {} 件\n", names.len()));
        for name in names {
            out.push_str(&format!("  {}\n", quote(name)));
        }
    }
    out
}

/// Legacy thumbnails attached on their own because the name matched exactly one
/// video: `(image file name, video name)` per link. Empty when there are none.
pub fn format_name_linked_thumbnails(links: &[(String, String)]) -> String {
    if links.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "名前が一致したため自動で紐付けた旧サムネイル: {} 件\n",
        links.len()
    );
    for (image, video) in links {
        out.push_str(&format!("  {} -> {}\n", quote(image), quote(video)));
    }
    out
}

pub fn format_thumbnail_folder_error(reason: &str) -> String {
    format!("旧サムネイルフォルダを読めなかったため、紐付けを行いませんでした: {reason}\n")
}

pub fn format_watch_folders(added: &[String]) -> String {
    if added.is_empty() {
        return "\n監視フォルダへ追加したパス: なし\n".to_string();
    }
    let mut out = format!("\n監視フォルダへ追加したパス: {} 件\n", added.len());
    for path in added {
        out.push_str(&format!("  {}\n", quote(path)));
    }
    out
}

/// Roots (drive or share) whose videos were registered but could not be added
/// to the watch folders. Empty when there are none, so a normal import's log
/// gains no line.
pub fn format_skipped_root_folders(roots: &[String]) -> String {
    if roots.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "\nルート直下のため監視フォルダへ追加しなかったパス: {} 件\n",
        roots.len()
    );
    for path in roots {
        out.push_str(&format!("  {}\n", quote(path)));
    }
    out
}

/// What became of one volume label (`::LABEL`) found in the `.wb`'s paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VolumeLabelResult {
    /// Rewritten onto this drive root.
    Resolved(String),
    NotConnected,
    Ambiguous,
}

/// One volume label with the result of looking it up and the number of rows
/// whose path used it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeLabelLog {
    pub label: String,
    pub rows: u32,
    pub result: VolumeLabelResult,
}

/// Per-label result of turning `::LABEL` paths into drive-letter paths. Empty
/// when the `.wb` has no such path, so a normal import's log gains no line.
pub fn format_volume_labels(labels: &[VolumeLabelLog]) -> String {
    if labels.is_empty() {
        return String::new();
    }
    let mut out = format!("ボリュームラベルのパス: {} 種類\n", labels.len());
    for l in labels {
        let result = match &l.result {
            VolumeLabelResult::Resolved(root) => format!("ドライブ文字に直しました -> {root}"),
            VolumeLabelResult::NotConnected => {
                "接続中のドライブに見つからないため、直せませんでした".to_string()
            }
            VolumeLabelResult::Ambiguous => {
                "同じラベルのドライブが複数あるため、直せませんでした".to_string()
            }
        };
        out.push_str(&format!(
            "  {}: {} ({} 行)\n",
            quote(&l.label),
            result,
            l.rows
        ));
    }
    out
}

/// The numbers of the result screen, copied from the import's summary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogSummary {
    pub total: u32,
    pub registered: u32,
    pub registered_online: u32,
    pub registered_offline: u32,
    pub skipped: u32,
    pub skipped_invalid_name: u32,
    pub failed_rows: u32,
    pub clamped_scores: u32,
    pub tags_assigned: u32,
    pub source_tag_count: u32,
    pub ratings_from_star_tags: u32,
    pub thumbnails_linked: u32,
    /// Part of `thumbnails_linked` attached by name rather than by hash.
    pub thumbnails_linked_by_name: u32,
    pub thumbnails_failed: u32,
    pub thumbnails_unmatched: u32,
    /// Part of `thumbnails_unmatched` whose file name carries an empty hash.
    pub thumbnails_unmatched_empty_hash: u32,
    pub thumbnails_skipped_online: u32,
    pub volume_label_resolved: u32,
    pub volume_label_unresolved: u32,
}

/// Closing summary, including the two reconciliation equations of the result
/// screen with whether each one holds.
pub fn format_summary(s: &LogSummary) -> String {
    let rows_sum = s.registered + s.skipped + s.skipped_invalid_name + s.failed_rows;
    let registered_sum = s.registered_online + s.registered_offline;
    let verdict = |ok: bool| if ok { "一致" } else { "不一致" };
    let mut out = format!(
        "\n== サマリー ==\n\
         総数: {total}\n\
         登録: {registered} (オンライン {online} / オフライン {offline})\n\
         登録済みのためスキップ: {skipped}\n\
         機種依存文字のため未登録: {invalid}\n\
         取り込めなかった行: {failed}\n\
         突き合わせ: 総数 {total} = 登録 {registered} + スキップ {skipped} + 未登録 {invalid} + 取り込めなかった {failed} \
         (合計 {rows_sum}: {rows_verdict})\n\
         突き合わせ: 登録 {registered} = オンライン {online} + オフライン {offline} \
         (合計 {registered_sum}: {reg_verdict})\n\
         スコアを5に丸めた行: {clamped}\n\
         タグ: 割り当て {tags_assigned} / 元データ {source_tags}\n\
         星タグから決めたレーティング: {stars}\n\
         旧サムネイル: 紐付け {linked} (うち名前の一致で自動 {by_name}) / 失敗 {thumb_failed} / 対応なし {unmatched} (ハッシュが空 {empty_hash} / 対応する動画なし {no_video}) / 動画から生成するため未使用 {skipped_online}\n",
        total = s.total,
        registered = s.registered,
        online = s.registered_online,
        offline = s.registered_offline,
        skipped = s.skipped,
        invalid = s.skipped_invalid_name,
        failed = s.failed_rows,
        rows_verdict = verdict(rows_sum == s.total),
        reg_verdict = verdict(registered_sum == s.registered),
        clamped = s.clamped_scores,
        tags_assigned = s.tags_assigned,
        source_tags = s.source_tag_count,
        stars = s.ratings_from_star_tags,
        linked = s.thumbnails_linked,
        by_name = s.thumbnails_linked_by_name,
        thumb_failed = s.thumbnails_failed,
        unmatched = s.thumbnails_unmatched,
        empty_hash = s.thumbnails_unmatched_empty_hash,
        no_video = s
            .thumbnails_unmatched
            .saturating_sub(s.thumbnails_unmatched_empty_hash),
        skipped_online = s.thumbnails_skipped_online,
    );
    // Only for a `.wb` that has such paths, so a normal import's summary is unchanged.
    if s.volume_label_resolved > 0 || s.volume_label_unresolved > 0 {
        out.push_str(&format!(
            "ボリュームラベルのパス: ドライブ文字に直した {} / 直せなかった {}\n",
            s.volume_label_resolved, s.volume_label_unresolved
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn row(outcome: RowOutcome) -> RowLog {
        RowLog {
            movie_id: 7,
            name: "a.mp4".into(),
            wb_path: "T:\\v\\a.mp4".into(),
            restoration: PathRestoration::Unchanged,
            availability: Availability::Offline,
            tag_count: 3,
            thumbnail_hash: Some("1e5e0fbf".into()),
            outcome,
            video_id: None,
        }
    }

    #[test]
    fn file_name_is_zero_padded_and_sequence_only_after_the_first() {
        let t = DateTimeParts {
            year: 2026,
            month: 1,
            day: 2,
            hour: 3,
            minute: 4,
            second: 5,
        };
        assert_eq!(log_file_name(&t, 0), "wb-import-20260102-030405.log");
        assert_eq!(log_file_name(&t, 2), "wb-import-20260102-030405-2.log");
    }

    #[test]
    fn unix_time_breaks_down_in_utc() {
        let t = DateTimeParts::from_unix_utc(0);
        assert_eq!((t.year, t.month, t.day, t.hour), (1970, 1, 1, 0));
        // 2024-02-29 23:59:59 UTC, a leap day.
        let t = DateTimeParts::from_unix_utc(1_709_251_199);
        assert_eq!(
            (t.year, t.month, t.day, t.hour, t.minute, t.second),
            (2024, 2, 29, 23, 59, 59)
        );
        let t = DateTimeParts::from_unix_utc(1_793_448_000);
        assert_eq!((t.year, t.month, t.day, t.hour), (2026, 10, 31, 12));
    }

    #[test]
    fn deletes_everything_beyond_the_newest_n() {
        let list = names(&[
            "wb-import-20261001-000000.log",
            "wb-import-20261003-000000.log",
            "wb-import-20261002-000000.log",
        ]);
        let mut del = logs_to_delete(&list, 2);
        del.sort();
        assert_eq!(del, names(&["wb-import-20261001-000000.log"]));
        assert!(logs_to_delete(&list, 3).is_empty());
        assert!(logs_to_delete(&list, 10).is_empty());
        assert_eq!(logs_to_delete(&list, 0).len(), 3);
    }

    #[test]
    fn same_second_sequence_counts_as_newer() {
        let list = names(&[
            "wb-import-20261001-000000.log",
            "wb-import-20261001-000000-1.log",
            "wb-import-20261001-000000-10.log",
            "wb-import-20261001-000000-2.log",
        ]);
        assert_eq!(
            latest_log_name(&list).as_deref(),
            Some("wb-import-20261001-000000-10.log")
        );
        assert_eq!(
            logs_to_delete(&list, 1),
            names(&[
                "wb-import-20261001-000000-2.log",
                "wb-import-20261001-000000-1.log",
                "wb-import-20261001-000000.log",
            ])
        );
    }

    #[test]
    fn other_files_are_never_selected() {
        let list = names(&[
            "app_rCURRENT.log",
            "app_r00001.log",
            "wb-import-latest.log",
            "wb-import-20261001-00000.log",
            "wb-import-2026100-1000000.log",
            "wb-import-20261001-000000.txt",
            "wb-import-20261001-000000-x.log",
            "wb-import-20261001-000000.log",
        ]);
        assert_eq!(
            logs_to_delete(&list, 0),
            names(&["wb-import-20261001-000000.log"])
        );
        assert_eq!(
            latest_log_name(&list).as_deref(),
            Some("wb-import-20261001-000000.log")
        );
        assert_eq!(latest_log_name(&names(&["app_rCURRENT.log"])), None);
        assert_eq!(latest_log_name(&[]), None);
    }

    #[test]
    fn registered_row_line_has_every_field() {
        let mut r = row(RowOutcome::Registered);
        r.video_id = Some("id-1".into());
        r.restoration = PathRestoration::Restored("T:\\v\\あ.mp4".into());
        let line = format_row(&r, &LinkStatus::Linked);
        assert!(line.starts_with("[登録] movie_id=7 "));
        assert!(line.contains("名前=\"a.mp4\""));
        assert!(line.contains(".wb内パス=\"T:\\v\\a.mp4\""));
        assert!(line.contains("復元後パス=\"T:\\v\\あ.mp4\""));
        assert!(line.contains("状態=オフライン"));
        assert!(line.contains("タグ数=3"));
        assert!(line.contains("旧サムネイル=1e5e0fbf"));
        assert!(line.contains("紐付け=紐付け成功"));
        assert!(line.contains("結果=登録"));
        assert!(line.ends_with("動画ID=id-1\n"));
        assert_eq!(line.matches('\n').count(), 1);
    }

    #[test]
    fn skipped_row_line() {
        let line = format_row(
            &row(RowOutcome::AlreadyRegistered),
            &LinkStatus::NotApplicable,
        );
        assert!(line.starts_with("[スキップ] "));
        assert!(line.contains("結果=登録済みのためスキップ"));
        assert!(line.contains("紐付け=-"));
    }

    #[test]
    fn invalid_name_row_line_shows_the_char_and_undecided_state() {
        let mut r = row(RowOutcome::InvalidName {
            detected_char: '①'
        });
        r.availability = Availability::NotJudged;
        let line = format_row(&r, &LinkStatus::NotApplicable);
        assert!(line.starts_with("[未登録] "));
        assert!(line.contains("状態=判定前"));
        assert!(line.contains("機種依存文字のため未登録(検出した文字: ①)"));
    }

    #[test]
    fn failed_row_line_carries_the_reason_and_unchecked_path() {
        let mut r = row(RowOutcome::Failed {
            reason: "日付が不正".into(),
        });
        r.availability = Availability::NotJudged;
        r.restoration = PathRestoration::NotChecked;
        r.thumbnail_hash = None;
        let line = format_row(&r, &LinkStatus::NotApplicable);
        assert!(line.starts_with("[失敗] "));
        assert!(line.contains("取り込めなかった(日付が不正)"));
        assert!(line.contains("復元後パス=未判定"));
        assert!(line.contains("旧サムネイル=なし"));
    }

    #[test]
    fn control_characters_in_names_cannot_split_a_line() {
        let mut r = row(RowOutcome::Registered);
        r.name = "a\nb\r\tc\u{1}.mp4".into();
        let line = format_row(&r, &LinkStatus::GeneratedFromVideo);
        assert_eq!(line.matches('\n').count(), 1);
        assert!(line.contains("名前=\"a\\nb\\r\\tc\\u{1}.mp4\""));
        assert!(line.contains("紐付け=動画から生成"));
    }

    #[test]
    fn link_status_covers_every_case() {
        let linked: HashSet<String> = ["L".to_string()].into();
        let failed: HashMap<String, String> = [("F".to_string(), "boom".to_string())].into();

        let mut r = row(RowOutcome::Registered);
        r.video_id = Some("L".into());
        assert_eq!(
            resolve_link_status(&r, &linked, &failed),
            LinkStatus::Linked
        );
        r.video_id = Some("F".into());
        assert_eq!(
            resolve_link_status(&r, &linked, &failed),
            LinkStatus::Failed("boom".into())
        );
        r.video_id = Some("Z".into());
        assert_eq!(
            resolve_link_status(&r, &linked, &failed),
            LinkStatus::NoMatchingFile
        );
        r.thumbnail_hash = None;
        assert_eq!(
            resolve_link_status(&r, &linked, &failed),
            LinkStatus::NoHash
        );
        r.availability = Availability::Online;
        assert_eq!(
            resolve_link_status(&r, &linked, &failed),
            LinkStatus::GeneratedFromVideo
        );
        assert_eq!(
            resolve_link_status(&row(RowOutcome::AlreadyRegistered), &linked, &failed),
            LinkStatus::NotApplicable
        );
    }

    #[test]
    fn header_and_headings() {
        let h = format_header(&at(12, 50, 46), "C:\\a.wb", "C:\\thumbs", true);
        assert!(h.contains("開始時刻: 2026-11-02 12:50:46\n"));
        assert!(h.contains(".wbファイル: \"C:\\a.wb\""));
        assert!(h.contains("監視フォルダへの追加: する"));
        assert!(h.contains("実際のファイル名・パスが含まれます"));
        assert!(format_header(&at(0, 0, 0), "a", "b", false).contains("追加: しない"));
        assert_eq!(
            format_stage_heading(WbImportStage::Registering, &at(12, 50, 48)),
            "\n== 動画の登録 (registering) == 2026-11-02 12:50:48\n"
        );
    }

    fn at(hour: u32, minute: u32, second: u32) -> DateTimeParts {
        DateTimeParts {
            year: 2026,
            month: 11,
            day: 2,
            hour,
            minute,
            second,
        }
    }

    #[test]
    fn datetime_is_zero_padded() {
        let t = DateTimeParts {
            year: 2026,
            month: 1,
            day: 2,
            hour: 3,
            minute: 4,
            second: 5,
        };
        assert_eq!(format_datetime(&t), "2026-01-02 03:04:05");
    }

    #[test]
    fn duration_drops_leading_zero_units_and_marks_sub_second() {
        assert_eq!(format_duration(0), "0秒");
        assert_eq!(format_duration(1), "1秒未満");
        assert_eq!(format_duration(999), "1秒未満");
        assert_eq!(format_duration(1_000), "1秒");
        assert_eq!(format_duration(1_999), "1秒");
        assert_eq!(format_duration(45_000), "45秒");
        assert_eq!(format_duration(59_999), "59秒");
        assert_eq!(format_duration(60_000), "1分0秒");
        assert_eq!(format_duration(83_000), "1分23秒");
        assert_eq!(format_duration(3_599_000), "59分59秒");
        assert_eq!(format_duration(3_600_000), "1時間0分0秒");
        assert_eq!(format_duration(3_723_000), "1時間2分3秒");
    }

    #[test]
    fn durations_list_only_the_stages_that_began() {
        let all = [
            (WbImportStage::Reading, 2_000),
            (WbImportStage::Registering, 83_000),
            (WbImportStage::Linking, 500),
        ];
        assert_eq!(
            format_durations(&all, 85_500),
            "所要時間: 全体 1分25秒 / 読み込み 2秒 / 動画の登録 1分23秒 / 旧サムネイルの紐付け 1秒未満\n"
        );
        assert_eq!(
            format_durations(&all[..1], 2_000),
            "所要時間: 全体 2秒 / 読み込み 2秒\n"
        );
        assert_eq!(format_durations(&[], 0), "所要時間: 全体 0秒\n");
    }

    #[test]
    fn stage_lasts_until_the_next_one_begins() {
        let starts = [
            (WbImportStage::Reading, 1_000),
            (WbImportStage::Registering, 3_000),
            (WbImportStage::Linking, 10_000),
        ];
        let (stages, total) = stage_durations(&starts, 12_500);
        assert_eq!(
            stages,
            vec![
                (WbImportStage::Reading, 2_000),
                (WbImportStage::Registering, 7_000),
                (WbImportStage::Linking, 2_500),
            ]
        );
        assert_eq!(total, 11_500);
    }

    #[test]
    fn stopped_import_has_only_the_stages_that_began() {
        let (stages, total) = stage_durations(&[(WbImportStage::Reading, 500)], 900);
        assert_eq!(stages, vec![(WbImportStage::Reading, 400)]);
        assert_eq!(total, 400);
        assert_eq!(stage_durations(&[], 900), (vec![], 0));
    }

    #[test]
    fn backwards_counter_saturates_to_zero() {
        let (stages, total) = stage_durations(&[(WbImportStage::Reading, 900)], 100);
        assert_eq!(stages, vec![(WbImportStage::Reading, 0)]);
        assert_eq!(total, 0);
    }

    #[test]
    fn finish_has_the_end_time_then_the_durations() {
        assert_eq!(
            format_finish(&at(12, 52, 11), &[(WbImportStage::Reading, 1_000)], 1_000),
            "\n終了時刻: 2026-11-02 12:52:11\n所要時間: 全体 1秒 / 読み込み 1秒\n"
        );
    }

    #[test]
    fn watch_folder_and_unmatched_sections() {
        assert!(format_watch_folders(&[]).contains("なし"));
        let text = format_watch_folders(&names(&["T:\\v"]));
        assert!(text.contains("1 件") && text.contains("  \"T:\\v\"\n"));
        assert_eq!(format_unmatched_filenames(&[]), "");
        let text = format_unmatched_filenames(&names(&["x.#abcdef01.jpg"]));
        assert!(text.contains("1 件") && text.contains("\"x.#abcdef01.jpg\""));
        assert!(text.contains("対応する動画なし: 1 件"));
        assert!(!text.contains("ハッシュが空"));
    }

    #[test]
    fn skipped_root_folders_section_is_silent_when_empty() {
        assert_eq!(format_skipped_root_folders(&[]), "");
        let text = format_skipped_root_folders(&names(&["E:\\", "\\\\nas\\share\\"]));
        assert_eq!(
            text,
            "\nルート直下のため監視フォルダへ追加しなかったパス: 2 件\n  \"E:\\\"\n  \"\\\\nas\\share\\\"\n"
        );
    }

    #[test]
    fn volume_label_section_is_silent_when_there_are_none() {
        assert_eq!(format_volume_labels(&[]), "");
    }

    #[test]
    fn volume_label_section_states_each_result() {
        let text = format_volume_labels(&[
            VolumeLabelLog {
                label: "ESD-USB".into(),
                rows: 6,
                result: VolumeLabelResult::Resolved("E:\\".into()),
            },
            VolumeLabelLog {
                label: "GONE".into(),
                rows: 2,
                result: VolumeLabelResult::NotConnected,
            },
            VolumeLabelLog {
                label: "TWIN".into(),
                rows: 1,
                result: VolumeLabelResult::Ambiguous,
            },
        ]);
        assert!(text.starts_with("ボリュームラベルのパス: 3 種類\n"));
        assert!(text.contains("\"ESD-USB\": ドライブ文字に直しました -> E:\\ (6 行)\n"));
        assert!(text
            .contains("\"GONE\": 接続中のドライブに見つからないため、直せませんでした (2 行)\n"));
        assert!(text
            .contains("\"TWIN\": 同じラベルのドライブが複数あるため、直せませんでした (1 行)\n"));
    }

    #[test]
    fn summary_mentions_volume_labels_only_when_present() {
        assert!(!format_summary(&summary()).contains("ボリュームラベル"));
        let mut s = summary();
        s.volume_label_resolved = 4;
        s.volume_label_unresolved = 2;
        assert!(format_summary(&s)
            .contains("ボリュームラベルのパス: ドライブ文字に直した 4 / 直せなかった 2\n"));
    }

    #[test]
    fn unmatched_files_are_listed_under_their_cause() {
        let text = format_unmatched_filenames(&names(&[
            "a.mp4.#.jpg",
            "orphan.#abcdef01.jpg",
            ".#.jpg",
            "b.mp4.#.jpg",
        ]));
        assert_eq!(
            text,
            "自動対応できなかった旧サムネイル: 4 件\n\
             ハッシュが空(自動では紐付けられない): 2 件\n  \"a.mp4.#.jpg\"\n  \"b.mp4.#.jpg\"\n\
             対応する動画なし: 2 件\n  \"orphan.#abcdef01.jpg\"\n  \".#.jpg\"\n"
        );
    }

    fn summary() -> LogSummary {
        LogSummary {
            total: 10,
            registered: 6,
            registered_online: 2,
            registered_offline: 4,
            skipped: 2,
            skipped_invalid_name: 1,
            failed_rows: 1,
            ..LogSummary::default()
        }
    }

    #[test]
    fn summary_states_both_equations_as_holding() {
        let text = format_summary(&summary());
        assert!(text.contains(
            "総数 10 = 登録 6 + スキップ 2 + 未登録 1 + 取り込めなかった 1 (合計 10: 一致)"
        ));
        assert!(text.contains("登録 6 = オンライン 2 + オフライン 4 (合計 6: 一致)"));
    }

    #[test]
    fn summary_splits_unmatched_thumbnails_by_cause() {
        let mut s = summary();
        s.thumbnails_unmatched = 99;
        s.thumbnails_unmatched_empty_hash = 97;
        assert!(format_summary(&s).contains("対応なし 99 (ハッシュが空 97 / 対応する動画なし 2)"));
    }

    #[test]
    fn summary_states_how_many_links_were_made_by_name() {
        let mut s = summary();
        s.thumbnails_linked = 80;
        s.thumbnails_linked_by_name = 77;
        assert!(format_summary(&s).contains("紐付け 80 (うち名前の一致で自動 77)"));
    }

    #[test]
    fn name_links_are_listed_image_to_video() {
        assert_eq!(format_name_linked_thumbnails(&[]), "");
        let text = format_name_linked_thumbnails(&[("A.#.jpg".to_string(), "a".to_string())]);
        assert_eq!(
            text,
            "名前が一致したため自動で紐付けた旧サムネイル: 1 件\n  \"A.#.jpg\" -> \"a\"\n"
        );
    }

    #[test]
    fn summary_flags_a_broken_equation() {
        let mut s = summary();
        s.total = 11;
        s.registered_offline = 3;
        let text = format_summary(&s);
        assert!(text.contains("合計 10: 不一致"));
        assert!(text.contains("合計 5: 不一致"));
    }
}
