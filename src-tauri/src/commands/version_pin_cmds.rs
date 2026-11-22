//! Developer-mode commands for installing a chosen release instead of the
//! latest. The frontend passes only a version number: the release list comes
//! from a fixed GitHub URL, the manifest URL is built from a validated version,
//! and the install itself is the updater's signature-verified
//! `downloadAndInstall`, so no new download path exists.
//!
//! The decisions live in `*_impl` functions that take the connection and the
//! release-list port as arguments, so they run in tests without a Tauri
//! runtime or the network.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use gb_core::ports::release_source::ReleaseSource;
use gb_core::update_channel::Channel;
use gb_core::version_pin::{
    assess_install, manifest_url, parse_installable_releases, schema_version_from_manifest,
    InstallableRelease,
};
use rusqlite::Connection;
use serde::Serialize;
use tauri::{State, Webview};

use super::db_tools_cmds::{db_file, lock_within, manual_backup_step, WRITER_WAIT};
use super::developer_cmds::ensure_developer_mode;
use super::update_cmds::{check_channel, UpdateMetadata};
use crate::adapters::release_source::GithubReleaseSource;
use crate::db::{backup, Db};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseDto {
    pub version: String,
    pub published_at: Option<String>,
}

impl From<InstallableRelease> for ReleaseDto {
    fn from(r: InstallableRelease) -> Self {
        Self {
            version: r.version,
            published_at: r.published_at,
        }
    }
}

/// What the confirmation screen needs, next to the registered update.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedInstall {
    update: UpdateMetadata,
    is_downgrade: bool,
    backup_required: bool,
    /// "tooNew", "unknown", or none.
    schema_warning: Option<&'static str>,
    current_schema_version: i64,
    target_schema_version: Option<i64>,
}

/// Refused before any network access when developer mode is off.
fn list_releases_impl(
    conn: &Connection,
    source: &dyn ReleaseSource,
) -> Result<Vec<ReleaseDto>, String> {
    ensure_developer_mode(conn)?;
    let body = source.fetch_releases_json()?;
    Ok(parse_installable_releases(&body)?
        .into_iter()
        .map(Into::into)
        .collect())
}

/// Validates `version` and returns the manifest URL to check. The version must
/// be a canonical semver that is on the published list: a version typed or
/// forged by the caller never reaches the URL, and a release that has been
/// withdrawn since the list was shown is refused.
fn resolve_target_impl(
    conn: &Connection,
    source: &dyn ReleaseSource,
    version: &str,
) -> Result<String, String> {
    ensure_developer_mode(conn)?;
    let url = manifest_url(version)
        .ok_or_else(|| format!("バージョン \"{version}\" は X.Y.Z の形式ではありません。"))?;
    let listed = list_releases_impl(conn, source)?;
    if !listed.iter().any(|r| r.version == version) {
        return Err(format!(
            "バージョン {version} は、インストールできるリリースの一覧にありません。"
        ));
    }
    Ok(url)
}

/// Takes the backup that must precede installing an older release, and
/// returns its file name. The writer is held for the backup so it is exactly
/// the state that is about to be left; a failure is an `Err`, and the caller
/// must not go on to install.
fn backup_before_install_impl(
    writer: &Mutex<Connection>,
    wait: Duration,
    backup_step: impl FnOnce(&Connection) -> anyhow::Result<PathBuf>,
) -> Result<String, String> {
    let conn = lock_within(writer, wait)?;
    ensure_developer_mode(&conn)?;
    let path = backup_step(&conn).map_err(|e| {
        log::error!("backup before installing another version failed: {e:#}");
        format!("事前のバックアップに失敗したため、インストールに進みません。({e:#})")
    })?;
    Ok(path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default())
}

#[tauri::command]
pub async fn list_installable_releases(db: State<'_, Db>) -> Result<Vec<ReleaseDto>, String> {
    let pool = db.read_pool.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| e.to_string())?;
        list_releases_impl(&conn, &GithubReleaseSource)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Fetches the chosen release's manifest and registers its `Update` for the
/// frontend to download and install. Nothing is installed here.
#[tauri::command]
pub async fn prepare_version_install(
    webview: Webview,
    db: State<'_, Db>,
    version: String,
) -> Result<PreparedInstall, String> {
    let pool = db.read_pool.clone();
    let requested = version.clone();
    let (url, current_schema) = tauri::async_runtime::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| e.to_string())?;
        let url = resolve_target_impl(&conn, &GithubReleaseSource, &requested)?;
        let schema = backup::current_schema_version(&conn).map_err(|e| e.to_string())?;
        Ok::<_, String>((url, schema))
    })
    .await
    .map_err(|e| e.to_string())??;

    let endpoint = url.parse::<tauri::Url>().map_err(|e| e.to_string())?;
    let update = check_channel(&webview, Some(endpoint), true)
        .await?
        .ok_or_else(|| format!("バージョン {version} の配布情報を取得できませんでした。"))?;
    // The manifest at that URL is the release's own; a different version in it
    // means the release is mislabelled, and installing it would not be what
    // the user chose. The unregistered `Update` is dropped here.
    if update.version != version {
        return Err(format!(
            "配布情報のバージョン({})が、選んだバージョン {version} と一致しません。",
            update.version
        ));
    }

    let target_schema = schema_version_from_manifest(&update.raw_json);
    let assessment = assess_install(
        env!("CARGO_PKG_VERSION"),
        &update.version,
        current_schema,
        target_schema,
    );
    Ok(PreparedInstall {
        update: UpdateMetadata::register(&webview, update, Channel::Stable),
        is_downgrade: assessment.is_downgrade,
        backup_required: assessment.backup_required,
        schema_warning: assessment.schema_warning.map(|w| w.as_str()),
        current_schema_version: current_schema,
        target_schema_version: target_schema,
    })
}

