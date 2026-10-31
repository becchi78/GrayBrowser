//! `.wb` (legacy WhiteBrowser) row -> GrayBrowser domain conversion. Pure
//! logic only -- no rusqlite, no file I/O. Row data comes from
//! `ports::wb_source`.

use crate::ports::wb_source::WbMovieRow;

/// Converts a `.wb` `score` (0-20) to a 0-5 star rating. 0 = unrated, 1-5
/// map to themselves, 6+ are clamped to 5 (confirmed with the data owner;
/// proportional remapping was rejected).
pub fn parse_score_to_rating(score: i64) -> u8 {
    score.clamp(0, 5) as u8
}

/// Splits a `.wb` `tag` column (newline-separated) into individual tags.
/// Blank lines are dropped so a trailing or doubled
/// newline doesn't produce an empty tag.
pub fn split_tags(tag: &str) -> Vec<String> {
    tag.split('\n')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

/// Max star rating; also the cap for a star-only tag with more stars than that.
const MAX_RATING: u8 = 5;

/// If `tag` consists solely of `★` (after width folding and trimming),
/// returns the star count capped at 5. WhiteBrowser users encoded ratings as
/// tags like `★★★`; anything mixed with other characters (`★お気に入り`) or
/// using a different glyph (`☆`) is a real tag and yields `None`.
pub fn star_tag_rating(tag: &str) -> Option<u8> {
    let folded = wb_fold_width(tag);
    let trimmed = folded.trim();
    if trimmed.is_empty() || !trimmed.chars().all(|c| c == '★') {
        return None;
    }
    let count = trimmed.chars().count();
    Some(count.min(MAX_RATING as usize) as u8)
}

/// Splits a `.wb` `tag` column into ordinary tags and the rating encoded by
/// star-only tags (the maximum when several are present).
fn split_tags_and_star_rating(tag: &str) -> (Vec<String>, Option<u8>) {
    let mut star_rating: Option<u8> = None;
    let mut tags = Vec::new();
    for t in split_tags(tag) {
        match star_tag_rating(&t) {
            Some(r) => star_rating = Some(star_rating.map_or(r, |cur| cur.max(r))),
            None => tags.push(t),
        }
    }
    (tags, star_rating)
}

/// Extracts the 8-hex-digit thumbnail hash from a legacy thumbnail filename
/// of the form `[video filename].#<hash>.jpg`. Matches lowercase hex only
/// -- real data never produced uppercase.
pub fn extract_thumbnail_hash(filename: &str) -> Option<String> {
    let (_, rest) = filename.rsplit_once(".#")?;
    let hash = rest.strip_suffix(".jpg")?;
    let is_lowercase_hex_8 = hash.len() == 8
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    is_lowercase_hex_8.then(|| hash.to_string())
}

/// Folds full-width (Zenkaku) ASCII-range characters (U+FF01-U+FF5E) and the
/// full-width space (U+3000) to their half-width equivalents. Mirrors
/// `tags::fold_width` (kept private there); duplicated here rather than
/// exposed crate-wide, since it's a step in this module's own
/// `.wb`-import-only key logic (see `wb_tag_merge_key`), which diverges from
/// `tags::normalize_tag_name` by additionally case-folding.
fn wb_fold_width(input: &str) -> String {
    input
        .chars()
        .map(|c| match c {
            '\u{3000}' => ' ',
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            other => other,
        })
        .collect()
}

/// Computes a matching key used ONLY to decide which `.wb` tag strings
/// should be merged into a single GrayBrowser tag during import: width-folds
/// full-width characters/space to half-width (as `tags::normalize_tag_name`
/// does), trims leading/trailing whitespace, and additionally case-folds via
/// `to_lowercase()`. Confirmed with the data owner: real `.wb` data has
/// casing variants of the same tag (e.g. "Action" / "action") that should be
/// treated as one tag on import.
///
/// This is a matching key, NOT the tag name to store in the database -- the
/// caller decides what display string to persist for a merged group (e.g.
/// first-seen spelling); this function only says which raw strings belong to
/// the same group.
///
/// Contrast with `tags::normalize_tag_name`, used for user-typed manual
/// tags, which deliberately does *not* case-fold. That function is
/// unaffected by this one and must not be changed to match it.
pub fn wb_tag_merge_key(tag: &str) -> String {
    wb_fold_width(tag).trim().to_lowercase()
}

/// Result of matching legacy `.wb` thumbnail files (by embedded hash)
/// against `movie` rows' `hash` column, for thumbnail migration.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThumbnailLinkPlan {
    /// `(video_id, source_filename)` pairs to copy/convert into the new
    /// `thumbnails/[id].webp` layout. If `movies` contains duplicate hashes
    /// (two videos sharing one legacy thumbnail file), the same filename
    /// appears once per matching `video_id` here.
    pub matched: Vec<(String, String)>,
    /// Filenames from the legacy thumbnail folder that matched no `movies`
    /// hash -- either the filename didn't fit the expected naming pattern,
    /// or its hash isn't among `movies`. Reported to the user in the import
    /// log rather than silently dropped.
    pub unmatched_filenames: Vec<String>,
    /// Filenames whose hash belongs to an online video (and to no offline
    /// one). Online videos get a fresh thumbnail from the video itself, so
    /// their legacy JPG is intentionally not migrated; keeping these apart
    /// from `unmatched_filenames` stops "deliberately skipped" from being
    /// reported as "needs manual attention".
    pub skipped_online_filenames: Vec<String>,
}

