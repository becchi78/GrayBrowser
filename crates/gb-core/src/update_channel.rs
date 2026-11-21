//! Choosing between the stable and the development update channel.
//!
//! With developer mode on, both channels are checked and only the newer
//! release is offered. Looking at the development channel alone would hide a
//! newer stable patch release (e.g. 0.17.1 while `dev` holds 0.18.0-dev.1).

/// Which channel's update to offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Dev,
}

/// Picks the channel whose version is newer by semver precedence, so
/// `0.18.0-dev.2 < 0.18.0`. `None` means neither channel has an update.
///
/// Equal versions pick `Stable`: it is the release that has gone through the
/// gated publish flow. An unparseable version is an error rather than a guess,
/// leaving the fallback to the caller.
pub fn pick_newer(
    stable: Option<&str>,
    dev: Option<&str>,
) -> Result<Option<Channel>, semver::Error> {
    match (stable, dev) {
        (None, None) => Ok(None),
        (Some(_), None) => Ok(Some(Channel::Stable)),
        (None, Some(_)) => Ok(Some(Channel::Dev)),
        (Some(s), Some(d)) => {
            let s = semver::Version::parse(s)?;
            let d = semver::Version::parse(d)?;
            Ok(Some(if d > s { Channel::Dev } else { Channel::Stable }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_newer_than_stable_picks_dev() {
        assert_eq!(
            pick_newer(Some("0.17.1"), Some("0.18.0-dev.1")).unwrap(),
            Some(Channel::Dev)
        );
    }

    #[test]
    fn stable_newer_than_dev_picks_stable() {
        assert_eq!(
            pick_newer(Some("0.18.0"), Some("0.18.0-dev.3")).unwrap(),
            Some(Channel::Stable)
        );
        assert_eq!(
            pick_newer(Some("0.17.1"), Some("0.17.0-dev.9")).unwrap(),
            Some(Channel::Stable)
        );
    }

    #[test]
    fn higher_dev_number_is_newer() {
        assert_eq!(
            pick_newer(Some("0.17.0"), Some("0.18.0-dev.10")).unwrap(),
            Some(Channel::Dev)
        );
        assert_eq!(
            pick_newer(Some("0.18.0-dev.2"), Some("0.18.0-dev.10")).unwrap(),
            Some(Channel::Dev)
        );
    }

    #[test]
    fn only_one_channel_picks_that_channel() {
        assert_eq!(
            pick_newer(Some("0.17.1"), None).unwrap(),
            Some(Channel::Stable)
        );
        assert_eq!(
            pick_newer(None, Some("0.18.0-dev.1")).unwrap(),
            Some(Channel::Dev)
        );
    }

    #[test]
    fn neither_channel_picks_nothing() {
        assert_eq!(pick_newer(None, None).unwrap(), None);
    }

    #[test]
    fn identical_versions_pick_stable() {
        assert_eq!(
            pick_newer(Some("0.18.0"), Some("0.18.0")).unwrap(),
            Some(Channel::Stable)
        );
    }

    #[test]
    fn an_unparseable_version_is_an_error_only_when_both_must_be_compared() {
        assert!(pick_newer(Some("0.17.1"), Some("garbage")).is_err());
        assert!(pick_newer(Some("garbage"), Some("0.18.0-dev.1")).is_err());
        assert_eq!(
            pick_newer(Some("garbage"), None).unwrap(),
            Some(Channel::Stable)
        );
    }
}
