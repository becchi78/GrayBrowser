use gb_core::appearance::{Mode, Theme};
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
// Ids of the "スタイル" items are derived from the value string, so the id
// used to build an item and the one matched in the handler cannot drift.
const MENU_ITEM_THEME_PREFIX: &str = "menu-theme-";
const MENU_ITEM_MODE_PREFIX: &str = "menu-mode-";
const EVENT_THEME_SELECTED: &str = "menu:theme-selected";
const EVENT_MODE_SELECTED: &str = "menu:appearance-selected";

/// A selection in the "スタイル" menu. The two groups are mutually exclusive
/// only within themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppearanceChoice {
    Theme(Theme),
    Mode(Mode),
}

fn theme_item_id(theme: Theme) -> String {
    format!("{MENU_ITEM_THEME_PREFIX}{}", theme.as_str())
}

fn mode_item_id(mode: Mode) -> String {
    format!("{MENU_ITEM_MODE_PREFIX}{}", mode.as_str())
}

fn theme_label(theme: Theme) -> &'static str {
    match theme {
        Theme::Fluent => "Fluent",
        Theme::Cinema => "シネマ",
        Theme::Console => "コンソール",
    }
}

fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::System => "OSに従う",
        Mode::Light => "ライト",
        Mode::Dark => "ダーク",
    }
}

fn appearance_choice(id: &str) -> Option<AppearanceChoice> {
    if let Some(value) = id.strip_prefix(MENU_ITEM_THEME_PREFIX) {
        return Theme::parse(value).map(AppearanceChoice::Theme);
    }
    id.strip_prefix(MENU_ITEM_MODE_PREFIX)
        .and_then(Mode::parse)
        .map(AppearanceChoice::Mode)
}

/// The frontend event and its payload for a "スタイル" selection.
fn appearance_event(choice: AppearanceChoice) -> (&'static str, &'static str) {
    match choice {
        AppearanceChoice::Theme(t) => (EVENT_THEME_SELECTED, t.as_str()),
        AppearanceChoice::Mode(m) => (EVENT_MODE_SELECTED, m.as_str()),
    }
}

/// The check items of the "スタイル" menu, kept so a click can move the check
/// mark: a native check item toggles only itself, which would leave two items
/// of one group checked.
pub(crate) struct AppearanceMenu {
    themes: Vec<(Theme, CheckMenuItem<tauri::Wry>)>,
    modes: Vec<(Mode, CheckMenuItem<tauri::Wry>)>,
}

impl AppearanceMenu {
    fn select(&self, choice: AppearanceChoice) {
        let results = match choice {
            AppearanceChoice::Theme(selected) => self
                .themes
                .iter()
                .map(|(t, item)| item.set_checked(*t == selected))
                .collect::<Vec<_>>(),
            AppearanceChoice::Mode(selected) => self
                .modes
                .iter()
                .map(|(m, item)| item.set_checked(*m == selected))
                .collect::<Vec<_>>(),
        };
        for e in results.into_iter().filter_map(Result::err) {
            log::error!("failed to update the スタイル menu check state: {e}");
        }
    }
}

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

/// Builds the menu bar but leaves installing it to the caller. The "スタイル"
/// check items are returned alongside because the event handler has to move
/// their check marks, and only the built items can do that. `theme`/`mode`
/// are the stored choices, so the menu starts out matching them.
pub(crate) fn build_menu(
    app: &tauri::App,
    theme: Theme,
    mode: Mode,
) -> tauri::Result<(Menu<tauri::Wry>, AppearanceMenu)> {
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

    let mut theme_items = Vec::new();
    for t in Theme::ALL {
        let item = CheckMenuItemBuilder::with_id(theme_item_id(t), theme_label(t))
            .checked(t == theme)
            .build(app)?;
        theme_items.push((t, item));
    }
    let mut mode_items = Vec::new();
    for m in Mode::ALL {
        let item = CheckMenuItemBuilder::with_id(mode_item_id(m), mode_label(m))
            .checked(m == mode)
            .build(app)?;
        mode_items.push((m, item));
    }
    let mut style_builder = SubmenuBuilder::new(app, "スタイル");
    for (_, item) in &theme_items {
        style_builder = style_builder.item(item);
    }
    style_builder = style_builder.separator();
    for (_, item) in &mode_items {
        style_builder = style_builder.item(item);
    }
    let style_menu = style_builder.build()?;

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
    Ok((
        menu,
        AppearanceMenu {
            themes: theme_items,
            modes: mode_items,
        },
    ))
}

/// The frontend event a payload-less menu item emits. Kept free of Tauri
/// types so the id-to-event table can be unit tested; the items that do more
/// than emit a payload-less event ("スタイル" selections and the backup) are
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
    appearance_menu: &AppearanceMenu,
) {
    // "スタイル" items carry a string payload (unlike the payload-less
    // dialog events below) and move their own group's check mark.
    if let Some(choice) = appearance_choice(event.id().as_ref()) {
        appearance_menu.select(choice);
        let (event_name, value) = appearance_event(choice);
        if let Err(e) = app_handle.emit(event_name, value) {
            log::error!("failed to emit {event_name}: {e}");
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
        assert_eq!(menu_event_name(MENU_ITEM_DB_BACKUP), None);
        for t in Theme::ALL {
            assert_eq!(menu_event_name(&theme_item_id(t)), None);
        }
        for m in Mode::ALL {
            assert_eq!(menu_event_name(&mode_item_id(m)), None);
        }
    }

    #[test]
    fn every_style_item_maps_to_its_group_event_and_value() {
        for t in Theme::ALL {
            let choice = appearance_choice(&theme_item_id(t));
            assert_eq!(choice, Some(AppearanceChoice::Theme(t)));
            assert_eq!(
                appearance_event(choice.unwrap()),
                ("menu:theme-selected", t.as_str())
            );
        }
        for m in Mode::ALL {
            let choice = appearance_choice(&mode_item_id(m));
            assert_eq!(choice, Some(AppearanceChoice::Mode(m)));
            assert_eq!(
                appearance_event(choice.unwrap()),
                ("menu:appearance-selected", m.as_str())
            );
        }
    }

    #[test]
    fn the_theme_and_mode_ids_do_not_overlap() {
        let mut ids: Vec<String> = Theme::ALL.iter().map(|t| theme_item_id(*t)).collect();
        ids.extend(Mode::ALL.iter().map(|m| mode_item_id(*m)));
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn unknown_or_foreign_ids_are_not_style_choices() {
        for id in [
            "menu-theme-",
            "menu-theme-neon",
            "menu-mode-auto",
            "menu-style-default",
            "menu-settings",
            "",
        ] {
            assert_eq!(appearance_choice(id), None, "{id:?}");
        }
    }
}
