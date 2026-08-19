//! File-based logging: `GrayBrowser\logs\`, 10MB
//! rotation, 5 generations kept. DEBUG is off by default.
//!
//! Filenames are `flexi_logger`'s native `Naming::Numbers` scheme
//! (`app_rCURRENT.log` while active, rotated to `app_r00000.log`,
//! `app_r00001.log`, ...) rather than a more conventional logrotate-style
//! `app.log`/`app.log.1`-style suffix numbering -- the crate always inserts
//! a generation infix before the extension and has no logrotate-style
//! suffix-numbering mode. Confirmed with the user: the native scheme is
//! functionally equivalent (same 10MB/5-generation behavior) and is used
//! as-is rather than building a custom rotation writer to match a specific
//! example filename literally.

use std::path::Path;

use flexi_logger::{Cleanup, Criterion, FileSpec, Logger, LoggerHandle, Naming};

/// Failure here must not stop the app from starting -- logging is a
/// diagnostic aid, not a startup precondition. Errors are downgraded to a
/// stderr warning and `None` is returned; the caller is free to ignore it.
///
/// The returned handle is only a side effect of keeping `log`'s global
/// registration alive -- there's no runtime log-level toggling yet
/// (that would back a `GRAY_BROWSER_LOG` env var, but the settings screen
/// it would belong to doesn't exist yet), so
/// callers may just drop it or hold it as `let _ = logging::init(...)`.
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
