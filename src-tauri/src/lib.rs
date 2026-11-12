pub mod adapters;
pub mod commands;
pub mod db;
pub mod dedup;
pub mod events;
pub mod logging;
pub mod metadata;
pub mod offline_sync;
pub mod paths;
pub mod reachability;
pub mod scan;
pub mod thumbnail;
pub mod watch;
pub mod wb_import;

mod menu;
mod startup;

use std::sync::Arc;

use tauri::Manager;

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let (app_dir, db) = startup::init_storage()?;
            let shared = startup::manage_shared_state(app, &db);
            startup::log_startup(&app_dir);

            let notifier = Arc::new(events::TauriCatalogNotifier::new(app.handle().clone()));
            let thumbnails_root = startup::prepare_thumbnails(&db, &app_dir)?;
            let services =
                startup::Services::new(app.handle().clone(), db, shared, notifier, thumbnails_root);
            startup::start_resume_passes(&services);

            let follow_up = startup::build_follow_up(&services);
            let registered = startup::wire_folder_state(&services, follow_up);
            let watch_manager = startup::start_watching(&services, registered)?;
            app.manage(watch_manager);

            let (menu, style_default_item) = menu::build_menu(app)?;
            app.set_menu(menu)?;
            app.on_menu_event(move |app_handle, event| {
                menu::handle_menu_event(app_handle, &event, &style_default_item);
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
            commands::view_state_cmds::get_restore_view_state_enabled,
            commands::view_state_cmds::set_restore_view_state_enabled,
            commands::view_state_cmds::load_view_state,
            commands::view_state_cmds::save_view_state,
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
            commands::tag_cmds::list_tags_with_video_count,
            commands::tag_cmds::delete_tag,
            commands::rating_cmds::set_rating,
            commands::properties_cmds::get_video_properties,
            commands::wb_import_cmds::pick_wb_file,
            commands::wb_import_cmds::pick_wb_thumbnail_folder,
            commands::wb_import_cmds::start_wb_import,
            commands::wb_import_cmds::open_wb_import_log,
            commands::wb_unmatched_cmds::list_unmatched_thumbnails,
            commands::wb_unmatched_cmds::search_link_candidates,
            commands::wb_unmatched_cmds::link_unmatched_thumbnail,
            commands::wb_unmatched_cmds::dismiss_unmatched_thumbnail,
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
