//! The diagnostics report shown in developer mode. The wrapper only gathers
//! facts from the environment; the report itself is built by a function that
//! takes the FFmpeg adapter, so it can be tested without a Tauri runtime or
//! real binaries.

use gb_core::diagnostics::{format_report, DiagnosticsInfo, MaskRule};
use gb_core::ports::ffmpeg::FfmpegAdapter;
use gb_core::release_status::ReleaseStatus;
use tauri::{State, Webview};

use super::developer_cmds::ensure_developer_mode;
use crate::adapters;
use crate::db::{backup, Db};

/// What the report needs from the running app, collected by the wrapper.
struct DiagnosticsEnv {
    app_version: String,
    release_status: ReleaseStatus,
    schema_version: Option<i64>,
    webview_version: Option<String>,
    os: String,
    db_path: String,
    data_dir: String,
    log_dir: String,
    mask_rules: Vec<MaskRule>,
}

/// Refused unless developer mode is on, like the other developer-only
/// commands, so the report is not reachable by skipping the menu.
fn guarded_report(
    conn: &rusqlite::Connection,
    adapter: &impl FfmpegAdapter,
    env: DiagnosticsEnv,
) -> Result<String, String> {
    ensure_developer_mode(conn)?;
    Ok(build_report(adapter, env))
}

fn build_report(adapter: &impl FfmpegAdapter, env: DiagnosticsEnv) -> String {
    // A failed check is reported as "unknown" like a missing binary: the
    // report should still be produced when FFmpeg is the thing that is broken.
    let (ffmpeg_version, ffprobe_version) = match adapter.check_available() {
        Ok(a) => (a.ffmpeg_version, a.ffprobe_version),
        Err(_) => (None, None),
    };
    let info = DiagnosticsInfo {
        app_version: env.app_version,
        release_status: env.release_status,
        schema_version: env.schema_version,
        ffmpeg_version,
        ffprobe_version,
        webview_version: env.webview_version,
        os: env.os,
        db_path: env.db_path,
        data_dir: env.data_dir,
        log_dir: env.log_dir,
    };
    format_report(&info, &env.mask_rules)
}

/// Folders under the user's profile are replaced by the variable that names
/// them, so a report pasted into an issue does not reveal the account name.
pub(crate) fn mask_rules_from_env() -> Vec<MaskRule> {
    [
        ("LOCALAPPDATA", "%LOCALAPPDATA%"),
        ("APPDATA", "%APPDATA%"),
        ("USERPROFILE", "%USERPROFILE%"),
    ]
    .into_iter()
    .filter_map(|(var, placeholder)| {
        std::env::var(var).ok().map(|prefix| MaskRule {
            prefix,
            placeholder: placeholder.to_string(),
        })
    })
    .collect()
}

/// The folder in use, shown beside the developer-mode badge so a session on a
/// substitute folder is never mistaken for the real one.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct DataDirInfo {
    pub path: String,
    pub overridden: bool,
}

/// Same developer-mode refusal and the same path masking as the diagnostics
/// report, even though it only reads: the folder path can carry the account
/// name.
fn guarded_data_dir_info(
    conn: &rusqlite::Connection,
    dir: &std::path::Path,
    overridden: bool,
    rules: &[MaskRule],
) -> Result<DataDirInfo, String> {
    ensure_developer_mode(conn)?;
    Ok(DataDirInfo {
        path: gb_core::diagnostics::mask_paths(&dir.display().to_string(), rules),
        overridden,
    })
}

#[tauri::command]
pub fn get_data_dir_info(db: State<'_, Db>) -> Result<DataDirInfo, String> {
    let conn = db.read_pool.get().map_err(|e| e.to_string())?;
    let dir = crate::paths::app_data_dir().map_err(|e| e.to_string())?;
    guarded_data_dir_info(
        &conn,
        &dir,
        crate::paths::data_dir_overridden(),
        &mask_rules_from_env(),
    )
}

