//! Whether the running build is already published as a stable release.
//!
//! The preview channel hands out the build that is still waiting as a draft,
//! so a running version newer than the latest published stable one can only
//! be such a pre-release build.

/// State of the running build relative to the latest published stable release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseStatus {
    /// Newer than the latest published stable release.
    Preview,
    /// Equal to or older than the latest published stable release.
    Published,
    /// The published version could not be obtained or compared.
    Unknown,
}

impl ReleaseStatus {
    /// Name the frontend receives. Fixed strings rather than a derived
    /// serialization so renaming a variant cannot silently change the contract.
    pub fn as_str(self) -> &'static str {
        match self {
            ReleaseStatus::Preview => "preview",
            ReleaseStatus::Published => "published",
            ReleaseStatus::Unknown => "unknown",
        }
    }
}

/// Compares by semver precedence. A missing or unparseable version on either
/// side is `Unknown` rather than a guess: claiming "published" for a build
/// that is not would hide that it is unreleased.
pub fn classify(running: &str, latest_stable: Option<&str>) -> ReleaseStatus {
    let Some(latest) = latest_stable else {
        return ReleaseStatus::Unknown;
    };
    match (
        semver::Version::parse(running),
        semver::Version::parse(latest),
    ) {
        (Ok(running), Ok(latest)) if running > latest => ReleaseStatus::Preview,
        (Ok(_), Ok(_)) => ReleaseStatus::Published,
        _ => ReleaseStatus::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_than_the_published_stable_is_preview() {
        assert_eq!(classify("0.18.2", Some("0.18.1")), ReleaseStatus::Preview);
        assert_eq!(classify("1.0.0", Some("0.99.99")), ReleaseStatus::Preview);
    }

    #[test]
    fn older_than_the_published_stable_is_published() {
        assert_eq!(classify("0.18.0", Some("0.18.1")), ReleaseStatus::Published);
    }

    #[test]
    fn the_same_version_is_published() {
        assert_eq!(classify("0.18.1", Some("0.18.1")), ReleaseStatus::Published);
    }

    #[test]
    fn no_published_version_is_unknown() {
        assert_eq!(classify("0.18.1", None), ReleaseStatus::Unknown);
    }

    #[test]
    fn an_unparseable_version_is_unknown() {
        assert_eq!(classify("0.18.1", Some("garbage")), ReleaseStatus::Unknown);
        assert_eq!(classify("garbage", Some("0.18.1")), ReleaseStatus::Unknown);
        assert_eq!(classify("", Some("")), ReleaseStatus::Unknown);
    }

    #[test]
    fn frontend_names_are_fixed() {
        assert_eq!(ReleaseStatus::Preview.as_str(), "preview");
        assert_eq!(ReleaseStatus::Published.as_str(), "published");
        assert_eq!(ReleaseStatus::Unknown.as_str(), "unknown");
    }
}
