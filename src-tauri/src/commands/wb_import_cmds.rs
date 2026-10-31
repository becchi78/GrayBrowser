//! `.wb` import commands: file/folder pickers plus the fire-and-forget
//! import launcher.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gb_core::ports::dialog::FolderPicker;
use gb_core::ports::wb_file::WbFilePicker;
use gb_core::ports::wb_import_log::WbImportLog;
use tauri::State;
use tauri_plugin_opener::OpenerExt;

use crate::adapters;
use crate::db::Db;
use crate::events::{TauriCatalogNotifier, TauriWatchReconfigurer, TauriWbImportNotifier};
use crate::reachability::ReachabilityRegistry;
use crate::thumbnail::ThumbnailQueueHandle;
use crate::wb_import::pipeline::{self, WbImportPaths};

/// Native single-file picker, filtered to `.wb`. Built on demand in the
/// command body -- same pattern as `settings_cmds::pick_watch_folders`'s
/// `RealFolderPicker::new(app.clone())` -- rather than a managed `State`,
/// since it's stateless and only ever used for the duration of one dialog.
///
/// `async` so it runs off the main thread: a sync command runs on the UI
/// thread on Windows, and the blocking dialog call would freeze the window.
#[tauri::command(async)]
pub fn pick_wb_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let picker = adapters::wb_file::RealWbFilePicker::new(app);
    picker
        .pick_wb_file()
        .map(|picked| picked.map(|p| p.to_string_lossy().to_string()))
        .map_err(|e| e.to_string())
}

/// Reuses the existing multi-folder `FolderPicker` (there is no
/// single-folder-only native dialog variant wired up in this codebase) and
/// keeps only the first selected folder -- the legacy thumbnail folder is a
/// single directory, so a user picking more than one just means "use the
/// first" rather than an error.
///
/// `async` for the same reason as `pick_wb_file`: keeps the blocking dialog
/// off the UI thread.
#[tauri::command(async)]
pub fn pick_wb_thumbnail_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let picker = adapters::dialog::RealFolderPicker::new(app);
    picker
        .pick_folders()
        .map(|picked| {
            picked
                .and_then(|folders| folders.into_iter().next())
                .map(|p| p.to_string_lossy().to_string())
        })
        .map_err(|e| e.to_string())
}

/// `async` for consistency with `scan_cmds::start_scan`, but -- unlike
/// `start_scan` -- does not await the import itself:
/// `pipeline::run_wb_import` spawns its own background thread, and this
/// command returns as soon as that thread is launched. This is what keeps a
/// ~3072-row real library from blocking the UI.
///
/// Opening the `.wb` file (a single `rusqlite::Connection::open_with_flags`,
/// fast) is the one part done synchronously here, so an unopenable path or a
/// corrupt file is reported immediately as a normal command `Err`, rather
/// than only surfacing later via a `wb_import:complete` event the frontend
/// might not even be listening for yet.
///
/// `queue: State<ThumbnailQueueHandle>` mirrors `scan_cmds::start_scan`'s
/// identical parameter: `pipeline::run_wb_import` hands this straight to
/// `thumbnail::enqueue_missing_thumbnails` once the row-import pass is done,
/// so online `.wb` videos get a thumbnail generated from the real video file
/// through the same pause-aware queue a normal scan uses (design change
/// after initial Stage 4 review -- online rows no longer go through the
/// legacy-JPG conversion path at all; see `pipeline::import_all`'s
/// `Inserted` match arm).
#[tauri::command]
pub async fn start_wb_import(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    queue: State<'_, ThumbnailQueueHandle>,
    reachability: State<'_, ReachabilityRegistry>,
    wb_path: String,
    thumbnail_folder_path: String,
    register_watch_folders: bool,
) -> Result<(), String> {
    // Created first so that even a `.wb` that cannot be opened leaves a log
    // for "ログを確認する". Failing to make the log never stops the import.
    let import_log = match super::log_cmds::logs_dir() {
        Ok(dir) => adapters::wb_import_log::FileWbImportLog::create(
            &dir,
            &adapters::wb_import_log::local_now(),
        ),
        Err(e) => {
            log::warn!("wb import log: no log directory: {e}; importing without a log");
            adapters::wb_import_log::FileWbImportLog::unavailable()
        }
    };

    let thumbnails_dir = crate::paths::app_data_dir()
        .map(|dir| dir.join("thumbnails"))
        .map_err(|e| e.to_string())?;
    // Best-effort, mirroring start_scan's identical create_dir_all before
    // enqueue_missing_thumbnails -- thumbnail generation below simply
    // produces 0 successes if this directory somehow still doesn't exist.
    let _ = std::fs::create_dir_all(&thumbnails_dir);

    let paths = WbImportPaths {
        wb_path: PathBuf::from(&wb_path),
        thumbnail_folder: PathBuf::from(&thumbnail_folder_path),
        thumbnails_dir,
    };
    let wb_source = open_wb_source_logged(&import_log, &paths, register_watch_folders, || {
        adapters::wb_source::RealWbSourceAdapter::open(&paths.wb_path).map_err(|e| e.to_string())
    })?;

    let notifier = Arc::new(TauriWbImportNotifier::new(app.clone()));
    let watch_reconfigurer = Arc::new(TauriWatchReconfigurer::new(
        app.clone(),
        crate::thumbnail::paths::thumbnails_root(
            &crate::paths::app_data_dir().map_err(|e| e.to_string())?,
        ),
    ));
    let catalog_notifier = Arc::new(TauriCatalogNotifier::new(app));

    pipeline::run_wb_import(
        db.inner().clone(),
        paths,
        wb_source,
        Arc::new(adapters::ffmpeg::RealFfmpegAdapter),
        notifier,
        catalog_notifier,
        queue.inner().clone(),
        reachability.inner().clone(),
        register_watch_folders,
        watch_reconfigurer,
        Arc::new(adapters::dir_lister::RealDirLister),
        import_log,
    );

    Ok(())
}