/// Matches legacy thumbnail filenames (`[video filename].#<hash>.jpg`)
/// against `.wb` `movie` rows' thumbnail hashes.
///
/// `movies` is `(video_id, thumbnail_hash)` pairs, `thumbnail_hash` being an
/// 8-hex-digit string as produced by `wb_row_to_import_candidate`'s `Some`
/// case. `filenames` is every filename (no path, extension included) found
/// in the legacy thumbnail folder.
///
/// Each filename's hash is extracted with `extract_thumbnail_hash` and
/// compared against every entry in `movies`; a filename with no extractable
/// hash, or whose hash matches no `movies` entry, lands in
/// `unmatched_filenames` instead. Duplicate hashes within `movies` do not
/// panic -- the filename is simply matched to every video sharing that hash.
///
/// `online_hashes` are the thumbnail hashes of online videos. A filename that
/// matches no `movies` entry but whose hash is in `online_hashes` goes to
/// `skipped_online_filenames` rather than `unmatched_filenames`. A match in
/// `movies` takes precedence if a hash is (unexpectedly) in both.
pub fn match_thumbnail_files(
    movies: &[(String, String)],
    online_hashes: &[String],
    filenames: &[String],
) -> ThumbnailLinkPlan {
    let mut plan = ThumbnailLinkPlan::default();
    for filename in filenames {
        let Some(hash) = extract_thumbnail_hash(filename) else {
            plan.unmatched_filenames.push(filename.clone());
            continue;
        };
        let mut matched_any = false;
        for (video_id, movie_hash) in movies {
            if *movie_hash == hash {
                plan.matched.push((video_id.clone(), filename.clone()));
                matched_any = true;
            }
        }
        if matched_any {
            continue;
        }
        if online_hashes.contains(&hash) {
            plan.skipped_online_filenames.push(filename.clone());
        } else {
            plan.unmatched_filenames.push(filename.clone());
        }
    }
    plan
}

#[derive(thiserror::Error, Debug, Clone, PartialEq, Eq)]
pub enum WbImportError {
    #[error("invalid datetime '{0}': expected 'YYYY-MM-DD HH:MM:SS'")]
    InvalidDatetime(String),
}

