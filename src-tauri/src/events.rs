//! Catalog-changed notification, following this app's usual trait+adapter+
//! fake pattern (`FfmpegAdapter`/`PlayerLauncher`/`FolderPicker`/
//! `FileWatcher`/`DriveTypeDetector`).
//!
//! **Why `CatalogNotifier` lives here (`src-tauri`), not in
//! `gb_core::ports`, unlike those other traits:** every other trait in that
//! list is defined with zero dependency on any OS/framework type in its
//! signature -- `gb-core` stays Tauri- and Windows-independent, and
//! `src-tauri::adapters` is what wires each one to a concrete framework
//! type. `CatalogNotifier`'s real implementation is inherently
//! `tauri::AppHandle<R>`-shaped (there's no OS-independent way to express
//! "notify the frontend"), so putting the *trait* in `gb-core` would still
//! leak `tauri::Runtime` into it through `TauriCatalogNotifier<R>` the
//! moment anything tried to implement it there -- breaking `gb-core`'s
//! Tauri-independence (`cargo tree -p gb-core` must never show
//! `tauri`). So both the trait and its real/fake implementations stay in
//! `src-tauri` together, unlike the `ports`/`adapters`/`testing` three-way
//! split used elsewhere.
//!
//! This split is also what sidesteps a genuine Windows test-environment
//! issue: constructing a real or
//! `tauri::test::mock_app()` `AppHandle` inside this crate's own unit/
//! integration tests pulls `tauri-runtime-wry`'s native dialog code
//! (`TaskDialogIndirect`) into the test binary's import table, which fails
//! to load with `STATUS_ENTRYPOINT_NOT_FOUND` because `cargo test`-generated
//! harness executables don't get the ComCtl32-v6-requesting manifest
//! `build.rs`/`tauri_build::build()` embeds into the real `graybrowser.exe`.
//! `FakeCatalogNotifier` never touches `tauri::AppHandle` at all, so tests
//! using it never hit this. (Confirmed harmless for the real app: nothing in
//! this codebase calls into that runtime-level dialog API -- the folder
//! picker goes through `tauri-plugin-dialog` instead, see
//! `adapters::dialog::RealFolderPicker` -- so this is a transitively-linked
//! but unused-in-production dependency; `graybrowser.exe` gets its manifest
//! regardless and would work either way.)

use tauri::Emitter;

pub trait CatalogNotifier: Send + Sync {
    /// Notifies the frontend that `list_videos`'s result may have changed.
    fn notify_changed(&self);
}

pub struct TauriCatalogNotifier<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: tauri::Runtime> TauriCatalogNotifier<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: tauri::Runtime> CatalogNotifier for TauriCatalogNotifier<R> {
    /// Payload-less by design -- the frontend just re-fetches the full list
    /// (`src/components/ThumbnailGrid.tsx`) rather than trying to reconcile
    /// a diff, so there's nothing useful to carry beyond "something changed".
    fn notify_changed(&self) {
        if let Err(e) = self.app.emit("catalog:changed", ()) {
            log::error!("failed to emit catalog:changed: {e}");
        }
    }
}

/// Progress/completion notification for the `.wb` import pipeline
/// (`wb_import::pipeline::run_wb_import`). Kept in this file rather than `gb_core::ports`, for the same reason
/// `CatalogNotifier` is (see this module's top doc comment): a real
/// implementation is inherently `tauri::AppHandle<R>`-shaped, and this one
/// additionally carries a payload (`WbImportSummary`), unlike
/// `CatalogNotifier`'s deliberately payload-less `notify_changed` -- the
/// import log needs actual counts, not just "something changed".
pub trait WbImportNotifier: Send + Sync {
    /// Emitted periodically while importing `.wb` movie rows (not during the
    /// subsequent thumbnail-linking pass) so a long-running import (e.g. the
    /// real ~3072-row library) can show live progress. `processed`/`total`
    /// are movie-row counts.
    fn notify_progress(&self, processed: u32, total: u32);

    /// Emitted exactly once, after both the row-import loop and the
    /// thumbnail-linking pass have finished.
    fn notify_complete(&self, summary: &WbImportSummary);