/// Opens the `.wb` through `open`; if that fails, the reason is written to the
/// import log before the error goes back to the caller.
fn open_wb_source_logged<S, L: WbImportLog>(
    import_log: &L,
    paths: &WbImportPaths,
    register_watch_folders: bool,
    open: impl FnOnce() -> Result<S, String>,
) -> Result<S, String> {
    open().inspect_err(|reason| {
        pipeline::record_start_failure(import_log, paths, register_watch_folders, reason)
    })
}

/// Opens the newest import log in `logs_dir` through `open`. Takes no path
/// from the caller: the only file this can open is an import log inside the
/// logs directory.
fn open_latest_wb_import_log_impl(
    logs_dir: &Path,
    open: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let path = adapters::wb_import_log::latest_log_path(logs_dir)
        .ok_or_else(|| "取り込みログがまだありません。".to_string())?;
    open(&path).map_err(|e| format!("取り込みログを開けませんでした: {e}"))
}

/// `async` so it runs on a worker thread rather than the UI thread; the shell
/// call needs COM set up on that thread first.
#[tauri::command(async)]
pub fn open_wb_import_log(app: tauri::AppHandle) -> Result<(), String> {
    open_latest_wb_import_log_impl(&super::log_cmds::logs_dir()?, |path| {
        let path_str = path
            .to_str()
            .ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()))?;
        adapters::player::init_com_for_shell();
        app.opener()
            .open_path(path_str, None::<&str>)
            .map_err(|e| e.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gb_core::testing::fake_wb_import_log::FakeWbImportLog;
    use std::cell::RefCell;

    fn paths() -> WbImportPaths {
        WbImportPaths {
            wb_path: PathBuf::from(r"C:\data\a.wb"),
            thumbnail_folder: PathBuf::from(r"C:\thumbs"),
            thumbnails_dir: PathBuf::new(),
        }
    }

    #[test]
    fn an_unopenable_wb_leaves_its_reason_in_the_log_and_still_errs() {
        let log = FakeWbImportLog::default();
        let err = open_wb_source_logged(&log, &paths(), true, || -> Result<(), String> {
            Err("file is not a database".into())
        })
        .unwrap_err();
        assert_eq!(err, "file is not a database");
        let text = log.text();
        assert!(text.contains("a.wb"));
        assert!(text.contains("取り込みを中止しました: file is not a database"));
    }

    #[test]
    fn an_openable_wb_writes_nothing_at_this_step() {
        let log = FakeWbImportLog::default();
        let v = open_wb_source_logged(&log, &paths(), true, || Ok(5)).unwrap();
        assert_eq!(v, 5);
        assert_eq!(log.append_count(), 0);
    }

    #[test]
    fn opens_the_newest_import_log_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "app_rCURRENT.log",
            "wb-import-20261001-000000.log",
            "wb-import-20261002-000000.log",
        ] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        let opened = RefCell::new(Vec::new());
        open_latest_wb_import_log_impl(dir.path(), |p| {
            opened.borrow_mut().push(p.to_path_buf());
            Ok(())
        })
        .unwrap();
        assert_eq!(
            *opened.borrow(),
            vec![dir.path().join("wb-import-20261002-000000.log")]
        );
    }

    #[test]
    fn reports_a_missing_log_without_trying_to_open() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app_rCURRENT.log"), "").unwrap();
        let called = RefCell::new(false);
        let err = open_latest_wb_import_log_impl(dir.path(), |_| {
            *called.borrow_mut() = true;
            Ok(())
        })
        .unwrap_err();
        assert!(err.contains("まだありません"));
        assert!(!*called.borrow());
    }

    #[test]
    fn passes_an_open_failure_on_with_context() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("wb-import-20261001-000000.log"), "").unwrap();
        let err = open_latest_wb_import_log_impl(dir.path(), |_| Err("no association".into()))
            .unwrap_err();
        assert!(err.contains("開けませんでした") && err.contains("no association"));
    }
}
