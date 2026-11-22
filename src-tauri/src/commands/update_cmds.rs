//! In-app update commands: the startup-check preference and the update check
//! itself. Download and install go through the updater plugin's own
//! `download_and_install` command on the resource this module registers.

use serde::Serialize;
use tauri::{Manager, State, Webview};
use tauri_plugin_updater::UpdaterExt;

use gb_core::release_status::{classify, ReleaseStatus};

use super::developer_cmds::ensure_developer_mode;
use super::log_cmds::apply_effective_log_level;
use crate::db::{queries, Db};
use crate::logging::LogLevelControl;

/// Upper bound for the update check request. Without it a stalled connection
/// would leave the manual-check dialog on "checking" forever.
const CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

#[tauri::command]
pub fn get_update_check_on_startup(db: State<Db>) -> Result<bool, String> {
    let conn = db.read_pool.get().map_err(|e| e.to_string())?;
    queries::get_update_check_on_startup(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_update_check_on_startup(db: State<Db>, enabled: bool) -> Result<(), String> {
    let conn = db.writer.lock().unwrap();
    queries::set_update_check_on_startup(&conn, enabled).map_err(|e| e.to_string())
}

/// Same shape as the updater plugin's JS `UpdateMetadata`, so the frontend can
/// wrap it in the plugin's `Update` class and call `downloadAndInstall` on the
/// registered resource.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMetadata {
    rid: tauri::ResourceId,
    current_version: String,
    version: String,
    body: Option<String>,
    raw_json: serde_json::Value,
    /// "stable" or "preview" (see `Channel::as_str`); lets the frontend say so
    /// when the offered build is not yet published.
    channel: &'static str,
}

/// Manifest of the preview channel. A compile-time constant on purpose:
/// the frontend can only toggle developer mode, never choose where updates
/// come from. Updates from it are verified against the same public key.
const DEV_CHANNEL_ENDPOINT: &str =
    "https://github.com/becchi78/GrayBrowser/releases/download/dev/latest.json";

#[tauri::command]
pub fn get_developer_mode(db: State<Db>) -> Result<bool, String> {
    let conn = db.read_pool.get().map_err(|e| e.to_string())?;
    queries::get_developer_mode(&conn).map_err(|e| e.to_string())
}

/// Turning the mode off also drops the log level back to `info` right away,
/// so a verbose level chosen earlier does not keep running unseen.
#[tauri::command]
pub fn set_developer_mode(
    db: State<Db>,
    control: State<LogLevelControl>,
    enabled: bool,
) -> Result<(), String> {
    let conn = db.writer.lock().unwrap();
    queries::set_developer_mode(&conn, enabled).map_err(|e| e.to_string())?;
    apply_effective_log_level(&conn, |l| control.apply(l)).map_err(|e| e.to_string())
}

/// Checks for a newer release. Own command instead of the plugin's JS
/// `check()` so that the frontend is never granted `updater:allow-check`,
/// whose `target`/`headers`/`proxy` arguments it could otherwise choose freely;
/// the endpoint and public key stay fixed to the build-time configuration.
///
/// With developer mode on, the preview channel is checked too and only the
/// newer of the two updates is returned (and registered as a resource).
#[tauri::command]
pub async fn check_for_update(
    webview: Webview,
    db: State<'_, Db>,
) -> Result<Option<UpdateMetadata>, String> {
    use gb_core::update_channel::{pick_newer, Channel};

    let developer_mode = {
        let conn = db.read_pool.get().map_err(|e| e.to_string())?;
        queries::get_developer_mode(&conn).map_err(|e| e.to_string())?
    };

    let stable = check_channel(&webview, None).await?;
    let preview = if developer_mode {
        let endpoint = DEV_CHANNEL_ENDPOINT
            .parse::<tauri::Url>()
            .map_err(|e| e.to_string())?;
        // A missing or unreachable preview channel must not hide a stable
        // update, so its failure is logged and treated as "no update".
        check_channel(&webview, Some(endpoint))
            .await
            .unwrap_or_else(|e| {
                log::warn!("preview channel update check failed: {e}");
                None
            })
    } else {
        None
    };

    let chosen = match pick_newer(
        stable.as_ref().map(|u| u.version.as_str()),
        preview.as_ref().map(|u| u.version.as_str()),
    ) {
        Ok(choice) => choice,
        Err(e) => {
            // Never guess between channels on versions we cannot order.
            log::warn!("cannot compare update versions, offering stable: {e}");
            stable.is_some().then_some(Channel::Stable)
        }
    };
    // The update that is not chosen is dropped here without ever entering the
    // resources table, so nothing is left to release.
    let (update, channel) = match chosen {
        Some(Channel::Stable) => (stable, Channel::Stable),
        Some(Channel::Preview) => (preview, Channel::Preview),
        None => (None, Channel::Stable),
    };
    Ok(update.map(|update| UpdateMetadata {
        channel: channel.as_str(),
        current_version: update.current_version.clone(),
        version: update.version.clone(),
        body: update.body.clone(),
        raw_json: update.raw_json.clone(),
        rid: webview.resources_table().add(update),
    }))
}

/// Version of the latest published stable release, or `None` when it cannot be
/// obtained. The comparator accepts every manifest, so the plugin returns the
/// published release even when it is not newer than the running build (which is
/// exactly the case this lookup exists for). Always the build-time stable
/// endpoint: nothing here takes a URL from the frontend.
async fn fetch_latest_stable_version(webview: &Webview) -> Result<Option<String>, String> {
    let builder = webview
        .updater_builder()
        .timeout(CHECK_TIMEOUT)
        .version_comparator(|_, _| true);
    let builder = debug_overrides::apply(builder)?;
    let updater = builder.build().map_err(|e| e.to_string())?;
    Ok(updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .map(|u| u.version))
}

/// A failed lookup is `Unknown`, never an error: being offline must not break
/// the About dialog or the diagnostics report.
fn status_from_lookup(running: &str, lookup: Result<Option<String>, String>) -> ReleaseStatus {
    match lookup {
        Ok(version) => classify(running, version.as_deref()),
        Err(e) => {
            log::warn!("latest stable version lookup failed: {e}");
            ReleaseStatus::Unknown
        }
    }
}

/// Status of the running build against the latest published stable release.
pub(crate) async fn current_release_status(webview: &Webview) -> ReleaseStatus {
    let lookup = fetch_latest_stable_version(webview).await;
    status_from_lookup(env!("CARGO_PKG_VERSION"), lookup)
}

/// Developer-mode only; refused before any network access when it is off.
#[tauri::command]
pub async fn get_release_status(
    webview: Webview,
    db: State<'_, Db>,
) -> Result<&'static str, String> {
    {
        let conn = db.read_pool.get().map_err(|e| e.to_string())?;
        ensure_developer_mode(&conn)?;
    }
    Ok(current_release_status(&webview).await.as_str())
}