/// Returns the backup file's name, or an error that must stop the install.
#[tauri::command]
pub async fn backup_before_install(db: State<'_, Db>) -> Result<String, String> {
    let writer = db.writer.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let file = db_file()?;
        backup_before_install_impl(&writer, WRITER_WAIT, manual_backup_step(&file))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::developer_cmds::DEVELOPER_MODE_REQUIRED;
    use crate::db::queries;
    use crate::db::test_support::init_temp_db;
    use gb_core::testing::fake_release_source::FakeReleaseSource;

    const BODY: &str = r#"[
        {"tag_name":"v0.18.2","draft":false,"prerelease":false,"published_at":"2026-11-20T00:00:00Z","assets":[{"name":"latest.json"}]},
        {"tag_name":"v0.18.1","draft":false,"prerelease":false,"published_at":"2026-11-10T00:00:00Z","assets":[{"name":"latest.json"}]},
        {"tag_name":"v0.18.3","draft":true,"prerelease":false,"assets":[{"name":"latest.json"}]}
    ]"#;

    fn dev_db(on: bool) -> (tempfile::TempDir, Db) {
        let (d, db) = init_temp_db();
        queries::set_developer_mode(&db.writer.lock().unwrap(), on).unwrap();
        (d, db)
    }

    fn resolve(db: &Db, source: &FakeReleaseSource, version: &str) -> Result<String, String> {
        resolve_target_impl(&db.read_pool.get().unwrap(), source, version)
    }

    #[test]
    fn listing_is_refused_without_developer_mode_and_before_any_fetch() {
        let (_d, db) = dev_db(false);
        // A source that would fail differently proves it was never asked.
        let source = FakeReleaseSource::failing("must not be called");
        let r = list_releases_impl(&db.read_pool.get().unwrap(), &source);
        assert_eq!(r.unwrap_err(), DEVELOPER_MODE_REQUIRED);
    }

    #[test]
    fn listing_returns_the_installable_releases_newest_first() {
        let (_d, db) = dev_db(true);
        let list = list_releases_impl(
            &db.read_pool.get().unwrap(),
            &FakeReleaseSource::with_body(BODY),
        )
        .unwrap();
        let versions: Vec<_> = list.iter().map(|r| r.version.as_str()).collect();
        assert_eq!(versions, ["0.18.2", "0.18.1"]);
        assert_eq!(
            list[0].published_at.as_deref(),
            Some("2026-11-20T00:00:00Z")
        );
    }

    #[test]
    fn a_failed_fetch_is_reported() {
        let (_d, db) = dev_db(true);
        let r = list_releases_impl(
            &db.read_pool.get().unwrap(),
            &FakeReleaseSource::failing("offline"),
        );
        assert_eq!(r.unwrap_err(), "offline");
    }

    #[test]
    fn a_listed_version_resolves_to_the_fixed_manifest_url() {
        let (_d, db) = dev_db(true);
        let url = resolve(&db, &FakeReleaseSource::with_body(BODY), "0.18.1").unwrap();
        assert_eq!(
            url,
            "https://github.com/becchi78/GrayBrowser/releases/download/v0.18.1/latest.json"
        );
    }

    #[test]
    fn a_version_that_is_not_listed_is_refused() {
        let (_d, db) = dev_db(true);
        let source = FakeReleaseSource::with_body(BODY);
        // 0.18.3 is only a draft; 0.1.0 does not exist.
        for version in ["0.18.3", "0.1.0"] {
            assert!(resolve(&db, &source, version).is_err(), "{version}");
        }
    }

    #[test]
    fn malformed_versions_are_refused_without_touching_the_network() {
        let (_d, db) = dev_db(true);
        let source = FakeReleaseSource::failing("must not be called");
        for version in [
            "",
            "abc",
            "0.18",
            "v0.18.1",
            "0.18.01",
            " 0.18.1",
            "0.18.1+build",
            "0.18.1/../../x",
            "..",
            "0.18.1\n",
        ] {
            let e = resolve(&db, &source, version).unwrap_err();
            assert_ne!(e, "must not be called", "{version:?}");
            assert!(e.contains("形式"), "{version:?}: {e}");
        }
    }

    #[test]
    fn resolving_is_refused_without_developer_mode() {
        let (_d, db) = dev_db(false);
        let r = resolve(&db, &FakeReleaseSource::with_body(BODY), "0.18.1");
        assert_eq!(r.unwrap_err(), DEVELOPER_MODE_REQUIRED);
    }

    #[test]
    fn backup_writes_a_verified_snapshot_and_returns_its_name() {
        let (d, db) = dev_db(true);
        let name = backup_before_install_impl(
            &db.writer,
            Duration::from_secs(1),
            manual_backup_step(&d.path().join("app.db")),
        )
        .unwrap();
        assert!(backup::backups_dir(d.path()).join(&name).is_file());
    }

    #[test]
    fn a_failed_backup_is_an_error_so_the_install_does_not_go_on() {
        let (_d, db) = dev_db(true);
        let r = backup_before_install_impl(&db.writer, Duration::from_secs(1), |_| {
            anyhow::bail!("disk full")
        });
        let e = r.unwrap_err();
        assert!(e.contains("進みません") && e.contains("disk full"), "{e}");
    }

    #[test]
    fn backup_is_refused_without_developer_mode_and_takes_none() {
        let (_d, db) = dev_db(false);
        let r = backup_before_install_impl(&db.writer, Duration::from_secs(1), |_| {
            panic!("no backup may be taken when the mode is off")
        });
        assert_eq!(r.unwrap_err(), DEVELOPER_MODE_REQUIRED);
    }
}
