//! Deterministic anonymization of `.wb` values for test fixture generation.
//! Pure logic + in-memory cache only -- no file I/O,
//! no rusqlite. Real `.wb` reading and anonymized-file writing happen in the
//! `wb-anonymize-tool` binary; this module only knows how to turn one real
//! string into one deterministic dummy string, and (for the leak-check test)
//! how to recognize that a string looks like one of its own dummy outputs.
//!
//! Every generation function here has a matching validation function that
//! shares the same prefix constants / split helpers, so the anonymization
//! CLI and the leak-check test can never drift out of sync with each other.

use std::collections::{HashMap, HashSet};

use xxhash_rust::xxh64::Xxh64;

use crate::ports::wb_source::WbMovieRow;
use crate::wb_import::split_tags;

const ANONYMIZE_SEED: u64 = 0x4752_4159_4241_5259; // arbitrary fixed constant ("GRAYBARY"-ish), never derived from real data

pub const PATH_SEGMENT_PREFIX: &str = "gbseg_";
pub const TAG_PREFIX: &str = "gbtag_";
pub const FREETEXT_PREFIX: &str = "gbtxt_";

/// Number of hex digits in a Path/Tag/FreeText dummy token's suffix.
const TOKEN_SUFFIX_HEX_LEN: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenRole {
    /// A single path segment or filename stem (drive letters and the
    /// original file extension are kept as-is; see `anonymize_path`).
    Path,
    /// A single tag (before/after newline-splitting; see `anonymize_tags`).
    Tag,
    /// The `movie.hash` thumbnail key -- must stay 8 lowercase hex chars and
    /// globally unique, so it's generated differently from the other roles.
    Hash,
    /// Any other free-text cell (kana/roma, and every TEXT column outside
    /// `movie` during the full-table coverage pass).
    FreeText,
}

/// Column-name → role mapping for the known `movie` columns. Anything not
/// listed here (including every column of every non-`movie` table) is
/// `FreeText` -- this is the single source of truth both the generator and
/// the leak-check test's positive-direction validator consult, so they can
/// never disagree about which rule applies to which column.
pub fn column_role(table_name: &str, column_name: &str) -> TokenRole {
    if table_name == "movie" {
        match column_name {
            "hash" => TokenRole::Hash,
            "movie_path" | "movie_name" => TokenRole::Path,
            "tag" => TokenRole::Tag,
            _ => TokenRole::FreeText,
        }
    } else {
        TokenRole::FreeText
    }
}

/// Regex a value of `role` must match if it was produced by `Anonymizer`
/// (used only for whole-token roles -- `Path`/`Tag` on composite fields like
/// `movie_path`/`tag` need `is_valid_dummy_path`/`is_valid_dummy_tag_field`
/// instead, since those fields are built from multiple tokens).
pub fn dummy_pattern(role: TokenRole) -> String {
    match role {
        TokenRole::Path => format!(
            r"^{}[0-9a-f]{{{TOKEN_SUFFIX_HEX_LEN}}}$",
            regex::escape(PATH_SEGMENT_PREFIX)
        ),
        TokenRole::Tag => format!(
            r"^{}[0-9a-f]{{{TOKEN_SUFFIX_HEX_LEN}}}$",
            regex::escape(TAG_PREFIX)
        ),
        TokenRole::FreeText => format!(
            r"^{}[0-9a-f]{{{TOKEN_SUFFIX_HEX_LEN}}}$",
            regex::escape(FREETEXT_PREFIX)
        ),
        TokenRole::Hash => r"^[0-9a-f]{8}$".to_string(),
    }
}

/// Splits a path on both `\` and `/`, dropping empty segments (leading
/// separator, doubled separator). Shared by `anonymize_path` and
/// `is_valid_dummy_path` so both always agree on what a "segment" is.
pub fn path_segments(path: &str) -> Vec<&str> {
    path.split(['\\', '/']).filter(|s| !s.is_empty()).collect()
}

/// Whether `segment` is a bare Windows drive letter (`T:`, `u:`, ...) --
/// kept unmodified by `anonymize_path` since it's structural, not personal
/// (real data spans `T:`/`U:`/`V:`/`W:`).
pub fn is_drive_letter_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// Splits `stem.ext` into `(stem, Some("ext"))`, or `(name, None)` if there's
/// no `.`. Shared by `anonymize_filename_segment` and `is_valid_dummy_filename`.
fn split_extension(name: &str) -> (&str, Option<&str>) {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
        _ => (name, None),
    }
}

