//! Decisions for installing a chosen (possibly older) release: what the move
//! does to the user's database, and which releases may be offered at all.
//! Everything here is pure; fetching the release list is the
//! `ports::release_source` port's job.

use std::cmp::Ordering;

use serde::Deserialize;

/// Why the chosen release may not work with the current database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaWarning {
    /// The release's schema is older than the database's. Migrations cannot be
    /// rolled back, so that release refuses to open this database.
    TooNew,
    /// The release's manifest has no schema version (it predates the field),
    /// so compatibility cannot be judged.
    Unknown,
}

impl SchemaWarning {
    /// Name the frontend receives; fixed strings so renaming a variant cannot
    /// silently change the contract.
    pub fn as_str(self) -> &'static str {
        match self {
            SchemaWarning::TooNew => "tooNew",
            SchemaWarning::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assessment {
    pub is_downgrade: bool,
    /// The database must be backed up before installing. A downgrade is the
    /// one move that can leave the database unreadable, and an upgrade already
    /// gets the automatic backup right before its migration.
    pub backup_required: bool,
    pub schema_warning: Option<SchemaWarning>,
}

/// A version that cannot be parsed counts as a downgrade: the backup it
/// triggers is cheap, while skipping it on a wrong guess is not.
pub fn assess_install(
    current_version: &str,
    target_version: &str,
    current_db_schema: i64,
    target_schema: Option<i64>,
) -> Assessment {
    let is_downgrade = match (
        semver::Version::parse(current_version),
        semver::Version::parse(target_version),
    ) {
        (Ok(current), Ok(target)) => target.cmp(&current) == Ordering::Less,
        _ => true,
    };
    let schema_warning = match target_schema {
        Some(target) if target < current_db_schema => Some(SchemaWarning::TooNew),
        Some(_) => None,
        None => Some(SchemaWarning::Unknown),
    };
    Assessment {
        is_downgrade,
        backup_required: is_downgrade || schema_warning == Some(SchemaWarning::TooNew),
        schema_warning,
    }
}

/// The schema version a release declares in its update manifest, under the
/// `graybrowser.schemaVersion` key. Anything but a positive integer there is
/// treated as absent rather than trusted.
pub fn schema_version_from_manifest(raw_json: &serde_json::Value) -> Option<i64> {
    raw_json
        .get("graybrowser")?
        .get("schemaVersion")?
        .as_i64()
        .filter(|v| *v >= 1)
}

/// A release that can be offered for installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallableRelease {
    pub version: String,
    pub published_at: Option<String>,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: Option<String>,
}

/// File the updater reads from a release; one without it cannot be installed
/// through the updater.
pub const MANIFEST_ASSET: &str = "latest.json";

/// Picks the installable releases out of the GitHub "list releases" response,
/// newest version first. Kept: published (not draft, not prerelease), with a
/// `latest.json` asset, tagged exactly `v{semver}`. The fixed-name preview
/// release (`dev`) and anything else fails the tag rule.
pub fn parse_installable_releases(json: &str) -> Result<Vec<InstallableRelease>, String> {
    let releases: Vec<ApiRelease> =
        serde_json::from_str(json).map_err(|e| format!("リリース一覧を解釈できません: {e}"))?;
    let mut found: Vec<(semver::Version, InstallableRelease)> = releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter(|r| {
            r.assets
                .iter()
                .any(|a| a.name.as_deref() == Some(MANIFEST_ASSET))
        })
        .filter_map(|r| {
            let tag = r.tag_name?;
            let version = semver::Version::parse(tag.strip_prefix('v')?).ok()?;
            // Build metadata is ignored by semver ordering and never appears
            // in a release tag; refusing it keeps the version in the download
            // URL exactly what the tag says.
            if !version.build.is_empty() {
                return None;
            }
            Some((
                version.clone(),
                InstallableRelease {
                    version: version.to_string(),
                    published_at: r.published_at,
                },
            ))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(found.into_iter().map(|(_, r)| r).collect())
}

/// User-facing text for a non-success response of the release list. 403 and
/// 429 are how GitHub reports an exhausted unauthenticated quota (as is
/// `x-ratelimit-remaining: 0` on any status), which is worth telling apart
/// from a generic failure because waiting fixes it.
pub fn describe_fetch_failure(status: u16, rate_limit_remaining: Option<&str>) -> String {
    let exhausted = matches!(status, 403 | 429) || rate_limit_remaining.map(str::trim) == Some("0");
    if exhausted {
        "GitHub の取得回数の上限に達しました。しばらくしてからやり直してください。".to_string()
    } else {
        format!("リリース一覧を取得できませんでした。(HTTP {status})")
    }
}

/// Update manifest URL of the release `version`, or `None` unless `version` is
/// a canonical semver (what `to_string` gives back: no `v`, no padding, no
/// build metadata). Going through a parsed `Version` means nothing free-form
/// from the caller can reach the URL.
pub fn manifest_url(version: &str) -> Option<String> {
    let parsed = semver::Version::parse(version).ok()?;
    if parsed.to_string() != version || !parsed.build.is_empty() {
        return None;
    }
    Some(format!(
        "https://github.com/becchi78/GrayBrowser/releases/download/v{parsed}/{MANIFEST_ASSET}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn upgrade_needs_no_backup() {
        let a = assess_install("0.18.2", "0.18.3", 13, Some(13));
        assert_eq!(
            a,
            Assessment {
                is_downgrade: false,
                backup_required: false,
                schema_warning: None
            }
        );
    }

    #[test]
    fn same_version_is_not_a_downgrade() {
        let a = assess_install("0.18.2", "0.18.2", 13, Some(13));
        assert!(!a.is_downgrade);
        assert!(!a.backup_required);
    }

    #[test]
    fn downgrade_requires_a_backup() {
        let a = assess_install("0.18.2", "0.18.1", 13, Some(13));
        assert!(a.is_downgrade);
        assert!(a.backup_required);
        assert_eq!(a.schema_warning, None);
    }

    #[test]
    fn older_target_schema_is_too_new_for_that_release() {
        let a = assess_install("0.18.2", "0.17.0", 13, Some(12));
        assert_eq!(a.schema_warning, Some(SchemaWarning::TooNew));
        assert!(a.is_downgrade);
        assert!(a.backup_required);
    }

    #[test]
    fn too_new_requires_a_backup_even_when_the_versions_look_like_an_upgrade() {
        let a = assess_install("0.18.2", "0.19.0", 13, Some(12));
        assert!(!a.is_downgrade);
        assert_eq!(a.schema_warning, Some(SchemaWarning::TooNew));
        assert!(a.backup_required);
    }

    #[test]
    fn missing_schema_field_is_unknown() {
        let a = assess_install("0.18.2", "0.17.0", 13, None);
        assert_eq!(a.schema_warning, Some(SchemaWarning::Unknown));
        assert!(a.backup_required);
        let up = assess_install("0.18.2", "0.19.0", 13, None);
        assert_eq!(up.schema_warning, Some(SchemaWarning::Unknown));
        assert!(!up.backup_required);
    }

    #[test]
    fn target_schema_at_or_above_the_database_is_fine() {
        assert_eq!(
            assess_install("1.0.0", "0.9.0", 13, Some(13)).schema_warning,
            None
        );
        assert_eq!(
            assess_install("1.0.0", "2.0.0", 13, Some(14)).schema_warning,
            None
        );
    }

    #[test]
    fn unparseable_versions_are_treated_as_a_downgrade() {
        assert!(assess_install("x", "0.1.0", 1, Some(1)).backup_required);
        assert!(assess_install("0.1.0", "", 1, Some(1)).backup_required);
    }

    #[test]
    fn prerelease_ordering_follows_semver() {
        assert!(assess_install("1.0.0", "1.0.0-rc.1", 1, Some(1)).is_downgrade);
        assert!(!assess_install("1.0.0-rc.1", "1.0.0", 1, Some(1)).is_downgrade);
    }

    #[test]
    fn manifest_schema_version_is_read_from_the_graybrowser_key() {
        assert_eq!(
            schema_version_from_manifest(&json!({"graybrowser": {"schemaVersion": 13}})),
            Some(13)
        );
    }

    #[test]
    fn manifest_without_a_usable_schema_version_gives_none() {
        for raw in [
            json!({}),
            json!({"graybrowser": {}}),
            json!({"graybrowser": {"schemaVersion": "13"}}),
            json!({"graybrowser": {"schemaVersion": 0}}),
            json!({"graybrowser": {"schemaVersion": -3}}),
            json!({"graybrowser": {"schemaVersion": 1.5}}),
            json!({"graybrowser": 13}),
            json!(null),
        ] {
            assert_eq!(schema_version_from_manifest(&raw), None, "{raw}");
        }
    }

    fn release(tag: &str, draft: bool, prerelease: bool, assets: &[&str]) -> serde_json::Value {
        json!({
            "tag_name": tag,
            "draft": draft,
            "prerelease": prerelease,
            "published_at": "2026-11-01T00:00:00Z",
            "assets": assets.iter().map(|n| json!({"name": n})).collect::<Vec<_>>(),
        })
    }

    #[test]
    fn only_published_releases_with_a_manifest_and_a_version_tag_are_listed() {
        let body = json!([
            release("v0.18.1", false, false, &["latest.json", "setup.exe"]),
            release("v0.18.2", false, false, &["latest.json"]),
            release("v0.18.3", true, false, &["latest.json"]),
            release("v0.19.0-rc.1", false, true, &["latest.json"]),
            release("v0.17.0", false, false, &["setup.exe"]),
            release("dev", false, false, &["latest.json"]),
            release("0.16.0", false, false, &["latest.json"]),
            release("v0.16", false, false, &["latest.json"]),
            release("v0.15.0+build", false, false, &["latest.json"]),
            release("v10.0.0", false, false, &["latest.json"]),
            json!({"tag_name": null, "assets": [{"name": "latest.json"}]}),
            json!({"draft": false}),
        ])
        .to_string();
        let versions: Vec<_> = parse_installable_releases(&body)
            .unwrap()
            .into_iter()
            .map(|r| r.version)
            .collect();
        assert_eq!(versions, ["10.0.0", "0.18.2", "0.18.1"]);
    }

    #[test]
    fn listed_releases_carry_the_publication_date() {
        let body = json!([release("v1.0.0", false, false, &["latest.json"])]).to_string();
        let list = parse_installable_releases(&body).unwrap();
        assert_eq!(
            list,
            [InstallableRelease {
                version: "1.0.0".into(),
                published_at: Some("2026-11-01T00:00:00Z".into())
            }]
        );
    }

    #[test]
    fn an_empty_list_is_fine_and_garbage_is_an_error() {
        assert_eq!(parse_installable_releases("[]").unwrap(), []);
        assert!(parse_installable_releases("not json").is_err());
        assert!(parse_installable_releases("{}").is_err());
    }

    #[test]
    fn rate_limit_responses_get_a_distinct_message() {
        for (status, remaining) in [
            (403, None),
            (429, None),
            (403, Some("0")),
            (200, Some("0")),
            (500, Some(" 0 ")),
        ] {
            let m = describe_fetch_failure(status, remaining);
            assert!(
                m.contains("上限に達しました"),
                "{status} {remaining:?}: {m}"
            );
        }
    }

    #[test]
    fn other_failures_keep_the_status_code() {
        assert_eq!(
            describe_fetch_failure(500, None),
            "リリース一覧を取得できませんでした。(HTTP 500)"
        );
        assert!(!describe_fetch_failure(404, Some("57")).contains("上限"));
    }

    #[test]
    fn manifest_url_follows_the_fixed_release_pattern() {
        assert_eq!(
            manifest_url("0.18.1").as_deref(),
            Some("https://github.com/becchi78/GrayBrowser/releases/download/v0.18.1/latest.json")
        );
    }

    #[test]
    fn manifest_url_refuses_anything_but_a_canonical_version() {
        for v in [
            "",
            "abc",
            "0.18",
            "v0.18.1",
            "0.18.01",
            " 0.18.1",
            "0.18.1+build",
            "0.18.1/../x",
            "..",
            "0.18.1\n",
        ] {
            assert_eq!(manifest_url(v), None, "{v:?}");
        }
    }
}
