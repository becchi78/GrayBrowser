//! OS-independent business logic for GrayBrowser.
//!
//! This crate must never depend on `tauri`, `rusqlite`, or any process-spawning
//! crate, and must never contain `#[cfg(windows)]`.

pub mod dedup;
pub mod filename_validation;
pub mod hash;
pub mod migrations;
pub mod paths;
pub mod ports;
pub mod rating;
pub mod reconcile;
pub mod retry;
pub mod scan_pipeline;
pub mod search;
pub mod sort;
pub mod tags;
pub mod thumbnail_policy;
pub mod watch_folders;
pub mod wb_anonymize;
pub mod wb_import;
pub mod wb_sampling;

#[cfg(feature = "testing")]
pub mod testing;

pub fn crate_name() -> &'static str {
    "gb-core"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_its_own_name() {
        assert_eq!(crate_name(), "gb-core");
    }
}