/// `endpoint: None` uses the endpoints from the build-time configuration. The
/// debug override is applied last so it redirects both channels' checks to the
/// same local server.
async fn check_channel(
    webview: &Webview,
    endpoint: Option<tauri::Url>,
) -> Result<Option<tauri_plugin_updater::Update>, String> {
    let mut builder = webview.updater_builder().timeout(CHECK_TIMEOUT);
    if let Some(url) = endpoint {
        builder = builder.endpoints(vec![url]).map_err(|e| e.to_string())?;
    }
    let builder = debug_overrides::apply(builder)?;
    let updater = builder.build().map_err(|e| e.to_string())?;
    updater.check().await.map_err(|e| e.to_string())
}

/// Verification-only endpoint/public-key overrides read from the environment,
/// so the update flow can be exercised against a local server and a throwaway
/// key without touching the real release channel. They exist only when
/// `debug_assertions` is on: the release variant of `apply` is an identity
/// function that never reads the environment and carries none of these names,
/// so a shipped binary cannot be redirected to another update source.
mod debug_overrides {
    use tauri_plugin_updater::UpdaterBuilder;

    #[cfg(debug_assertions)]
    pub fn apply(builder: UpdaterBuilder) -> Result<UpdaterBuilder, String> {
        let overrides = parse(|name| std::env::var(name).ok())?;
        let mut builder = builder;
        if let Some(url) = overrides.endpoint {
            builder = builder.endpoints(vec![url]).map_err(|e| e.to_string())?;
            log::warn!("update check endpoint overridden by {ENDPOINT_VAR}");
        }
        if let Some(key) = overrides.pubkey {
            builder = builder.pubkey(key);
            log::warn!("update signature public key overridden by {PUBKEY_VAR}");
        }
        Ok(builder)
    }