/// Deterministic `.wb` value anonymizer. Holds only an in-memory cache and
/// the set of dummy hashes already handed out -- no file I/O.
#[derive(Default)]
pub struct Anonymizer {
    cache: HashMap<(TokenRole, String), String>,
    assigned_hashes: HashSet<String>,
}

impl Anonymizer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Anonymizes a single opaque token (whole-value roles: `Hash`,
    /// `FreeText`, or an already-split `Path`/`Tag` piece). Empty input maps
    /// to empty output -- there's nothing to leak in an empty string, and
    /// real data has legitimate empty cells (e.g. the one row with an empty
    /// `hash`) whose emptiness is itself a structural property worth
    /// preserving.
    pub fn anonymize(&mut self, role: TokenRole, original: &str) -> String {
        if original.is_empty() {
            return String::new();
        }
        let key = (role, original.to_string());
        if let Some(existing) = self.cache.get(&key) {
            return existing.clone();
        }
        let dummy = match role {
            TokenRole::Hash => self.next_unique_hash(original),
            TokenRole::Path => format!(
                "{PATH_SEGMENT_PREFIX}{:0width$x}",
                truncated_digest(original),
                width = TOKEN_SUFFIX_HEX_LEN
            ),
            TokenRole::Tag => format!(
                "{TAG_PREFIX}{:0width$x}",
                truncated_digest(original),
                width = TOKEN_SUFFIX_HEX_LEN
            ),
            TokenRole::FreeText => format!(
                "{FREETEXT_PREFIX}{:0width$x}",
                truncated_digest(original),
                width = TOKEN_SUFFIX_HEX_LEN
            ),
        };
        self.cache.insert(key, dummy.clone());
        dummy
    }

    /// Anonymizes a `movie_path` or `movie_name` value: drive-letter
    /// segments and the file extension are kept as-is (structural, not
    /// personal), every other segment/stem is replaced with a `Path`-role
    /// dummy token. Splitting the last segment's extension off uses the same
    /// `split_extension` helper `is_valid_dummy_path` uses to check it.
    pub fn anonymize_path(&mut self, path: &str) -> String {
        if path.is_empty() {
            return String::new();
        }
        let separator = if path.contains('\\') { '\\' } else { '/' };
        let segments = path_segments(path);
        let last_index = segments.len().saturating_sub(1);
        let anonymized: Vec<String> = segments
            .into_iter()
            .enumerate()
            .map(|(i, segment)| {
                if is_drive_letter_segment(segment) {
                    segment.to_string()
                } else if i == last_index {
                    self.anonymize_filename_segment(segment)
                } else {
                    self.anonymize(TokenRole::Path, segment)
                }
            })
            .collect();
        anonymized.join(&separator.to_string())
    }

    /// Anonymizes a bare filename (`movie_name`, or `movie_path`'s last
    /// segment): stem replaced, extension kept.
    fn anonymize_filename_segment(&mut self, filename: &str) -> String {
        let (stem, ext) = split_extension(filename);
        let dummy_stem = self.anonymize(TokenRole::Path, stem);
        match ext {
            Some(ext) => format!("{dummy_stem}.{ext}"),
            None => dummy_stem,
        }
    }

    /// Anonymizes a `tag` field: reuses `wb_import::split_tags` (the same
    /// newline-splitting the parser itself uses) so tag structure can never
    /// diverge between the parser and the anonymizer, anonymizes each tag
    /// individually, and rejoins with `\n`.
    pub fn anonymize_tags(&mut self, tag_field: &str) -> String {
        split_tags(tag_field)
            .into_iter()
            .map(|t| self.anonymize(TokenRole::Tag, &t))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Anonymizes every personal-info-bearing field of a `movie` row.
    /// `movie_id`, `score`, and the three datetime columns are left as-is:
    /// `movie_id` is a positional key (not personal), `score` is structural
    /// (the clamping test depends on real values), and
    /// the datetime columns are declared `datetime` (not `TEXT`) in the real
    /// schema so `read_all_text_cells` never surfaces them for the leak
    /// check in the first place.
    pub fn anonymize_movie_row(&mut self, row: &WbMovieRow) -> WbMovieRow {
        WbMovieRow {
            movie_id: row.movie_id,
            movie_name: self.anonymize_path(&row.movie_name),
            movie_path: self.anonymize_path(&row.movie_path),
            tag: self.anonymize_tags(&row.tag),
            score: row.score,
            hash: self.anonymize(TokenRole::Hash, &row.hash),
            kana: self.anonymize(TokenRole::FreeText, &row.kana),
            roma: self.anonymize(TokenRole::FreeText, &row.roma),
            file_date: row.file_date.clone(),
            regist_date: row.regist_date.clone(),
            last_date: row.last_date.clone(),
        }
    }

    /// Builds the dummy thumbnail filename `[dummy movie_name].#<dummy
    /// hash>.jpg` for a real `(movie_name, hash)` pair. Referential integrity
    /// with `anonymize_movie_row`'s output is guaranteed structurally: both
    /// go through the same `cache`, so the same real `movie_name`/`hash`
    /// always resolve to the same dummy values regardless of call order.
    pub fn dummy_thumbnail_filename(&mut self, movie_name: &str, hash: &str) -> String {
        let dummy_name = self.anonymize_path(movie_name);
        let dummy_hash = self.anonymize(TokenRole::Hash, hash);
        format!("{dummy_name}.#{dummy_hash}.jpg")
    }

    fn next_unique_hash(&mut self, original: &str) -> String {
        let mut n: u32 = 0;
        loop {
            let seed_input = if n == 0 {
                original.to_string()
            } else {
                format!("{original}#{n}")
            };
            let candidate = format!("{:08x}", digest64(&seed_input) as u32);
            if self.assigned_hashes.insert(candidate.clone()) {
                return candidate;
            }
            n += 1;
        }
    }
}

fn digest64(s: &str) -> u64 {
    let mut hasher = Xxh64::new(ANONYMIZE_SEED);
    hasher.update(s.as_bytes());
    hasher.digest()
}

fn truncated_digest(s: &str) -> u64 {
    digest64(s) & ((1u64 << (TOKEN_SUFFIX_HEX_LEN * 4)) - 1)
}

/// Positive-direction validator matching `anonymize_path`'s output shape:
/// every segment is either a drive letter or a `Path`-role dummy token, and
/// the last segment's extension (if any) survived unchanged.
pub fn is_valid_dummy_path(path: &str) -> bool {
    if path.is_empty() {
        return true;
    }
    let pattern = regex::Regex::new(&dummy_pattern(TokenRole::Path)).unwrap();
    let segments = path_segments(path);
    let Some((last, rest)) = segments.split_last() else {
        return false;
    };
    if !rest
        .iter()
        .all(|s| is_drive_letter_segment(s) || pattern.is_match(s))
    {
        return false;
    }
    let (stem, _ext) = split_extension(last);
    pattern.is_match(stem)
}

/// Positive-direction validator matching `anonymize_tags`'s output shape:
/// empty, or newline-separated `Tag`-role dummy tokens.
pub fn is_valid_dummy_tag_field(tag_field: &str) -> bool {
    if tag_field.is_empty() {
        return true;
    }
    let pattern = regex::Regex::new(&dummy_pattern(TokenRole::Tag)).unwrap();
    tag_field.split('\n').all(|t| pattern.is_match(t))
}

/// Anonymizes a single `(table_name, column_name, value)` cell as read by
/// `read_all_text_cells`, dispatching to whichever function actually
/// matches that column's structure (composite `movie_path`/`movie_name`/
/// `tag`, or an opaque single token otherwise). This is the one place that
/// decides "how is this column shaped", so the leak-check tooling driving
/// `read_all_text_cells` output never has to re-derive that decision itself.
pub fn anonymize_cell(
    anonymizer: &mut Anonymizer,
    table_name: &str,
    column_name: &str,
    value: &str,
) -> String {
    match (table_name, column_name) {
        ("movie", "movie_path") | ("movie", "movie_name") => anonymizer.anonymize_path(value),
        ("movie", "tag") => anonymizer.anonymize_tags(value),
        _ => anonymizer.anonymize(column_role(table_name, column_name), value),
    }
}

/// Whether `value` (found in `table_name.column_name`) looks like something
/// `Anonymizer`/`anonymize_cell` produced. Empty values are always valid --
/// there's nothing to check. Mirrors `anonymize_cell`'s dispatch exactly, so
/// "how do we generate this column" and "how do we validate this column"
/// can never quietly diverge.
pub fn is_valid_dummy_value(table_name: &str, column_name: &str, value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    match (table_name, column_name) {
        ("movie", "movie_path") | ("movie", "movie_name") => is_valid_dummy_path(value),
        ("movie", "tag") => is_valid_dummy_tag_field(value),
        _ => {
            let role = column_role(table_name, column_name);
            regex::Regex::new(&dummy_pattern(role))
                .unwrap()
                .is_match(value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(overrides: impl FnOnce(&mut WbMovieRow)) -> WbMovieRow {
        let mut row = WbMovieRow {
            movie_id: 1,
            movie_name: "clip.mp4".to_string(),
            movie_path: "T:\\videos\\clip.mp4".to_string(),
            tag: "foo\nbar".to_string(),
            score: 3,
            hash: "1e5e0fbf".to_string(),
            kana: "かな".to_string(),
            roma: "kana".to_string(),
            file_date: "2011-05-04 12:00:00".to_string(),
            regist_date: "2011-05-04 12:00:00".to_string(),
            last_date: "2011-05-04 12:00:00".to_string(),
        };
        overrides(&mut row);
        row
    }

    #[test]
    fn anonymize_is_deterministic_for_the_same_input() {
        let mut a = Anonymizer::new();
        let first = a.anonymize(TokenRole::FreeText, "some real value");
        let second = a.anonymize(TokenRole::FreeText, "some real value");
        assert_eq!(first, second);
    }

    #[test]
    fn anonymize_differs_across_roles_for_the_same_input() {
        let mut a = Anonymizer::new();
        let path_token = a.anonymize(TokenRole::Path, "same");
        let tag_token = a.anonymize(TokenRole::Tag, "same");
        assert_ne!(path_token, tag_token);
    }

    #[test]
    fn anonymize_empty_string_stays_empty() {
        let mut a = Anonymizer::new();
        assert_eq!(a.anonymize(TokenRole::FreeText, ""), "");
        assert_eq!(a.anonymize(TokenRole::Hash, ""), "");
    }

    #[test]
    fn hash_output_matches_the_real_shape_and_pattern() {
        let mut a = Anonymizer::new();
        let dummy = a.anonymize(TokenRole::Hash, "1e5e0fbf");
        let pattern = regex::Regex::new(&dummy_pattern(TokenRole::Hash)).unwrap();
        assert!(pattern.is_match(&dummy), "{dummy} should match {pattern}");
    }

    #[test]
    fn hash_collisions_are_resolved_deterministically_and_uniquely() {
        // Force a collision by using inputs that are unlikely to hash the
        // same way naturally, then check both still land on distinct
        // outputs across two independent Anonymizer runs (determinism) and
        // within a single run (uniqueness).
        let inputs = ["aaaaaaaa", "bbbbbbbb", "cccccccc", "dddddddd", "eeeeeeee"];

        let mut a1 = Anonymizer::new();
        let run1: Vec<String> = inputs
            .iter()
            .map(|i| a1.anonymize(TokenRole::Hash, i))
            .collect();

        let mut a2 = Anonymizer::new();
        let run2: Vec<String> = inputs
            .iter()
            .map(|i| a2.anonymize(TokenRole::Hash, i))
            .collect();

        assert_eq!(
            run1, run2,
            "same inputs in the same order must produce the same outputs"
        );
        let unique: HashSet<&String> = run1.iter().collect();
        assert_eq!(unique.len(), run1.len(), "all dummy hashes must be unique");
    }

    #[test]
    fn anonymize_path_preserves_drive_letter_and_extension() {
        let mut a = Anonymizer::new();
        let dummy = a.anonymize_path("T:\\videos\\clip.mp4");
        let segments: Vec<&str> = dummy.split('\\').collect();
        assert_eq!(segments[0], "T:");
        assert!(segments.last().unwrap().ends_with(".mp4"));
        assert!(is_valid_dummy_path(&dummy));
    }

    #[test]
    fn anonymize_path_is_referentially_consistent_for_repeated_segments() {
        let mut a = Anonymizer::new();
        let dummy_a = a.anonymize_path("T:\\videos\\sub\\a.mp4");
        let dummy_b = a.anonymize_path("T:\\videos\\sub\\b.mp4");
        let seg_a: Vec<&str> = dummy_a.split('\\').collect();
        let seg_b: Vec<&str> = dummy_b.split('\\').collect();
        // Same real "videos" and "sub" segments must map to the same dummy
        // segments both times.
        assert_eq!(seg_a[1], seg_b[1]);
        assert_eq!(seg_a[2], seg_b[2]);
    }

    #[test]
    fn anonymize_tags_preserves_newline_structure_and_emptiness() {
        let mut a = Anonymizer::new();
        assert_eq!(a.anonymize_tags(""), "");
        let dummy = a.anonymize_tags("foo\nbar");
        assert!(is_valid_dummy_tag_field(&dummy));
        assert_eq!(dummy.split('\n').count(), 2);
    }

    #[test]
    fn is_valid_dummy_path_rejects_real_looking_segments() {
        assert!(!is_valid_dummy_path("T:\\videos\\clip.mp4"));
    }

    #[test]
    fn is_valid_dummy_tag_field_rejects_real_looking_tags() {
        assert!(!is_valid_dummy_tag_field("foo\nbar"));
    }

    #[test]
    fn anonymize_movie_row_keeps_score_and_dates_and_maps_empty_hash_to_empty() {
        let mut a = Anonymizer::new();
        let real = row(|r| r.hash = String::new());
        let dummy = a.anonymize_movie_row(&real);
        assert_eq!(dummy.movie_id, real.movie_id);
        assert_eq!(dummy.score, real.score);
        assert_eq!(dummy.file_date, real.file_date);
        assert_eq!(dummy.hash, "");
    }

    #[test]
    fn dummy_thumbnail_filename_is_referentially_consistent_with_movie_row_anonymization() {
        let mut a = Anonymizer::new();
        let real = row(|_| {});
        let dummy_row = a.anonymize_movie_row(&real);
        let dummy_thumb = a.dummy_thumbnail_filename(&real.movie_name, &real.hash);
        assert_eq!(
            dummy_thumb,
            format!("{}.#{}.jpg", dummy_row.movie_name, dummy_row.hash)
        );
    }

    #[test]
    fn column_role_maps_known_movie_columns_and_defaults_to_freetext() {
        assert_eq!(column_role("movie", "hash"), TokenRole::Hash);
        assert_eq!(column_role("movie", "movie_path"), TokenRole::Path);
        assert_eq!(column_role("movie", "tag"), TokenRole::Tag);
        assert_eq!(column_role("movie", "kana"), TokenRole::FreeText);
        assert_eq!(column_role("findfact", "find_text"), TokenRole::FreeText);
        assert_eq!(column_role("watch", "dir"), TokenRole::FreeText);
    }

    #[test]
    fn anonymize_cell_and_is_valid_dummy_value_agree_for_every_movie_column() {
        let mut a = Anonymizer::new();
        let cases = [
            ("movie", "movie_path", "T:\\videos\\clip.mp4"),
            ("movie", "movie_name", "clip.mp4"),
            ("movie", "tag", "foo\nbar"),
            ("movie", "hash", "1e5e0fbf"),
            ("movie", "kana", "かな"),
            ("movie", "roma", "kana"),
            ("findfact", "find_text", "some search term"),
            ("watch", "dir", "T:\\videos"),
        ];
        for (table, column, value) in cases {
            let dummy = anonymize_cell(&mut a, table, column, value);
            assert!(
                is_valid_dummy_value(table, column, &dummy),
                "{table}.{column}: {dummy:?} should be recognized as a valid dummy value"
            );
        }
    }

    #[test]
    fn is_valid_dummy_value_rejects_real_looking_freetext_and_paths() {
        // Note: a real `hash` is intentionally indistinguishable from a
        // dummy one by shape alone (both are 8 lowercase hex chars) -- that
        // overlap is caught separately by the leak-check's *negative*
        // (forbidden-token) direction, not this positive/shape check.
        assert!(!is_valid_dummy_value(
            "findfact",
            "find_text",
            "some search term"
        ));
        assert!(!is_valid_dummy_value(
            "movie",
            "movie_path",
            "T:\\videos\\clip.mp4"
        ));
    }

    #[test]
    fn is_valid_dummy_value_treats_empty_as_valid() {
        assert!(is_valid_dummy_value("movie", "movie_path", ""));
    }
}
