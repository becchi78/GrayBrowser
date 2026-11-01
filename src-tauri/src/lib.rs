pub mod adapters;
pub mod commands;
pub mod db;
pub mod dedup;
pub mod events;
pub mod logging;
pub mod metadata;
pub mod paths;
pub mod reachability;
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
const MENU_ITEM_TAG_BAR_MANAGE: &str = "menu-tag-bar-manage";
const MENU_ITEM_DB_BACKUP: &str = "menu-db-backup";
const MENU_ITEM_SETTINGS: &str = "menu-settings";
const MENU_ITEM_ABOUT: &str = "menu-about";
const MENU_ITEM_LOG_VIEWER: &str = "menu-log-viewer";
const MENU_ITEM_CHECK_UPDATE: &str = "menu-check-update";
// The "スタイル" menu. Only one style ("Default") exists today, so there is
// nothing to select between yet -- the item exists so the menu bar has the
// intended shape, and clicking it just re-asserts its own checked state
// (see `on_menu_event` below). Extend this to real mutual-exclusion once a
// second style is added.
const MENU_ITEM_STYLE_DEFAULT: &str = "menu-style-default";

fn run_manual_backup(app_handle: tauri::AppHandle) {
    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

    std::thread::spawn(move || {
        let outcome = (|| -> anyhow::Result<std::path::PathBuf> {
            let app_dir = paths::app_data_dir()?;
            let db = app_handle.state::<db::Db>();
            let conn = db.read_pool.get()?;
            db::backup::create_backup(
                &conn,
                &db::backup::backups_dir(&app_dir),
                gb_core::backup::BackupKind::Manual,
                db::backup::unix_now(),
            )
        })();
        let (message, kind) = match outcome {
            Ok(path) => (
                format!("バックアップを作成しました。\n{}", path.display()),
                MessageDialogKind::Info,
            ),
            Err(e) => {
                log::error!("manual database backup failed: {e:#}");
                (
                    format!("バックアップに失敗しました。\n{e:#}"),
                    MessageDialogKind::Error,
                )
            }
        };
        app_handle
            .dialog()
            .message(message)
            .title("データベースをバックアップ")
            .kind(kind)
            .show(|_| {});
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
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

            // Which watch folders cannot be reached right now. Nothing is
            // probed here: the first probe of a folder happens on whichever
            // background thread needs the answer first, because a dead
            // network share blocks for ~20s and `setup` must not.
            let reachability = reachability::ReachabilityRegistry::new(
                std::sync::Arc::new(adapters::folder_probe::RealFolderProbe),
                std::sync::Arc::new(events::TauriReachabilityNotifier::new(app.handle().clone())),
            );
            app.manage(reachability.clone());

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

            // Nothing is generating yet, so every leftover .tmp is an orphan.
            // Clearing them before the queue starts keeps stalled videos from
            // carrying stale partial output into regeneration.
            let removed = thumbnail::remove_orphan_tmp_files(&thumbnails_root);
            if removed > 0 {
                log::info!("removed {removed} orphan thumbnail .tmp file(s)");
            }

            // Startup resume: re-enumerate videos missing a
            // thumbnail and kick off generation in the background.
            thumbnail::enqueue_missing_thumbnails(
                db.clone(),
                thumbnails_root.clone(),
                queue.clone(),
                std::sync::Arc::new(adapters::ffmpeg::RealFfmpegAdapter),
                std::sync::Arc::clone(&notifier),
                reachability.clone(),
            );

            // Startup resume: same stateless philosophy as the
            // thumbnail queue above -- re-enumerate videos missing metadata
            // (`probed_at IS NULL`) and kick off probing in the background.
            metadata::enqueue_missing_metadata_probes(
                db.clone(),
                std::sync::Arc::new(adapters::ffmpeg::RealFfmpegAdapter),
                std::sync::Arc::clone(&notifier),
                reachability.clone(),
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
                reachability.clone(),
            );

            // Local folders get realtime
            // `notify` watching, network folders get startup diff-scan +
            // periodic polling. `reconfigure` classifies each registered
            // folder by drive type and is re-invoked (with the manager kept
            // alive across calls) whenever `pick_watch_folders` changes the
            // folder list -- see commands::settings_cmds::pick_watch_folders.
            //
            // Files the watchers/pollers register arrive with no thumbnail or
            // metadata; the follow-up re-runs the same enqueue pair a scan
            // does, once the burst of registrations settles.
            let follow_up = {
                let db = db.clone();
                let thumbnails_root = thumbnails_root.clone();
                let queue = queue.clone();
                let notifier = std::sync::Arc::clone(&notifier);
                let reachability = reachability.clone();
                std::sync::Arc::new(watch::follow_up::FollowUp::new(move || {
                    thumbnail::enqueue_missing_thumbnails(
                        db.clone(),
                        thumbnails_root.clone(),
                        queue.clone(),
                        std::sync::Arc::new(adapters::ffmpeg::RealFfmpegAdapter),
                        std::sync::Arc::clone(&notifier),
                        reachability.clone(),
                    );
                    metadata::enqueue_missing_metadata_probes(
                        db.clone(),
                        std::sync::Arc::new(adapters::ffmpeg::RealFfmpegAdapter),
                        std::sync::Arc::clone(&notifier),
                        reachability.clone(),
                    );
                }))
            };
            // A folder coming back resumes the work that was skipped while
            // it was down. The registry and the follow-up reference each
            // other, which is fine: both live for the whole process.
            {
                let follow_up = std::sync::Arc::clone(&follow_up);
                reachability.set_on_recovered(move || follow_up.trigger());
            }
            // Detached on purpose: it runs for the life of the process.
            let _ = reachability.start_recheck(reachability::DEFAULT_RECHECK_INTERVAL);
            let watch_manager = watch::RealtimeWatchManager::new(follow_up, reachability.clone());
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

            // Native menu bar. "ファイル" / "タグ" / "スタイル" /
            // "ヘルプ" -- "表示" is deliberately not added yet (undecided).
            // Item clicks just emit a frontend event; the modals
            // themselves are handled by the frontend, so there's nothing
            // else to wire up here.
            let folder_manage_item =
                MenuItemBuilder::with_id(MENU_ITEM_FOLDER_MANAGE, "フォルダ管理...").build(app)?;
            let wb_import_item =
                MenuItemBuilder::with_id(MENU_ITEM_WB_IMPORT, ".wbインポート...").build(app)?;
            let db_backup_item =
                MenuItemBuilder::with_id(MENU_ITEM_DB_BACKUP, "データベースをバックアップ")
                    .build(app)?;
            let quit_item = PredefinedMenuItem::quit(app, Some("終了"))?;
            let file_menu = SubmenuBuilder::new(app, "ファイル")
                .item(&folder_manage_item)
                .item(&wb_import_item)
                .separator()
                .item(&db_backup_item)
                .separator()
                .item(&quit_item)
                .build()?;

            // "タグ" -- its own top-level submenu (not nested under
            // "ファイル") since it's not a file-management action; only one
            // item exists today (the pinned-tags editor), but this gives
            // future tag-related menu entries a home without further
            // reshuffling "ファイル".
            let tag_bar_manage_item =
                MenuItemBuilder::with_id(MENU_ITEM_TAG_BAR_MANAGE, "タグバーの編集...")
                    .build(app)?;
            let tag_menu = SubmenuBuilder::new(app, "タグ")
                .item(&tag_bar_manage_item)
                .build()?;

            let style_default_item =
                CheckMenuItemBuilder::with_id(MENU_ITEM_STYLE_DEFAULT, "Default")
                    .checked(true)
                    .build(app)?;
            let style_menu = SubmenuBuilder::new(app, "スタイル")
                .item(&style_default_item)
                .build()?;

            let settings_item =
                MenuItemBuilder::with_id(MENU_ITEM_SETTINGS, "設定...").build(app)?;
            let settings_menu = SubmenuBuilder::new(app, "設定")
                .item(&settings_item)
                .build()?;

            let about_item =
                MenuItemBuilder::with_id(MENU_ITEM_ABOUT, "バージョン情報").build(app)?;
            let log_viewer_item =
                MenuItemBuilder::with_id(MENU_ITEM_LOG_VIEWER, "動作ログ...").build(app)?;
            let check_update_item =
                MenuItemBuilder::with_id(MENU_ITEM_CHECK_UPDATE, "更新の確認...").build(app)?;
            let help_menu = SubmenuBuilder::new(app, "ヘルプ")
                .item(&log_viewer_item)
                .item(&check_update_item)
                .item(&about_item)
                .build()?;

            let menu = MenuBuilder::new(app)
                .item(&file_menu)
                .item(&tag_menu)
                .item(&style_menu)
                .item(&settings_menu)
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

                // Handled entirely in Rust (no frontend modal involved): the
                // backup runs off the event-loop thread and reports through a
                // native message dialog.
                if event.id().as_ref() == MENU_ITEM_DB_BACKUP {
                    run_manual_backup(app_handle.clone());
                    return;
                }

                let event_name = match event.id().as_ref() {
                    id if id == MENU_ITEM_FOLDER_MANAGE => "menu:open-folder-dialog",
                    id if id == MENU_ITEM_WB_IMPORT => "menu:open-wb-import-dialog",
                    id if id == MENU_ITEM_TAG_BAR_MANAGE => "menu:open-tag-bar-dialog",
                    id if id == MENU_ITEM_SETTINGS => "menu:open-settings-dialog",
                    id if id == MENU_ITEM_LOG_VIEWER => "menu:open-log-dialog",
                    id if id == MENU_ITEM_CHECK_UPDATE => "menu:check-update",
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
            commands::log_cmds::read_log,
            commands::log_cmds::get_log_dir,
            commands::reachability_cmds::get_unreachable_folders,
            commands::reachability_cmds::check_folder_reachability,
            commands::update_cmds::get_update_check_on_startup,
            commands::update_cmds::set_update_check_on_startup,
            commands::update_cmds::check_for_update,
            commands::settings_cmds::pick_watch_folders,
            commands::settings_cmds::list_watch_folders,
            commands::settings_cmds::reorder_watch_folders,
            commands::settings_cmds::list_watch_check_disabled_folders,
            commands::settings_cmds::set_watch_folder_check_enabled,
            commands::settings_cmds::get_nas_poll_interval_minutes,
            commands::settings_cmds::set_nas_poll_interval_minutes,
            commands::settings_cmds::count_videos_under_folder,
            commands::settings_cmds::remove_watch_folder,
            commands::settings_cmds::rename_watch_folder,
            commands::scan_cmds::start_scan,
            commands::scan_cmds::list_videos,
            commands::scan_cmds::list_skipped_files,
            commands::skipped_cmds::rename_skipped_file,
            commands::skipped_cmds::dismiss_skipped_file,
            commands::skipped_cmds::register_missing_skipped_file,
            commands::thumbnail_cmds::toggle_thumbnail_pause,
            commands::thumbnail_cmds::get_thumbnails,
            commands::thumbnail_cmds::get_thumbnail_progress,
            commands::player_cmds::play_video,
            commands::tag_cmds::assign_tag,
            commands::tag_cmds::assign_tag_to_videos,
            commands::tag_cmds::remove_tag,
            commands::tag_cmds::list_tags_for_video,
            commands::tag_cmds::list_all_tags,
            commands::info_pane_cmds::get_info_pane_collapsed,
            commands::info_pane_cmds::set_info_pane_collapsed,
            commands::sidebar_cmds::get_sidebar_sections_collapsed,
            commands::sidebar_cmds::set_sidebar_sections_collapsed,
            commands::tag_cmds::set_tag_order,
            commands::rating_cmds::set_rating,
            commands::properties_cmds::get_video_properties,
            commands::wb_import_cmds::pick_wb_file,
            commands::wb_import_cmds::pick_wb_thumbnail_folder,
            commands::wb_import_cmds::start_wb_import,
            commands::wb_import_cmds::open_wb_import_log,
            commands::catalog_cmds::unregister_videos,
            commands::dedup_cmds::list_duplicate_groups,
            commands::dedup_cmds::refresh_duplicate_groups,
            commands::dedup_cmds::delete_duplicate_video,
            commands::dedup_cmds::delete_duplicate_video_and_file,
            commands::generation_retry_cmds::list_generation_failures,
            commands::generation_retry_cmds::retry_thumbnail_generation,
            commands::generation_retry_cmds::retry_metadata_probe,
            commands::generation_retry_cmds::count_missing_thumbnails,
            commands::generation_retry_cmds::regenerate_missing_thumbnails,
            commands::generation_retry_cmds::count_missing_metadata,
            commands::generation_retry_cmds::reprobe_missing_metadata
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