    #[cfg(not(debug_assertions))]
    pub fn apply(builder: UpdaterBuilder) -> Result<UpdaterBuilder, String> {
        Ok(builder)
    }

    #[cfg(debug_assertions)]
    pub(super) const ENDPOINT_VAR: &str = "GB_UPDATE_ENDPOINT";
    #[cfg(debug_assertions)]
    pub(super) const PUBKEY_VAR: &str = "GB_UPDATE_PUBKEY";

    #[cfg(debug_assertions)]
    #[derive(Debug, Default, PartialEq)]
    pub(super) struct Overrides {
        pub endpoint: Option<tauri::Url>,
        pub pubkey: Option<String>,
    }

    /// `get` is injected so parsing is testable without mutating the process
    /// environment. Empty values count as unset.
    #[cfg(debug_assertions)]
    pub(super) fn parse(get: impl Fn(&str) -> Option<String>) -> Result<Overrides, String> {
        let endpoint = match get(ENDPOINT_VAR).filter(|v| !v.is_empty()) {
            Some(raw) => Some(
                raw.parse::<tauri::Url>()
                    .map_err(|e| format!("{ENDPOINT_VAR} is not a valid URL: {e}"))?,
            ),
            None => None,
        };
        let pubkey = get(PUBKEY_VAR).filter(|v| !v.is_empty());
        Ok(Overrides { endpoint, pubkey })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_lookup_is_unknown_not_an_error() {
        assert_eq!(
            status_from_lookup("0.18.1", Err("offline".into())),
            ReleaseStatus::Unknown
        );
        assert_eq!(
            status_from_lookup("0.18.1", Ok(None)),
            ReleaseStatus::Unknown
        );
    }

    #[test]
    fn a_successful_lookup_is_classified_against_the_running_version() {
        assert_eq!(
            status_from_lookup("0.18.2", Ok(Some("0.18.1".into()))),
            ReleaseStatus::Preview
        );
        assert_eq!(
            status_from_lookup("0.18.1", Ok(Some("0.18.1".into()))),
            ReleaseStatus::Published
        );
        assert_eq!(
            status_from_lookup("0.18.0", Ok(Some("0.18.1".into()))),
            ReleaseStatus::Published
        );
    }

    #[cfg(debug_assertions)]
    mod overrides {
        use super::super::debug_overrides::{parse, Overrides, ENDPOINT_VAR, PUBKEY_VAR};

        #[test]
        fn nothing_set_means_no_override() {
            assert_eq!(parse(|_| None).unwrap(), Overrides::default());
            assert_eq!(
                parse(|_| Some(String::new())).unwrap(),
                Overrides::default()
            );
        }

        #[test]
        fn invalid_endpoint_is_rejected() {
            assert!(parse(|n| (n == ENDPOINT_VAR).then(|| "not a url".into())).is_err());
        }

        #[test]
        fn http_loopback_endpoint_and_key_are_parsed() {
            let o = parse(|n| match n {
                ENDPOINT_VAR => Some("http://127.0.0.1:8123/latest.json".into()),
                PUBKEY_VAR => Some("key".into()),
                _ => None,
            })
            .unwrap();
            assert_eq!(
                o.endpoint.unwrap().as_str(),
                "http://127.0.0.1:8123/latest.json"
            );
            assert_eq!(o.pubkey.as_deref(), Some("key"));
        }
    }
}