/// Async because the release status is a network lookup; the FFmpeg version
/// probes block, so they run on a blocking worker rather than the async
/// runtime's threads. Developer mode is checked before any network access.
#[tauri::command]
pub async fn get_diagnostics(webview: Webview, db: State<'_, Db>) -> Result<String, String> {
    let pool = db.read_pool.clone();
    {
        let conn = pool.get().map_err(|e| e.to_string())?;
        ensure_developer_mode(&conn)?;
    }
    let release_status = super::update_cmds::current_release_status(&webview).await;
    tauri::async_runtime::spawn_blocking(move || {
        let app_dir = crate::paths::app_data_dir().map_err(|e| e.to_string())?;
        let conn = pool.get().map_err(|e| e.to_string())?;
        ensure_developer_mode(&conn)?;
        let schema_version = backup::current_schema_version(&conn).ok();
        let env = DiagnosticsEnv {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            release_status,
            schema_version,
            webview_version: tauri::webview_version().ok(),
            os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            db_path: app_dir.join("app.db").display().to_string(),
            data_dir: app_dir.display().to_string(),
            log_dir: app_dir.join("logs").display().to_string(),
            mask_rules: mask_rules_from_env(),
        };
        guarded_report(&conn, &adapters::ffmpeg::RealFfmpegAdapter, env)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use gb_core::ports::ffmpeg::{FfmpegAvailability, FfmpegError};
    use gb_core::testing::fake_ffmpeg::FakeFfmpegAdapter;

    fn env() -> DiagnosticsEnv {
        DiagnosticsEnv {
            app_version: "0.18.1".into(),
            release_status: ReleaseStatus::Published,
            schema_version: Some(17),
            webview_version: Some("130.0".into()),
            os: "windows x86_64".into(),
            db_path: r"C:\Users\alice\AppData\Local\GrayBrowser\app.db".into(),
            data_dir: r"C:\Users\alice\AppData\Local\GrayBrowser".into(),
            log_dir: r"C:\Users\alice\AppData\Local\GrayBrowser\logs".into(),
            mask_rules: vec![MaskRule {
                prefix: r"C:\Users\alice\AppData\Local".into(),
                placeholder: "%LOCALAPPDATA%".into(),
            }],
        }
    }

    #[test]
    fn reports_the_versions_the_adapter_finds_and_masks_paths() {
        let fake = FakeFfmpegAdapter {
            availability: Ok(FfmpegAvailability {
                ffmpeg_version: Some("ffmpeg version 7.0".into()),
                ffprobe_version: Some("ffprobe version 7.0".into()),
            }),
            ..Default::default()
        };
        let report = build_report(&fake, env());
        assert!(report.contains("FFmpeg: ffmpeg version 7.0"));
        assert!(report.contains("FFprobe: ffprobe version 7.0"));
        assert!(report.contains(r"Log folder: %LOCALAPPDATA%\GrayBrowser\logs"));
        assert!(!report.contains("alice"));
    }

    #[test]
    fn the_report_carries_the_release_status() {
        let fake = FakeFfmpegAdapter::default();
        let mut e = env();
        e.release_status = ReleaseStatus::Preview;
        assert!(build_report(&fake, e).contains("Release status: draft (not yet published)"));
    }

    #[test]
    fn the_report_is_refused_unless_developer_mode_is_on() {
        use crate::db::{queries, test_support::init_temp_db};
        let (_d, db) = init_temp_db();
        let conn = db.writer.lock().unwrap();
        let fake = FakeFfmpegAdapter::default();
        assert!(guarded_report(&conn, &fake, env()).is_err());
        queries::set_developer_mode(&conn, true).unwrap();
        assert!(guarded_report(&conn, &fake, env()).is_ok());
    }

    #[test]
    fn data_dir_info_is_refused_unless_developer_mode_is_on() {
        use crate::db::{queries, test_support::init_temp_db};
        let (_d, db) = init_temp_db();
        let conn = db.writer.lock().unwrap();
        let dir = std::path::Path::new(r"C:\Users\alice\AppData\Local\GrayBrowser");
        assert!(guarded_data_dir_info(&conn, dir, false, &[]).is_err());
        queries::set_developer_mode(&conn, true).unwrap();
        assert!(guarded_data_dir_info(&conn, dir, false, &[]).is_ok());
    }

    #[test]
    fn data_dir_info_masks_the_path_and_carries_the_override_flag() {
        use crate::db::{queries, test_support::init_temp_db};
        let (_d, db) = init_temp_db();
        let conn = db.writer.lock().unwrap();
        queries::set_developer_mode(&conn, true).unwrap();
        let rules = vec![MaskRule {
            prefix: r"C:\Users\alice\AppData\Local".into(),
            placeholder: "%LOCALAPPDATA%".into(),
        }];
        let dir = std::path::Path::new(r"C:\Users\alice\AppData\Local\GrayBrowser");
        let info = guarded_data_dir_info(&conn, dir, true, &rules).unwrap();
        assert_eq!(
            info,
            DataDirInfo {
                path: r"%LOCALAPPDATA%\GrayBrowser".into(),
                overridden: true
            }
        );
    }

    #[test]
    fn a_failed_ffmpeg_check_still_yields_a_report() {
        let fake = FakeFfmpegAdapter {
            availability: Err(FfmpegError::Spawn("not found".into())),
            ..Default::default()
        };
        let report = build_report(&fake, env());
        assert!(report.contains("FFmpeg: unknown"));
        assert!(report.contains("App version: 0.18.1"));
    }

    #[test]
    fn a_missing_ffprobe_is_reported_as_unknown_on_its_own() {
        let fake = FakeFfmpegAdapter {
            availability: Ok(FfmpegAvailability {
                ffmpeg_version: Some("ffmpeg version 7.0".into()),
                ffprobe_version: None,
            }),
            ..Default::default()
        };
        let report = build_report(&fake, env());
        assert!(report.contains("FFmpeg: ffmpeg version 7.0"));
        assert!(report.contains("FFprobe: unknown"));
    }
}