/// Validates a `.wb` datetime string has the `YYYY-MM-DD HH:MM:SS` shape
/// confirmed against real data, and returns it as-is. Not parsed into a
/// structured date type: the rest of the codebase
/// keeps SQLite `DATETIME` columns as plain TEXT (see db/migrations.rs), and
/// there is no chrono/time dependency in this workspace to add one for.
pub fn parse_wb_datetime(s: &str) -> Result<String, WbImportError> {
    let bytes = s.as_bytes();
    let separators_ok = bytes.len() == 19
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b' '
        && bytes[13] == b':'
        && bytes[16] == b':';
    let digits_ok = separators_ok
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| matches!(i, 4 | 7 | 10 | 13 | 16) || b.is_ascii_digit());

    if digits_ok {
        Ok(s.to_string())
    } else {
        Err(WbImportError::InvalidDatetime(s.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportCandidate {
    pub movie_name: String,
    pub movie_path: String,
    pub tags: Vec<String>,
    pub rating: u8,
    /// True when `rating` came from a star-only tag rather than `score`.
    pub rating_from_star_tags: bool,
    /// `None` for the small number of real rows with an empty `hash`
    /// (a known gap in the source data: 1 of 3072 rows) -- these import
    /// without a thumbnail rather than failing.
    pub thumbnail_hash: Option<String>,
    pub kana: String,
    pub roma: String,
    pub file_date: String,
    pub regist_date: String,
    pub last_date: String,
}

/// Converts one `.wb` `movie` row into an import candidate.
pub fn wb_row_to_import_candidate(row: &WbMovieRow) -> Result<ImportCandidate, WbImportError> {
    // A star tag wins over `score`: the user typed it deliberately, whereas
    // `score` may be a leftover default.
    let (tags, star_rating) = split_tags_and_star_rating(&row.tag);
    Ok(ImportCandidate {
        movie_name: row.movie_name.clone(),
        movie_path: row.movie_path.clone(),
        tags,
        rating: star_rating.unwrap_or_else(|| parse_score_to_rating(row.score)),
        rating_from_star_tags: star_rating.is_some(),
        thumbnail_hash: (!row.hash.is_empty()).then(|| row.hash.clone()),
        kana: row.kana.clone(),
        roma: row.roma.clone(),
        file_date: parse_wb_datetime(&row.file_date)?,
        regist_date: parse_wb_datetime(&row.regist_date)?,
        last_date: parse_wb_datetime(&row.last_date)?,
    })
}

/// Counts rows with `score > 5` that were therefore clamped to a 5-star
/// rating, for the import log.
pub fn count_clamped_scores(rows: &[WbMovieRow]) -> usize {
    rows.iter().filter(|r| r.score > 5).count()
}

/// Total number of individual `.wb` tags present in the raw source data
/// (every row's `tag` column, split via `split_tags`), regardless of
/// whether the row's video was ultimately inserted or skipped (already
/// registered) by `import_wb_video`.
///
/// The import result dialog needs to tell "元データにタグが無かった" apart from "移行に
/// 失敗した" when `WbImportSummary::tags_assigned == 0`. `tags_assigned`
/// alone can't distinguish those (nor a third case: every row was already
/// registered, so nothing new was written at all) -- this count gives the
/// dialog the other half of that comparison: `source_tag_count == 0` means
/// the source `.wb` genuinely never had tag data to migrate in the first
/// place, independent of what actually got written.
///
/// Star-only tags are excluded because they become a rating, never a
/// `video_tags` row; counting them would make `tags_assigned` fall short of
/// this number and trigger the dialog's "要確認" warning for a correct import.
pub fn count_source_tags(rows: &[WbMovieRow]) -> usize {
    rows.iter()
        .map(|r| split_tags_and_star_rating(&r.tag).0.len())
        .sum()
}

/// The phases an import passes through, in order. This is the single
/// definition of their names: the frontend step list and the import log share
/// `as_str`, so they cannot drift apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WbImportStage {
    /// Reading the `.wb` file.
    Reading,
    /// Registering each movie row as a video.
    Registering,
    /// Linking legacy thumbnails to the registered videos.
    Linking,
}

impl WbImportStage {
    /// Stable identifier; also the `wb_import:stage` event payload.
    pub fn as_str(self) -> &'static str {
        match self {
            WbImportStage::Reading => "reading",
            WbImportStage::Registering => "registering",
            WbImportStage::Linking => "linking",
        }
    }

    /// Name shown to the user; the import log's section headings.
    pub fn label(self) -> &'static str {
        match self {
            WbImportStage::Reading => "読み込み",
            WbImportStage::Registering => "動画の登録",
            WbImportStage::Linking => "旧サムネイルの紐付け",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_names_are_distinct_and_stable() {
        assert_eq!(WbImportStage::Reading.as_str(), "reading");
        assert_eq!(WbImportStage::Registering.as_str(), "registering");
        assert_eq!(WbImportStage::Linking.as_str(), "linking");
    }

    fn row(overrides: impl FnOnce(&mut WbMovieRow)) -> WbMovieRow {
        let mut row = WbMovieRow {
            movie_id: 1,
            movie_name: "movie.mp4".to_string(),
            movie_path: "T:\\videos\\movie.mp4".to_string(),
            tag: String::new(),
            score: 0,
            hash: "1e5e0fbf".to_string(),
            kana: String::new(),
            roma: String::new(),
            file_date: "2011-05-04 12:00:00".to_string(),
            regist_date: "2011-05-04 12:00:00".to_string(),
            last_date: "2011-05-04 12:00:00".to_string(),
        };
        overrides(&mut row);
        row
    }

    #[test]
    fn parse_score_to_rating_leaves_zero_to_five_untouched() {
        for score in 0..=5 {
            assert_eq!(parse_score_to_rating(score), score as u8);
        }
    }

    #[test]
    fn parse_score_to_rating_clamps_six_and_above_to_five() {
        for score in [6, 12, 19, 20] {
            assert_eq!(parse_score_to_rating(score), 5);
        }
    }

    #[test]
    fn parse_score_to_rating_clamps_negative_to_zero() {
        assert_eq!(parse_score_to_rating(-1), 0);
    }

    #[test]
    fn split_tags_returns_empty_vec_for_empty_string() {
        assert_eq!(split_tags(""), Vec::<String>::new());
    }

    #[test]
    fn split_tags_splits_on_newline() {
        assert_eq!(split_tags("foo\nbar\nbaz"), vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn split_tags_drops_blank_lines_from_trailing_or_doubled_newlines() {
        assert_eq!(split_tags("foo\n\nbar\n"), vec!["foo", "bar"]);
    }

    #[test]
    fn extract_thumbnail_hash_matches_expected_pattern() {
        assert_eq!(
            extract_thumbnail_hash("MyVideo.mp4.#1e5e0fbf.jpg"),
            Some("1e5e0fbf".to_string())
        );
    }

    #[test]
    fn extract_thumbnail_hash_rejects_wrong_length() {
        assert_eq!(extract_thumbnail_hash("MyVideo.mp4.#1e5e0f.jpg"), None);
    }

    #[test]
    fn extract_thumbnail_hash_rejects_uppercase_hex() {
        assert_eq!(extract_thumbnail_hash("MyVideo.mp4.#1E5E0FBF.jpg"), None);
    }

    #[test]
    fn extract_thumbnail_hash_rejects_missing_marker() {
        assert_eq!(extract_thumbnail_hash("MyVideo.mp4.jpg"), None);
    }

    #[test]
    fn parse_wb_datetime_accepts_the_real_data_format() {
        assert_eq!(
            parse_wb_datetime("2011-05-04 12:34:56").unwrap(),
            "2011-05-04 12:34:56"
        );
    }

    #[test]
    fn parse_wb_datetime_rejects_wrong_shape() {
        assert!(parse_wb_datetime("2011/05/04 12:34:56").is_err());
        assert!(parse_wb_datetime("2011-05-04").is_err());
        assert!(parse_wb_datetime("").is_err());
    }

    #[test]
    fn wb_row_to_import_candidate_converts_all_fields() {
        let r = row(|r| {
            r.tag = "foo\nbar".to_string();
            r.score = 12;
        });
        let candidate = wb_row_to_import_candidate(&r).unwrap();
        assert_eq!(candidate.movie_name, "movie.mp4");
        assert_eq!(candidate.tags, vec!["foo", "bar"]);
        assert_eq!(candidate.rating, 5);
        assert_eq!(candidate.thumbnail_hash, Some("1e5e0fbf".to_string()));
    }

    #[test]
    fn wb_row_to_import_candidate_maps_empty_hash_to_none() {
        let r = row(|r| r.hash = String::new());
        let candidate = wb_row_to_import_candidate(&r).unwrap();
        assert_eq!(candidate.thumbnail_hash, None);
    }

    #[test]
    fn wb_row_to_import_candidate_propagates_datetime_errors() {
        let r = row(|r| r.regist_date = "not-a-date".to_string());
        assert!(wb_row_to_import_candidate(&r).is_err());
    }

    #[test]
    fn count_clamped_scores_counts_only_scores_above_five() {
        let rows = vec![
            row(|r| r.score = 0),
            row(|r| r.score = 5),
            row(|r| r.score = 6),
            row(|r| r.score = 20),
        ];
        assert_eq!(count_clamped_scores(&rows), 2);
    }

    #[test]
    fn count_source_tags_sums_split_tags_across_every_row() {
        let rows = vec![
            row(|r| r.tag = "foo\nbar".to_string()), // 2
            row(|r| r.tag = String::new()),          // 0
            row(|r| r.tag = "baz".to_string()),      // 1
        ];
        assert_eq!(count_source_tags(&rows), 3);
    }

    #[test]
    fn count_source_tags_is_zero_for_an_all_untagged_source() {
        let rows = vec![
            row(|r| r.tag = String::new()),
            row(|r| r.tag = String::new()),
        ];
        assert_eq!(count_source_tags(&rows), 0);
    }

    #[test]
    fn count_source_tags_counts_blank_lines_as_dropped_not_as_tags() {
        // Mirrors split_tags_drops_blank_lines_from_trailing_or_doubled_newlines
        // below -- a trailing/doubled newline must not inflate the count.
        let rows = vec![row(|r| r.tag = "foo\n\nbar\n".to_string())];
        assert_eq!(count_source_tags(&rows), 2);
    }

    fn star_row(tag: &str, score: i64) -> WbMovieRow {
        row(|r| {
            r.tag = tag.to_string();
            r.score = score;
        })
    }

    #[test]
    fn star_tag_rating_counts_stars_and_caps_at_five() {
        assert_eq!(star_tag_rating("★"), Some(1));
        assert_eq!(star_tag_rating("★★★"), Some(3));
        assert_eq!(star_tag_rating("★★★★★"), Some(5));
        assert_eq!(star_tag_rating("★★★★★★"), Some(5));
        assert_eq!(star_tag_rating("★★★★★★★"), Some(5));
    }

    #[test]
    fn star_tag_rating_tolerates_surrounding_whitespace() {
        assert_eq!(star_tag_rating("  ★★ "), Some(2));
        assert_eq!(star_tag_rating("\u{3000}★★★\u{3000}"), Some(3));
    }

    #[test]
    fn star_tag_rating_rejects_non_star_only_tags() {
        assert_eq!(star_tag_rating("☆"), None);
        assert_eq!(star_tag_rating("★お気に入り"), None);
        assert_eq!(star_tag_rating("★ ★"), None);
        assert_eq!(star_tag_rating("★☆"), None);
        assert_eq!(star_tag_rating(""), None);
        assert_eq!(star_tag_rating("   "), None);
    }

    #[test]
    fn candidate_converts_star_tag_to_rating_and_removes_it_from_tags() {
        let c = wb_row_to_import_candidate(&star_row("★★★\nfoo", 0)).unwrap();
        assert_eq!(c.rating, 3);
        assert!(c.rating_from_star_tags);
        assert_eq!(c.tags, vec!["foo"]);
    }

    #[test]
    fn candidate_caps_star_tag_rating_at_five() {
        let c = wb_row_to_import_candidate(&star_row("★★★★★★★", 0)).unwrap();
        assert_eq!(c.rating, 5);
        assert!(c.tags.is_empty());
    }

    #[test]
    fn candidate_keeps_hollow_star_and_mixed_tags_as_ordinary_tags() {
        let c = wb_row_to_import_candidate(&star_row("☆\n★お気に入り", 2)).unwrap();
        assert_eq!(c.tags, vec!["☆", "★お気に入り"]);
        assert_eq!(c.rating, 2);
        assert!(!c.rating_from_star_tags);
    }

    #[test]
    fn candidate_uses_max_of_multiple_star_tags() {
        let c = wb_row_to_import_candidate(&star_row("★\n★★★\n★★", 0)).unwrap();
        assert_eq!(c.rating, 3);
        assert!(c.tags.is_empty());
    }

    #[test]
    fn candidate_prefers_star_tag_over_score() {
        let c = wb_row_to_import_candidate(&star_row("★★", 5)).unwrap();
        assert_eq!(c.rating, 2);
        assert!(c.rating_from_star_tags);
    }

    #[test]
    fn candidate_without_star_tag_uses_score() {
        let c = wb_row_to_import_candidate(&star_row("foo", 4)).unwrap();
        assert_eq!(c.rating, 4);
        assert!(!c.rating_from_star_tags);
    }

    #[test]
    fn count_source_tags_excludes_star_only_tags() {
        let rows = vec![star_row("★★\nfoo", 0), star_row("★", 0)];
        assert_eq!(count_source_tags(&rows), 1);
    }

    #[test]
    fn wb_tag_merge_key_folds_case() {
        assert_eq!(wb_tag_merge_key("Action"), wb_tag_merge_key("action"));
        assert_eq!(wb_tag_merge_key("Action"), "action");
    }

    #[test]
    fn wb_tag_merge_key_folds_full_width_characters() {
        // Full-width "A", "c", "t", "i", "o", "n" (U+FF21/FF43/FF54/FF29(I)/FF2F/FF2E)
        // should fold and case-fold to match the plain ASCII "Action".
        let full_width = "\u{FF21}\u{FF43}\u{FF54}\u{FF49}\u{FF4F}\u{FF4E}";
        assert_eq!(wb_tag_merge_key(full_width), wb_tag_merge_key("Action"));
        assert_eq!(wb_tag_merge_key(full_width), "action");
    }

    #[test]
    fn wb_tag_merge_key_trims_whitespace() {
        assert_eq!(wb_tag_merge_key("  Action  "), "action");
        assert_eq!(
            wb_tag_merge_key("\u{3000}Action\u{3000}"),
            wb_tag_merge_key("Action")
        );
    }

    #[test]
    fn wb_tag_merge_key_leaves_japanese_text_unchanged_apart_from_trim() {
        assert_eq!(wb_tag_merge_key("アクション"), "アクション");
        assert_eq!(wb_tag_merge_key("コメディ映画"), "コメディ映画");
        assert_eq!(wb_tag_merge_key("  ひらがな  "), "ひらがな");
    }

    #[test]
    fn match_thumbnail_files_matches_by_hash() {
        let movies = vec![("id1".to_string(), "1e5e0fbf".to_string())];
        let filenames = vec!["MyVideo.mp4.#1e5e0fbf.jpg".to_string()];
        let plan = match_thumbnail_files(&movies, &[], &filenames);
        assert_eq!(
            plan.matched,
            vec![("id1".to_string(), "MyVideo.mp4.#1e5e0fbf.jpg".to_string())]
        );
        assert!(plan.unmatched_filenames.is_empty());
    }

    #[test]
    fn match_thumbnail_files_leaves_hash_mismatch_unmatched() {
        let movies = vec![("id1".to_string(), "aaaaaaaa".to_string())];
        let filenames = vec!["MyVideo.mp4.#1e5e0fbf.jpg".to_string()];
        let plan = match_thumbnail_files(&movies, &[], &filenames);
        assert!(plan.matched.is_empty());
        assert_eq!(
            plan.unmatched_filenames,
            vec!["MyVideo.mp4.#1e5e0fbf.jpg".to_string()]
        );
    }

    #[test]
    fn match_thumbnail_files_leaves_malformed_filenames_unmatched() {
        let movies = vec![("id1".to_string(), "1e5e0fbf".to_string())];
        let filenames = vec!["not_a_thumbnail.jpg".to_string()];
        let plan = match_thumbnail_files(&movies, &[], &filenames);
        assert!(plan.matched.is_empty());
        assert_eq!(
            plan.unmatched_filenames,
            vec!["not_a_thumbnail.jpg".to_string()]
        );
    }

    #[test]
    fn match_thumbnail_files_does_not_panic_on_duplicate_hashes_in_movies() {
        let movies = vec![
            ("id1".to_string(), "1e5e0fbf".to_string()),
            ("id2".to_string(), "1e5e0fbf".to_string()),
        ];
        let filenames = vec!["MyVideo.mp4.#1e5e0fbf.jpg".to_string()];
        let plan = match_thumbnail_files(&movies, &[], &filenames);
        assert_eq!(
            plan.matched,
            vec![
                ("id1".to_string(), "MyVideo.mp4.#1e5e0fbf.jpg".to_string()),
                ("id2".to_string(), "MyVideo.mp4.#1e5e0fbf.jpg".to_string()),
            ]
        );
        assert!(plan.unmatched_filenames.is_empty());
    }

    #[test]
    fn match_thumbnail_files_separates_online_hash_from_unmatched() {
        let movies = vec![("id1".to_string(), "aaaaaaaa".to_string())];
        let online = vec!["bbbbbbbb".to_string()];
        let filenames = vec![
            "Off.mp4.#aaaaaaaa.jpg".to_string(),
            "On.mp4.#bbbbbbbb.jpg".to_string(),
            "Other.mp4.#cccccccc.jpg".to_string(),
            "Thumbs.db".to_string(),
        ];
        let plan = match_thumbnail_files(&movies, &online, &filenames);
        assert_eq!(
            plan.matched,
            vec![("id1".to_string(), "Off.mp4.#aaaaaaaa.jpg".to_string())]
        );
        assert_eq!(
            plan.skipped_online_filenames,
            vec!["On.mp4.#bbbbbbbb.jpg".to_string()]
        );
        assert_eq!(
            plan.unmatched_filenames,
            vec![
                "Other.mp4.#cccccccc.jpg".to_string(),
                "Thumbs.db".to_string()
            ]
        );
    }

    #[test]
    fn match_thumbnail_files_prefers_offline_match_over_online_skip() {
        let movies = vec![("id1".to_string(), "aaaaaaaa".to_string())];
        let online = vec!["aaaaaaaa".to_string()];
        let filenames = vec!["V.mp4.#aaaaaaaa.jpg".to_string()];
        let plan = match_thumbnail_files(&movies, &online, &filenames);
        assert_eq!(plan.matched.len(), 1);
        assert!(plan.skipped_online_filenames.is_empty());
        assert!(plan.unmatched_filenames.is_empty());
    }
}
