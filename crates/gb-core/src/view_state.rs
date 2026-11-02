//! The search/folder/sort/filter state restored at the next launch. Pure
//! parsing and clean-up only; storage lives in `src-tauri`.

use serde::{Deserialize, Serialize};

use crate::rating::MAX_RATING;

pub const DEFAULT_SORT_FIELD: &str = "created_at";
pub const DEFAULT_SORT_DIRECTION: &str = "asc";

/// Must stay in step with the frontend's `SortField` values.
const SORT_FIELDS: [&str; 6] = [
    "file_name",
    "created_at",
    "updated_date",
    "rating",
    "file_size",
    "folder",
];
const SORT_DIRECTIONS: [&str; 2] = ["asc", "desc"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedViewState {
    pub search: String,
    /// `None` is "all folders".
    pub folder_path: Option<String>,
    pub sort_field: String,
    pub sort_direction: String,
    pub tag_ids: Vec<i64>,
    pub ratings: Vec<i64>,
}

impl Default for SavedViewState {
    fn default() -> Self {
        Self {
            search: String::new(),
            folder_path: None,
            sort_field: DEFAULT_SORT_FIELD.to_string(),
            sort_direction: DEFAULT_SORT_DIRECTION.to_string(),
            tag_ids: Vec::new(),
            ratings: Vec::new(),
        }
    }
}

/// Drops whatever no longer resolves, field by field, so one stale value never
/// discards the rest: an unregistered folder falls back to "all", deleted tags
/// are removed, unknown sort values and out-of-range ratings are reset or
/// dropped.
pub fn sanitize(
    saved: SavedViewState,
    watch_folders: &[String],
    existing_tag_ids: &[i64],
) -> SavedViewState {
    let folder_path = saved
        .folder_path
        .filter(|f| watch_folders.iter().any(|w| w == f));
    let sort_field = if SORT_FIELDS.contains(&saved.sort_field.as_str()) {
        saved.sort_field
    } else {
        DEFAULT_SORT_FIELD.to_string()
    };
    let sort_direction = if SORT_DIRECTIONS.contains(&saved.sort_direction.as_str()) {
        saved.sort_direction
    } else {
        DEFAULT_SORT_DIRECTION.to_string()
    };
    let tag_ids = dedup_keep_order(
        saved
            .tag_ids
            .into_iter()
            .filter(|id| existing_tag_ids.contains(id)),
    );
    let ratings = dedup_keep_order(
        saved
            .ratings
            .into_iter()
            .filter(|r| (0..=i64::from(MAX_RATING)).contains(r)),
    );
    SavedViewState {
        search: saved.search,
        folder_path,
        sort_field,
        sort_direction,
        tag_ids,
        ratings,
    }
}

fn dedup_keep_order(values: impl Iterator<Item = i64>) -> Vec<i64> {
    let mut out = Vec::new();
    for v in values {
        if !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

/// A missing or corrupt value reads as the default state: a hand-edited or
/// half-written value must never block startup.
pub fn parse_saved(raw: Option<&str>) -> SavedViewState {
    raw.and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders() -> Vec<String> {
        vec!["/a".to_string(), "/b".to_string()]
    }

    fn sample() -> SavedViewState {
        SavedViewState {
            search: "abc".into(),
            folder_path: Some("/a".into()),
            sort_field: "rating".into(),
            sort_direction: "desc".into(),
            tag_ids: vec![1, 2],
            ratings: vec![3, 5],
        }
    }

    #[test]
    fn valid_state_is_kept_as_is() {
        assert_eq!(sanitize(sample(), &folders(), &[1, 2, 3]), sample());
    }

    #[test]
    fn unregistered_folder_falls_back_to_all_without_touching_other_fields() {
        let mut s = sample();
        s.folder_path = Some("/gone".into());
        let out = sanitize(s, &folders(), &[1, 2]);
        assert_eq!(out.folder_path, None);
        assert_eq!(out.search, "abc");
        assert_eq!(out.tag_ids, vec![1, 2]);
    }

    #[test]
    fn deleted_tags_are_removed() {
        let out = sanitize(sample(), &folders(), &[2]);
        assert_eq!(out.tag_ids, vec![2]);
        assert!(sanitize(sample(), &folders(), &[]).tag_ids.is_empty());
    }

    #[test]
    fn unknown_sort_values_reset_to_default() {
        let mut s = sample();
        s.sort_field = "bogus".into();
        s.sort_direction = "sideways".into();
        let out = sanitize(s, &folders(), &[1, 2]);
        assert_eq!(out.sort_field, DEFAULT_SORT_FIELD);
        assert_eq!(out.sort_direction, DEFAULT_SORT_DIRECTION);
    }

    #[test]
    fn every_known_sort_field_survives() {
        for f in SORT_FIELDS {
            let mut s = sample();
            s.sort_field = f.into();
            assert_eq!(sanitize(s, &folders(), &[]).sort_field, f);
        }
    }

    #[test]
    fn out_of_range_ratings_are_dropped_and_boundaries_kept() {
        let mut s = sample();
        s.ratings = vec![-1, 0, 5, 6, 5];
        assert_eq!(sanitize(s, &folders(), &[]).ratings, vec![0, 5]);
    }

    #[test]
    fn duplicate_tag_ids_collapse() {
        let mut s = sample();
        s.tag_ids = vec![2, 1, 2];
        assert_eq!(sanitize(s, &folders(), &[1, 2]).tag_ids, vec![2, 1]);
    }

    #[test]
    fn missing_or_broken_json_reads_as_default() {
        assert_eq!(parse_saved(None), SavedViewState::default());
        assert_eq!(parse_saved(Some("{not json")), SavedViewState::default());
        assert_eq!(parse_saved(Some("")), SavedViewState::default());
        assert_eq!(
            parse_saved(Some(r#"{"search":1}"#)),
            SavedViewState::default()
        );
    }

    #[test]
    fn partial_json_fills_the_rest_with_defaults() {
        let s = parse_saved(Some(r#"{"search":"x"}"#));
        assert_eq!(s.search, "x");
        assert_eq!(s.sort_field, DEFAULT_SORT_FIELD);
        assert_eq!(s.folder_path, None);
    }

    #[test]
    fn round_trips_through_json() {
        let json = serde_json::to_string(&sample()).unwrap();
        assert_eq!(parse_saved(Some(&json)), sample());
    }
}