    /// Emitted instead of `notify_complete` when the import never got far
    /// enough to produce a `WbImportSummary` at all -- currently, only when
    /// `WbSourceAdapter::read_movies` itself fails (e.g. a `.wb` file whose
    /// schema doesn't have a `movie` table). Without this, `start_wb_import`
    /// returning `Ok(())` gives the frontend no way to learn the import
    /// silently died -- it would just look "in progress" forever, contrary
    /// to the intent to present the migration log to the user. `reason`
    /// is a human-readable message, not a structured error -- there is
    /// exactly one call site today and nothing downstream needs to match on
    /// it programmatically.
    fn notify_failed(&self, reason: &str);
}

/// Final tally reported by `wb_import::pipeline::run_wb_import`'s
/// `notify_complete`.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct WbImportSummary {
    /// New `videos` rows created (`WbImportOutcome::Inserted`).
    pub registered: u32,
    /// `.wb` rows whose `movie_path` was already a registered `file_path`
    /// (`WbImportOutcome::Skipped`) -- e.g. a repeat import run.
    pub skipped: u32,
    /// `.wb` rows with `score > 5`, clamped to a 5-star rating
    /// (`gb_core::wb_import::count_clamped_scores`).
    pub clamped_scores: u32,
    /// Total `video_tags` associations written across every inserted row.
    pub tags_assigned: u32,
    /// Total individual tags present in the raw `.wb` source data across
    /// *every* row (`gb_core::wb_import::count_source_tags`), regardless of
    /// whether that row was inserted or skipped. Lets the frontend tell
    /// "`tags_assigned == 0` because the source never had tags" apart from
    /// "...because something went wrong" -- `tags_assigned` alone can't
    /// make that distinction.
    pub source_tag_count: u32,
    /// **Offline rows only.** Legacy JPG thumbnails successfully converted
    /// to WebP and linked to their video
    /// (`gb_core::wb_import::match_thumbnail_files`'s `matched` set, minus
    /// any that failed conversion/rename). An *online* row's video file is
    /// available, so it never goes through this legacy-JPG path at all --
    /// its thumbnail is instead generated from the real video by the
    /// existing background pipeline
    /// (`thumbnail::enqueue_missing_thumbnails`, kicked off by
    /// `wb_import::pipeline::run_wb_import` right after this summary is
    /// built) and is therefore not reflected in this count, or in
    /// `thumbnails_failed`/`thumbnails_unmatched` below -- that pipeline
    /// reports its own progress via `catalog:changed`, not via
    /// `WbImportSummary`.
    pub thumbnails_linked: u32,
    /// **Offline rows only** (see `thumbnails_linked`). Legacy JPGs that
    /// matched an *offline* video by hash but failed to convert or rename
    /// into place.
    pub thumbnails_failed: u32,
    /// Legacy JPG filenames that matched no offline video's hash (unknown
    /// naming pattern, or a hash with no corresponding `.wb` row) --
    /// **including** filenames whose hash happens to belong to an *online*
    /// row, since online rows are deliberately excluded from legacy-JPG
    /// matching (see `thumbnails_linked`) and so read as "unmatched" here
    /// even though their hash technically exists in the `.wb` data.
    pub thumbnails_unmatched: u32,
}

pub struct TauriWbImportNotifier<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: tauri::Runtime> TauriWbImportNotifier<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: tauri::Runtime> WbImportNotifier for TauriWbImportNotifier<R> {
    fn notify_progress(&self, processed: u32, total: u32) {
        #[derive(Clone, serde::Serialize)]
        struct Progress {
            processed: u32,
            total: u32,
        }
        if let Err(e) = self
            .app
            .emit("wb_import:progress", Progress { processed, total })
        {
            log::error!("failed to emit wb_import:progress: {e}");
        }
    }

    fn notify_complete(&self, summary: &WbImportSummary) {
        if let Err(e) = self.app.emit("wb_import:complete", summary) {
            log::error!("failed to emit wb_import:complete: {e}");
        }
    }

    fn notify_failed(&self, reason: &str) {
        if let Err(e) = self.app.emit("wb_import:failed", reason) {
            log::error!("failed to emit wb_import:failed: {e}");
        }
    }
}

