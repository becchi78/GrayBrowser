//! Applies the remembered size to the main window and keeps it up to date.
//! Only thin glue over Tauri window calls; the decisions live in
//! `gb_core::window_state` and `commands::window_state_cmds`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use gb_core::window_state::fit;
use tauri::{LogicalSize, Manager, WebviewWindow, WindowEvent};

use crate::commands::window_state_cmds;
use crate::db::Db;

const MAIN_WINDOW_LABEL: &str = "main";

/// Must match `minWidth`/`minHeight` in tauri.conf.json.
const MIN_SIZE: (u32, u32) = (900, 580);

/// Resize events arrive in a stream while dragging; saving once the stream has
/// been quiet this long avoids a DB write per pixel.
const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

/// The window is created hidden so a restore never flashes the default size.
/// `show()` is reached whether or not the restore succeeded: a failed restore
/// must only cost the remembered size, never leave the app without a window.
pub fn restore_and_show(app: &tauri::App) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        log::error!("main window not found; cannot restore or show it");
        return;
    };
    if let Err(e) = restore(&window, &app.state::<Db>()) {
        log::warn!("failed to restore window size: {e}");
    }
    if let Err(e) = window.show() {
        log::error!("failed to show main window: {e}");
    }
    // Window events are delivered asynchronously, so a Resized caused by the
    // restore can still arrive after this registration. That is harmless: the
    // debounced save records the window's actual state.
    watch_for_changes(&window, app.state::<Db>().inner().clone());
}

fn restore(window: &WebviewWindow, db: &Db) -> anyhow::Result<()> {
    let Some(saved) = window_state_cmds::load(db)? else {
        return Ok(());
    };
    let scale = window.scale_factor()?;
    let monitor = match window.current_monitor()? {
        Some(m) => Some(m),
        None => window.primary_monitor()?,
    };
    // Without a monitor the upper bound is unknown; only the minimum is
    // enforced so a bogus stored value still cannot produce a tiny window.
    let work_area = match monitor {
        Some(m) => {
            let size = m.work_area().size;
            (
                (f64::from(size.width) / scale).round() as u32,
                (f64::from(size.height) / scale).round() as u32,
            )
        }
        None => (u32::MAX, u32::MAX),
    };
    let fitted = fit(saved, MIN_SIZE, work_area);
    window.set_size(LogicalSize::new(
        f64::from(fitted.width),
        f64::from(fitted.height),
    ))?;
    window.center()?;
    if fitted.maximized {
        window.maximize()?;
    }
    Ok(())
}

fn watch_for_changes(window: &WebviewWindow, db: Db) {
    // Bumped on every event so a pending debounced save knows it was
    // superseded (by a newer resize or by the close-time save).
    let generation = Arc::new(AtomicU64::new(0));
    let watched = window.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Resized(_) => {
            let mine = generation.fetch_add(1, Ordering::SeqCst) + 1;
            let generation = Arc::clone(&generation);
            let window = watched.clone();
            let db = db.clone();
            std::thread::spawn(move || {
                std::thread::sleep(SAVE_DEBOUNCE);
                if generation.load(Ordering::SeqCst) == mine {
                    save_current(&window, &db);
                }
            });
        }
        WindowEvent::CloseRequested { .. } => {
            generation.fetch_add(1, Ordering::SeqCst);
            save_current(&watched, &db);
        }
        _ => {}
    });
}

fn save_current(window: &WebviewWindow, db: &Db) {
    if let Err(e) = try_save_current(window, db) {
        log::warn!("failed to save window size: {e}");
    }
}

fn try_save_current(window: &WebviewWindow, db: &Db) -> anyhow::Result<()> {
    // A minimized window reports a meaningless (often 0x0) size.
    if window.is_minimized()? {
        return Ok(());
    }
    let size = window.inner_size()?;
    if size.width == 0 || size.height == 0 {
        return Ok(());
    }
    let scale = window.scale_factor()?;
    let logical = (
        (f64::from(size.width) / scale).round() as u32,
        (f64::from(size.height) / scale).round() as u32,
    );
    window_state_cmds::save(db, logical, window.is_maximized()?)
}

#[cfg(test)]
mod tests {
    use super::MIN_SIZE;
    use gb_core::window_state::DEFAULT_SIZE;

    #[test]
    fn size_constants_match_tauri_conf() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let w = &conf["app"]["windows"][0];
        let n = |k: &str| w[k].as_u64().unwrap() as u32;
        assert_eq!(MIN_SIZE, (n("minWidth"), n("minHeight")));
        assert_eq!(DEFAULT_SIZE, (n("width"), n("height")));
    }
}
