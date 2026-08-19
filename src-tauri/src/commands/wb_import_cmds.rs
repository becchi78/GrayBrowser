//! `.wb` import commands: file/folder pickers plus the fire-and-forget
//! import launcher.

use std::path::PathBuf;
use std::sync::Arc;

use gb_core::ports::dialog::FolderPicker;
use gb_core::ports::wb_file::WbFilePicker;
use tauri::State;

use crate::adapters;
use crate::db::Db;
use crate::events::{TauriCatalogNotifier, TauriWbImportNotifier};
use crate::thumbnail::ThumbnailQueueHandle;
use crate::wb_import::pipeline::{self, WbImportPaths};

/// Native single-file picker, filtered to `.wb`. Built on demand in the
/// command body -- same pattern as `settings_cmds::pick_watch_folders`'s
/// `RealFolderPicker::new(app.clone())` -- rather than a managed `State`,
/// since it's stateless and only ever used for the duration of one dialog.
#[tauri::command]
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
#[tauri::command]
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
    wb_path: String,
    thumbnail_folder_path: String,
) -> Result<(), String> {
    let wb_source = adapters::wb_source::RealWbSourceAdapter::open(&PathBuf::from(&wb_path))
        .map_err(|e| e.to_string())?;

    let thumbnails_dir = crate::paths::app_data_dir()
        .map(|dir| dir.join("thumbnails"))
        .map_err(|e| e.to_string())?;
    // Best-effort, mirroring start_scan's identical create_dir_all before
    // enqueue_missing_thumbnails -- thumbnail generation below simply
    // produces 0 successes if this directory somehow still doesn't exist.
    let _ = std::fs::create_dir_all(&thumbnails_dir);

    let notifier = Arc::new(TauriWbImportNotifier::new(app.clone()));
    let catalog_notifier = Arc::new(TauriCatalogNotifier::new(app));

    pipeline::run_wb_import(
        db.inner().clone(),
        WbImportPaths {
            wb_path: PathBuf::from(wb_path),
            thumbnail_folder: PathBuf::from(thumbnail_folder_path),
            thumbnails_dir,
        },
        wb_source,
        Arc::new(adapters::ffmpeg::RealFfmpegAdapter),
        notifier,
        catalog_notifier,
        queue.inner().clone(),
    );

    Ok(())
}
