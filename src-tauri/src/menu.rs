use tauri::menu::{
    CheckMenuItem, CheckMenuItemBuilder, Menu, MenuBuilder, MenuEvent, MenuItemBuilder,
    PredefinedMenuItem, SubmenuBuilder,
};
use tauri::{Emitter, Manager};

use crate::{db, paths};

// Native menu item ids, matched against in `handle_menu_event` and
// `menu_event_name` below. Kept as constants rather than inline string
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
// (see `handle_menu_event` below). Extend this to real mutual-exclusion once a
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

/// Builds the menu bar but leaves installing it to the caller. The "Default"
/// style item is returned alongside because the event handler has to
/// re-assert its checked state, and only the built item can do that.
pub(crate) fn build_menu(
    app: &tauri::App,
) -> tauri::Result<(Menu<tauri::Wry>, CheckMenuItem<tauri::Wry>)> {
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
        MenuItemBuilder::with_id(MENU_ITEM_DB_BACKUP, "データベースをバックアップ").build(app)?;
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
    // item exists today (the tag editor: reorder and delete), but this gives
    // future tag-related menu entries a home without further
    // reshuffling "ファイル".
    let tag_bar_manage_item =
        MenuItemBuilder::with_id(MENU_ITEM_TAG_BAR_MANAGE, "タグの編集...").build(app)?;
    let tag_menu = SubmenuBuilder::new(app, "タグ")
        .item(&tag_bar_manage_item)
        .build()?;

    let style_default_item = CheckMenuItemBuilder::with_id(MENU_ITEM_STYLE_DEFAULT, "Default")
        .checked(true)
        .build(app)?;
    let style_menu = SubmenuBuilder::new(app, "スタイル")
        .item(&style_default_item)
        .build()?;

    let settings_item = MenuItemBuilder::with_id(MENU_ITEM_SETTINGS, "設定...").build(app)?;
    let settings_menu = SubmenuBuilder::new(app, "設定")
        .item(&settings_item)
        .build()?;

    let about_item = MenuItemBuilder::with_id(MENU_ITEM_ABOUT, "バージョン情報").build(app)?;
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
    Ok((menu, style_default_item))
}

/// The frontend event a payload-less menu item emits. Kept free of Tauri
/// types so the id-to-event table can be unit tested; the items that do more
/// than emit a payload-less event ("スタイル > Default" and the backup) are
/// handled by `handle_menu_event` before this is consulted, so they map to
/// `None` here just like an id that never reaches the listener.
fn menu_event_name(id: &str) -> Option<&'static str> {
    match id {
        id if id == MENU_ITEM_FOLDER_MANAGE => Some("menu:open-folder-dialog"),
        id if id == MENU_ITEM_WB_IMPORT => Some("menu:open-wb-import-dialog"),
        id if id == MENU_ITEM_TAG_BAR_MANAGE => Some("menu:open-tag-bar-dialog"),
        id if id == MENU_ITEM_SETTINGS => Some("menu:open-settings-dialog"),
        id if id == MENU_ITEM_LOG_VIEWER => Some("menu:open-log-dialog"),
        id if id == MENU_ITEM_CHECK_UPDATE => Some("menu:check-update"),
        id if id == MENU_ITEM_ABOUT => Some("menu:about"),
        // "終了" is Tauri's PredefinedMenuItem::quit -- it's
        // handled natively and never reaches this listener.
        _ => None,
    }
}

pub(crate) fn handle_menu_event(
    app_handle: &tauri::AppHandle,
    event: &MenuEvent,
    style_default_item: &CheckMenuItem<tauri::Wry>,
) {
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

    let Some(event_name) = menu_event_name(event.id().as_ref()) else {
        return;
    };
    if let Err(e) = app_handle.emit(event_name, ()) {
        log::error!("failed to emit {event_name}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_dialog_item_maps_to_its_frontend_event() {
        let cases = [
            (MENU_ITEM_FOLDER_MANAGE, "menu:open-folder-dialog"),
            (MENU_ITEM_WB_IMPORT, "menu:open-wb-import-dialog"),
            (MENU_ITEM_TAG_BAR_MANAGE, "menu:open-tag-bar-dialog"),
            (MENU_ITEM_SETTINGS, "menu:open-settings-dialog"),
            (MENU_ITEM_LOG_VIEWER, "menu:open-log-dialog"),
            (MENU_ITEM_CHECK_UPDATE, "menu:check-update"),
            (MENU_ITEM_ABOUT, "menu:about"),
        ];
        for (id, expected) in cases {
            assert_eq!(menu_event_name(id), Some(expected), "id {id}");
        }
    }

    #[test]
    fn items_handled_before_the_table_have_no_event_name() {
        assert_eq!(menu_event_name(MENU_ITEM_STYLE_DEFAULT), None);
        assert_eq!(menu_event_name(MENU_ITEM_DB_BACKUP), None);
    }

    #[test]
    fn unknown_ids_have_no_event_name() {
        for id in ["", "quit", "menu-unknown", "MENU-ABOUT", "menu-about "] {
            assert_eq!(menu_event_name(id), None, "id {id:?}");
        }
    }
}
