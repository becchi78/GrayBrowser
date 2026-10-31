//! The part of a `.wb` row kept alongside an unregistered-file record, so that
//! registering the record later (when the file itself is gone) can still
//! restore the user's rating, tags and readings. Stored as JSON in
//! `skipped_files.wb_payload`.

use serde::{Deserialize, Serialize};

use crate::wb_import::ImportCandidate;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WbSkippedPayload {
    pub rating: u8,
    /// True when `rating` came from a star-only tag rather than `score`.
    pub rating_from_star_tags: bool,
    pub kana: String,
    pub roma: String,
    /// Raw `.wb` tag strings, in the form `import_wb_video` takes them.
    pub tags: Vec<String>,
}

impl WbSkippedPayload {
    pub fn from_candidate(candidate: &ImportCandidate) -> Self {
        Self {
            rating: candidate.rating,
            rating_from_star_tags: candidate.rating_from_star_tags,
            kana: candidate.kana.clone(),
            roma: candidate.roma.clone(),
            tags: candidate.tags.clone(),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("plain strings and numbers always serialize")
    }

    /// `None` for malformed JSON: a damaged payload must degrade to "no .wb
    /// information" rather than block registering the record.
    pub fn from_json(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> WbSkippedPayload {
        WbSkippedPayload {
            rating: 4,
            rating_from_star_tags: true,
            kana: "かな".to_string(),
            roma: "kana".to_string(),
            tags: vec!["a".to_string(), "日本語\"quote".to_string()],
        }
    }

    #[test]
    fn round_trips_through_json() {
        let p = sample();
        assert_eq!(WbSkippedPayload::from_json(&p.to_json()), Some(p));
    }

    #[test]
    fn empty_values_round_trip() {
        let p = WbSkippedPayload {
            rating: 0,
            rating_from_star_tags: false,
            kana: String::new(),
            roma: String::new(),
            tags: vec![],
        };
        assert_eq!(WbSkippedPayload::from_json(&p.to_json()), Some(p));
    }

    #[test]
    fn malformed_json_is_none() {
        assert_eq!(WbSkippedPayload::from_json("{not json"), None);
        assert_eq!(WbSkippedPayload::from_json(""), None);
        assert_eq!(WbSkippedPayload::from_json("{\"rating\":300}"), None);
    }
}