/// Duplicate-group-refresh notification for
/// `dedup::refresh_duplicate_groups`. Kept in this file rather than
/// `gb_core::ports`, for the same reason `CatalogNotifier`/`WbImportNotifier`
/// are (see this module's top doc comment): a real implementation is
/// inherently `tauri::AppHandle<R>`-shaped, and this one carries a payload
/// (the freshly detected groups) the same way `WbImportNotifier::
/// notify_complete` does.
pub trait DedupNotifier: Send + Sync {
    /// Emitted once a `detect_duplicate_groups` pass finishes, carrying the
    /// full up-to-date group list -- mirrors `dedup::DuplicateGroupsState`'s
    /// contents at the moment this fires, so the frontend can either use the
    /// payload directly or simply treat it as a "changed" signal and re-call
    /// `list_duplicate_groups`.
    fn notify_updated(&self, groups: &[crate::dedup::DuplicateGroup]);
}

pub struct TauriDedupNotifier<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
}

impl<R: tauri::Runtime> TauriDedupNotifier<R> {
    pub fn new(app: tauri::AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: tauri::Runtime> DedupNotifier for TauriDedupNotifier<R> {
    fn notify_updated(&self, groups: &[crate::dedup::DuplicateGroup]) {
        if let Err(e) = self.app.emit("dedup:updated", groups) {
            log::error!("failed to emit dedup:updated: {e}");
        }
    }
}

#[cfg(feature = "testing")]
pub use fake::{FakeCatalogNotifier, FakeDedupNotifier, FakeWbImportNotifier};

#[cfg(feature = "testing")]
mod fake {
    use super::{CatalogNotifier, DedupNotifier, WbImportNotifier, WbImportSummary};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// Records how many times `notify_changed` was called, so tests can
    /// assert the exact emit-triggering conditions (e.g. a `Registered`
    /// `ProcessOutcome` calls it, an `Unchanged` one doesn't) without ever
    /// constructing a `tauri::AppHandle` -- see this module's doc comment
    /// for why that matters on Windows.
    #[derive(Default)]
    pub struct FakeCatalogNotifier {
        pub call_count: AtomicUsize,
    }

    impl FakeCatalogNotifier {
        pub fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    impl CatalogNotifier for FakeCatalogNotifier {
        fn notify_changed(&self) {
            self.call_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Records every `notify_progress`/`notify_complete` call, mirroring
    /// `FakeCatalogNotifier`'s reasoning: never touches `tauri::AppHandle`,
    /// so `.wb`-import pipeline tests (`wb_import::pipeline`) can assert on
    /// exact progress/summary values without the Windows
    /// `STATUS_ENTRYPOINT_NOT_FOUND` test-harness issue documented above.
    #[derive(Default)]
    pub struct FakeWbImportNotifier {
        pub progress_calls: Mutex<Vec<(u32, u32)>>,
        pub complete_calls: Mutex<Vec<WbImportSummary>>,
        pub failed_calls: Mutex<Vec<String>>,
    }

    impl WbImportNotifier for FakeWbImportNotifier {
        fn notify_progress(&self, processed: u32, total: u32) {
            self.progress_calls.lock().unwrap().push((processed, total));
        }

        fn notify_complete(&self, summary: &WbImportSummary) {
            self.complete_calls.lock().unwrap().push(summary.clone());
        }

        fn notify_failed(&self, reason: &str) {
            self.failed_calls.lock().unwrap().push(reason.to_string());
        }
    }

    /// Records every `notify_updated` call's group count, mirroring
    /// `FakeCatalogNotifier`/`FakeWbImportNotifier`'s reasoning: never
    /// touches `tauri::AppHandle`, so `dedup::` tests can assert on exact
    /// notification counts/payload sizes without the Windows
    /// `STATUS_ENTRYPOINT_NOT_FOUND` test-harness issue documented above.
    /// Stores only each call's group count (not the full
    /// `Vec<crate::dedup::DuplicateGroup>`) -- `DuplicateGroup` doesn't
    /// derive `PartialEq`/`Debug`, and no test so far has needed to inspect
    /// more than "how many groups, how many times".
    #[derive(Default)]
    pub struct FakeDedupNotifier {
        pub updated_calls: Mutex<Vec<usize>>,
    }

    impl DedupNotifier for FakeDedupNotifier {
        fn notify_updated(&self, groups: &[crate::dedup::DuplicateGroup]) {
            self.updated_calls.lock().unwrap().push(groups.len());
        }
    }
}
