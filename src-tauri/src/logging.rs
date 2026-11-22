//! File-based logging: `GrayBrowser\logs\`, 10MB
//! rotation, 5 generations kept. DEBUG is off unless developer mode is on and
//! a more verbose level was chosen.
//!
//! Filenames are `flexi_logger`'s native `Naming::Numbers` scheme
//! (`app_rCURRENT.log` while active, rotated to `app_r00000.log`,
//! `app_r00001.log`, ...) rather than a more conventional logrotate-style
//! `app.log`/`app.log.1`-style suffix numbering -- the crate always inserts
//! a generation infix before the extension and has no logrotate-style
//! suffix-numbering mode. The native scheme is functionally equivalent (same
//! 10MB/5-generation behavior), so it is used as-is rather than building a
//! custom rotation writer to match a specific filename literally.

use std::path::Path;
use std::sync::Mutex;

use flexi_logger::{Cleanup, Criterion, FileSpec, LogSpecification, Logger, LoggerHandle, Naming};
use gb_core::log_level::LogLevel;

/// Failure here must not stop the app from starting -- logging is a
/// diagnostic aid, not a startup precondition. Errors are downgraded to a
/// stderr warning and `None` is returned; the caller is free to ignore it.
///
/// Starts at "info"; the caller keeps the returned handle (in a
/// `LogLevelControl`) so the level can be changed at runtime. Dropping the
/// handle would shut the logger down.
pub fn init(app_dir: &Path) -> Option<LoggerHandle> {
    let logs_dir = app_dir.join("logs");
    let result = Logger::try_with_str("info").and_then(|logger| {
        logger
            .log_to_file(
                FileSpec::default()
                    .directory(&logs_dir)
                    .basename("app")
                    .suppress_timestamp(),
            )
            .rotate(
                Criterion::Size(10 * 1024 * 1024),
                Naming::Numbers,
                Cleanup::KeepLogFiles(5),
            )
            .start()
    });
    match result {
        Ok(handle) => Some(handle),
        Err(e) => {
            eprintln!("warning: failed to initialize file logging: {e}");
            None
        }
    }
}

/// The filter spec for `level`. Only this app's own crates get the verbose
/// level: at debug or trace, the web stack's own output would drown the
/// entries a developer is looking for.
fn spec_for(level: LogLevel) -> String {
    let l = level.as_str();
    format!("info,graybrowser_lib={l},gb_core={l}")
}

/// Holds the logger handle so the level can change while the app runs.
/// Managed as Tauri state; `None` inside means file logging failed to start,
/// in which case there is nothing to adjust.
pub struct LogLevelControl {
    handle: Mutex<Option<LoggerHandle>>,
}

impl LogLevelControl {
    pub fn new(handle: Option<LoggerHandle>) -> Self {
        Self {
            handle: Mutex::new(handle),
        }
    }

    pub fn apply(&self, level: LogLevel) {
        let guard = self.handle.lock().unwrap_or_else(|e| e.into_inner());
        let Some(handle) = guard.as_ref() else {
            return;
        };
        match LogSpecification::parse(spec_for(level)) {
            Ok(spec) => {
                handle.set_new_spec(spec);
                log::info!("log level set to {}", level.as_str());
            }
            Err(e) => log::warn!("failed to change the log level: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_level_yields_a_spec_flexi_logger_accepts() {
        for level in LogLevel::ALL {
            assert!(
                LogSpecification::parse(spec_for(level)).is_ok(),
                "{level:?}"
            );
        }
    }

    #[test]
    fn only_this_apps_crates_get_the_chosen_level() {
        assert_eq!(
            spec_for(LogLevel::Debug),
            "info,graybrowser_lib=debug,gb_core=debug"
        );
    }

    #[test]
    fn applying_without_a_logger_does_nothing() {
        LogLevelControl::new(None).apply(LogLevel::Trace);
    }
}
