pub mod adapters;
pub mod commands;
pub mod db;
pub mod dedup;
pub mod events;
pub mod logging;
pub mod metadata;
pub mod paths;
pub mod scan;
pub mod thumbnail;
pub mod watch;
pub mod wb_import;

use tauri::menu::{
    CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder,
};
use tauri::{Emitter, Manager};

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

// Native menu item ids, matched against in the `on_menu_event` handler
// installed in `run()` below. Kept as constants rather than inline string
// literals so the id used to build each item and the id matched on in the
// handler can't silently drift apart.
const MENU_ITEM_FOLDER_MANAGE: &str = "menu-folder-manage";
const MENU_ITEM_WB_IMPORT: &str = "menu-wb-import";
const MENU_ITEM_ABOUT: &str = "menu-about";
// The "スタイル" menu. Only one style ("Default") exists today, so there is
// nothing to select between yet -- the item exists so the menu bar has the
// intended shape, and clicking it just re-asserts its own checked state
// (see `on_menu_event` below). Extend this to real mutual-exclusion once a
// second style is added.
const MENU_ITEM_STYLE_DEFAULT: &str = "menu-style-default";

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let app_dir = paths::app_data_dir()?;
            std::fs::create_dir_all(&app_dir)?;
            // Before db::init so a migration failure is captured in app.log
            // too, not just wherever Tauri prints its own startup error.
            let _ = logging::init(&app_dir);
            // Before db::init: moves any pre-existing double-nested install
            // data (see `paths::migrate_legacy_nested_app_dir`) into place
            // first, so db::init opens the real database rather than
            // creating a fresh empty one.
            paths::migrate_legacy_nested_app_dir(&app_dir)?;
            let db = db::init(&app_dir.join("app.db"))
                .inspect_err(|e| log::error!("failed to initialize database: {e}"))?;
            app.manage(db.clone());

            let queue = thumbnail::ThumbnailQueueHandle::default();
            app.manage(queue.clone());

            let dedup_state = dedup::DuplicateGroupsState::default();
            app.manage(dedup_state.clone());

            println!(
                "graybrowser starting (linked against {}); app dir: {}",
                gb_core::crate_name(),
                app_dir.display()
            );
            log::info!(
                "graybrowser starting (linked against {}); app dir: {}",
                gb_core::crate_name(),
                app_dir.display()
            );

            let notifier =
                std::sync::Arc::new(events::TauriCatalogNotifier::new(app.handle().clone()));

            // One-time migration from the pre-#6 flat thumbnail layout to
            // per-registered-folder subdirectories. Runs after db::init
            // (needs the DB to look up each leftover file's owning video)
            // and before enqueue_missing_thumbnails (which must see the
            // post-migration layout, not race it). Thumbnails are
            // regenerable, so a failure here is logged and startup
            // continues -- never `?`, unlike the DB migration above.
            let thumbnails_root = thumbnail::paths::thumbnails_root(&app_dir);
            std::fs::create_dir_all(&thumbnails_root)?;
            if let Err(e) = thumbnail::migration::migrate_flat_thumbnails_to_folder_subdirs(
                &db,
                &thumbnails_root,
            ) {
                log::error!("thumbnail layout migration failed: {e}");
            }

            // Startup resume: re-enumerate videos missing a
            // thumbnail and kick off generation in the background.
            thumbnail::enqueue_missing_thumbnails(
                db.clone(),
                thumbnails_root.clone(),
                queue,
                std::sync::Arc::new(adapters::ffmpeg::RealFfmpegAdapter),
                std::sync::Arc::clone(&notifier),
            );

            // Startup resume: same stateless philosophy as the
            // thumbnail queue above -- re-enumerate videos missing metadata
            // (`probed_at IS NULL`) and kick off probing in the background.
            metadata::enqueue_missing_metadata_probes(
                db.clone(),
                std::sync::Arc::new(adapters::ffmpeg::RealFfmpegAdapter),
                std::sync::Arc::clone(&notifier),
            );

            // Startup resume: same
            // stateless philosophy as the two resume passes above --
            // duplicate detection isn't persisted as its own job either, it
            // just re-derives everything from `videos`/`path_collisions` on
            // every run.
            dedup::refresh_duplicate_groups(
                db.clone(),
                dedup_state,
                std::sync::Arc::new(events::TauriDedupNotifier::new(app.handle().clone())),
            );

            // Local folders get realtime
            // `notify` watching, network folders get startup diff-scan +
            // periodic polling. `reconfigure` classifies each registered
            // folder by drive type and is re-invoked (with the manager kept
            // alive across calls) whenever `pick_watch_folders` changes the
            // folder list -- see commands::settings_cmds::pick_watch_folders.
            let watch_manager = watch::RealtimeWatchManager::default();
            let (watch_folders, nas_poll_interval_secs) = {
                let conn = db.read_pool.get()?;
                (
                    db::queries::get_watch_folders(&conn)?,
                    db::queries::get_nas_poll_interval_secs(&conn)?,
                )
            };
            watch::reconfigure_real_watch_manager(
                app.handle(),
                &db,
                &watch_manager,
                &thumbnails_root,
                &watch_folders,
                nas_poll_interval_secs,
            );
            app.manage(watch_manager);

            // Native menu bar. "ファイル" / "スタイル" /
            // "ヘルプ" -- "表示" is deliberately not added yet (undecided).
            // Item clicks just emit a frontend event; the modals
            // themselves are handled by the frontend, so there's nothing
            // else to wire up here.
            let folder_manage_item =
                MenuItemBuilder::with_id(MENU_ITEM_FOLDER_MANAGE, "フォルダ管理...").build(app)?;
            let wb_import_item =
                MenuItemBuilder::with_id(MENU_ITEM_WB_IMPORT, ".wbインポート...").build(app)?;
            let quit_item = PredefinedMenuItem::quit(app, Some("終了"))?;
            let file_menu = SubmenuBuilder::new(app, "ファイル")
                .item(&folder_manage_item)
                .item(&wb_import_item)
                .separator()
                .item(&quit_item)
                .build()?;

            let style_default_item =
                CheckMenuItemBuilder::with_id(MENU_ITEM_STYLE_DEFAULT, "Default")
                    .checked(true)
                    .build(app)?;
            let style_menu = SubmenuBuilder::new(app, "スタイル")
                .item(&style_default_item)
                .build()?;

            let about_item =
                MenuItemBuilder::with_id(MENU_ITEM_ABOUT, "バージョン情報").build(app)?;
            let help_menu = SubmenuBuilder::new(app, "ヘルプ")
                .item(&about_item)
                .build()?;

            let menu = MenuBuilder::new(app)
                .item(&file_menu)
                .item(&style_menu)
                .item(&help_menu)
                .build()?;
            app.set_menu(menu)?;

            app.on_menu_event(move |app_handle, event| {
                // "スタイル > Default" is handled separately from the
                // payload-less events below: today it's the only style, so
                // clicking it must not visibly uncheck itself (there's
                // nothing else to select instead), and it carries a string
                // payload rather than none.
                if event.id().as_ref() == MENU_ITEM_STYLE_DEFAULT {
                    if let Err(e) = style_default_item.set_checked(true) {
                        log::error!("failed to re-assert スタイル > Default checked state: {e}");
                    }
                    if let Err(e) = app_handle.emit("menu:style-selected", "default") {
                        log::error!("failed to emit menu:style-selected: {e}");
                    }
                    return;
                }

                let event_name = match event.id().as_ref() {
                    id if id == MENU_ITEM_FOLDER_MANAGE => "menu:open-folder-dialog",
                    id if id == MENU_ITEM_WB_IMPORT => "menu:open-wb-import-dialog",
                    id if id == MENU_ITEM_ABOUT => "menu:about",
                    // "終了" is Tauri's PredefinedMenuItem::quit -- it's
                    // handled natively and never reaches this listener.
                    _ => return,
                };
                if let Err(e) = app_handle.emit(event_name, ()) {
                    log::error!("failed to emit {event_name}: {e}");
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            commands::ffmpeg_cmds::get_ffmpeg_status,
            commands::settings_cmds::pick_watch_folders,
            commands::settings_cmds::list_watch_folders,
            commands::settings_cmds::count_videos_under_folder,
            commands::settings_cmds::remove_watch_folder,
            commands::settings_cmds::rename_watch_folder,
            commands::scan_cmds::start_scan,
            commands::scan_cmds::list_videos,
            commands::scan_cmds::list_skipped_files,
            commands::thumbnail_cmds::toggle_thumbnail_pause,
            commands::thumbnail_cmds::get_thumbnails,
            commands::player_cmds::play_video,
            commands::tag_cmds::assign_tag,
            commands::tag_cmds::remove_tag,
            commands::tag_cmds::list_tags_for_video,
            commands::tag_cmds::list_all_tags,
            commands::rating_cmds::set_rating,
            commands::properties_cmds::get_video_properties,
            commands::wb_import_cmds::pick_wb_file,
            commands::wb_import_cmds::pick_wb_thumbnail_folder,
            commands::wb_import_cmds::start_wb_import,
            commands::dedup_cmds::list_duplicate_groups,
            commands::dedup_cmds::refresh_duplicate_groups,
            commands::dedup_cmds::delete_duplicate_video,
            commands::generation_retry_cmds::list_generation_failures,
            commands::generation_retry_cmds::retry_thumbnail_generation,
            commands::generation_retry_cmds::retry_metadata_probe
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
