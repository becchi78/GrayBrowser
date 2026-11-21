//! The main window size restored at the next launch. Pure parsing and
//! clamping only; storage and the window calls live in `src-tauri`.

use serde::{Deserialize, Serialize};

/// Sizes are logical pixels so a restore stays correct when the display scale
/// differs from the previous session. `width`/`height` are always the
/// non-maximized size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedWindowState {
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

/// A missing or corrupt value reads as `None` (use the default size): a
/// hand-edited or half-written value must never block startup.
pub fn parse_saved(raw: Option<&str>) -> Option<SavedWindowState> {
    raw.and_then(|s| serde_json::from_str(s).ok())
}

/// Size of a fresh window. Must match `width`/`height` in tauri.conf.json: it
/// is the normal size used when the first thing the user does is maximize, so
/// un-maximizing returns to the size a new window would have had.
pub const DEFAULT_SIZE: (u32, u32) = (1280, 900);

/// Clamps the size into `[min, work_area]`. The minimum wins when the work
/// area is smaller than it, because the window cannot shrink below its
/// minimum anyway. Zero and extreme values are covered by the same clamp.
pub fn fit(saved: SavedWindowState, min: (u32, u32), work_area: (u32, u32)) -> SavedWindowState {
    SavedWindowState {
        width: saved.width.min(work_area.0).max(min.0),
        height: saved.height.min(work_area.1).max(min.1),
        maximized: saved.maximized,
    }
}

/// While maximized, the stored normal size is kept (the current size is the
/// screen size and would make "un-maximize" a no-op at the next launch). With
/// nothing stored yet it falls back to `DEFAULT_SIZE` for the same reason.
pub fn next_state(
    prev: Option<SavedWindowState>,
    current: (u32, u32),
    maximized: bool,
) -> SavedWindowState {
    let (width, height) = match (maximized, prev) {
        (true, Some(p)) => (p.width, p.height),
        (true, None) => DEFAULT_SIZE,
        (false, _) => current,
    };
    SavedWindowState {
        width,
        height,
        maximized,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: (u32, u32) = (900, 580);
    const WORK: (u32, u32) = (1920, 1040);

    fn st(width: u32, height: u32, maximized: bool) -> SavedWindowState {
        SavedWindowState {
            width,
            height,
            maximized,
        }
    }

    #[test]
    fn parse_reads_a_valid_value() {
        let raw = r#"{"width":1000,"height":700,"maximized":true}"#;
        assert_eq!(parse_saved(Some(raw)), Some(st(1000, 700, true)));
    }

    #[test]
    fn parse_returns_none_for_missing_or_broken_values() {
        assert_eq!(parse_saved(None), None);
        assert_eq!(parse_saved(Some("")), None);
        assert_eq!(parse_saved(Some("{broken")), None);
        assert_eq!(parse_saved(Some(r#"{"width":1000}"#)), None);
        assert_eq!(
            parse_saved(Some(r#"{"width":-1,"height":700,"maximized":false}"#)),
            None
        );
        assert_eq!(
            parse_saved(Some(r#"{"width":"a","height":700,"maximized":false}"#)),
            None
        );
    }

    #[test]
    fn fit_keeps_a_size_inside_the_range() {
        assert_eq!(fit(st(1280, 900, false), MIN, WORK), st(1280, 900, false));
    }

    #[test]
    fn fit_raises_below_minimum_and_lowers_above_work_area() {
        assert_eq!(fit(st(100, 100, false), MIN, WORK), st(900, 580, false));
        assert_eq!(fit(st(5000, 4000, true), MIN, WORK), st(1920, 1040, true));
        assert_eq!(fit(st(5000, 300, false), MIN, WORK), st(1920, 580, false));
    }

    #[test]
    fn fit_handles_zero_and_extreme_values() {
        assert_eq!(fit(st(0, 0, false), MIN, WORK), st(900, 580, false));
        assert_eq!(
            fit(st(u32::MAX, u32::MAX, false), MIN, WORK),
            st(1920, 1040, false)
        );
    }

    #[test]
    fn fit_boundaries_are_inclusive() {
        assert_eq!(fit(st(900, 580, false), MIN, WORK), st(900, 580, false));
        assert_eq!(fit(st(1920, 1040, false), MIN, WORK), st(1920, 1040, false));
        assert_eq!(fit(st(899, 579, false), MIN, WORK), st(900, 580, false));
        assert_eq!(fit(st(1921, 1041, false), MIN, WORK), st(1920, 1040, false));
    }

    #[test]
    fn fit_prefers_the_minimum_when_the_work_area_is_smaller() {
        assert_eq!(
            fit(st(1280, 900, false), MIN, (800, 500)),
            st(900, 580, false)
        );
    }

    #[test]
    fn next_state_saves_the_current_size_when_not_maximized() {
        assert_eq!(
            next_state(Some(st(1000, 700, false)), (1100, 800), false),
            st(1100, 800, false)
        );
        assert_eq!(next_state(None, (1100, 800), false), st(1100, 800, false));
    }

    #[test]
    fn next_state_keeps_the_normal_size_while_maximized() {
        assert_eq!(
            next_state(Some(st(1000, 700, false)), (1920, 1040), true),
            st(1000, 700, true)
        );
        // Repeated maximized saves must not drift.
        let once = next_state(Some(st(1000, 700, true)), (1920, 1040), true);
        assert_eq!(once, st(1000, 700, true));
    }

    #[test]
    fn next_state_uses_the_default_size_when_maximized_without_history() {
        assert_eq!(
            next_state(None, (1920, 1040), true),
            st(DEFAULT_SIZE.0, DEFAULT_SIZE.1, true)
        );
    }

    #[test]
    fn unmaximizing_without_history_returns_to_the_default_size() {
        let maximized = next_state(None, (1920, 1040), true);
        let restored = next_state(Some(maximized), (DEFAULT_SIZE.0, DEFAULT_SIZE.1), false);
        assert_eq!(restored, st(DEFAULT_SIZE.0, DEFAULT_SIZE.1, false));
    }

    #[test]
    fn unmaximizing_returns_to_the_previous_normal_size() {
        let maximized = next_state(Some(st(1000, 700, false)), (1920, 1040), true);
        let restored = next_state(Some(maximized), (1000, 700), false);
        assert_eq!(restored, st(1000, 700, false));
    }
}
