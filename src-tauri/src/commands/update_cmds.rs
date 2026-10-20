//! In-app update commands: the startup-check preference and the update check
//! itself. Download and install go through the updater plugin's own
//! `download_and_install` command on the resource this module registers.

use serde::Serialize;
use tauri::{Manager, State, Webview};
use tauri_plugin_updater::UpdaterExt;

use crate::db::{queries, Db};

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
}

/// Checks for a newer release. Own command instead of the plugin's JS
/// `check()` so that the frontend is never granted `updater:allow-check`,
/// whose `target`/`headers`/`proxy` arguments it could otherwise choose freely;
/// the endpoint and public key stay fixed to the build-time configuration.
#[tauri::command]
pub async fn check_for_update(webview: Webview) -> Result<Option<UpdateMetadata>, String> {
    let builder = webview.updater_builder().timeout(CHECK_TIMEOUT);
    let builder = debug_overrides::apply(builder)?;
    let updater = builder.build().map_err(|e| e.to_string())?;
    let update = updater.check().await.map_err(|e| e.to_string())?;
    Ok(update.map(|update| UpdateMetadata {
        current_version: update.current_version.clone(),
        version: update.version.clone(),
        body: update.body.clone(),
        raw_json: update.raw_json.clone(),
        rid: webview.resources_table().add(update),
    }))
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
    fn startup_check_defaults_to_on_and_round_trips() {
        let (_d, db) = crate::db::test_support::init_temp_db();
        let conn = db.writer.lock().unwrap();
        assert!(queries::get_update_check_on_startup(&conn).unwrap());
        queries::set_update_check_on_startup(&conn, false).unwrap();
        assert!(!queries::get_update_check_on_startup(&conn).unwrap());
        queries::set_update_check_on_startup(&conn, true).unwrap();
        assert!(queries::get_update_check_on_startup(&conn).unwrap());
    }

    #[test]
    fn unrecognized_stored_value_reads_as_on() {
        let (_d, db) = crate::db::test_support::init_temp_db();
        let conn = db.writer.lock().unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('update_check_on_startup', 'garbage')",
            [],
        )
        .unwrap();
        assert!(queries::get_update_check_on_startup(&conn).unwrap());
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
