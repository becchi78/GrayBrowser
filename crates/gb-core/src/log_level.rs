//! Log verbosity chosen in developer mode. The stored strings are also the
//! values the frontend sends and shows, so parsing and formatting live in one
//! place.

/// `Info` is the level every user runs at. The more verbose levels exist only
/// for investigating a problem and are reachable only in developer mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogLevel {
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub const ALL: [LogLevel; 3] = [LogLevel::Info, LogLevel::Debug, LogLevel::Trace];

    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Info => "info",
            LogLevel::Debug => "debug",
            LogLevel::Trace => "trace",
        }
    }

    /// Exact lowercase match only: anything else (including a hand-edited or
    /// corrupt stored value) is rejected rather than guessed at.
    pub fn parse(s: &str) -> Option<LogLevel> {
        LogLevel::ALL.into_iter().find(|l| l.as_str() == s)
    }
}

/// The level actually in force. Outside developer mode it is always `Info`,
/// whatever was configured, so a leftover verbose setting can neither slow a
/// normal user's app nor fill their disk with detail they never asked for.
pub fn effective_log_level(developer_mode: bool, configured: LogLevel) -> LogLevel {
    if developer_mode {
        configured
    } else {
        LogLevel::Info
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_level_round_trips_through_its_string() {
        for level in LogLevel::ALL {
            assert_eq!(LogLevel::parse(level.as_str()), Some(level));
        }
    }

    #[test]
    fn unknown_empty_and_differently_cased_values_are_rejected() {
        for s in [
            "", "warn", "error", "off", "DEBUG", "Info", " info", "debug ",
        ] {
            assert_eq!(LogLevel::parse(s), None, "{s:?}");
        }
    }

    #[test]
    fn the_default_is_info() {
        assert_eq!(LogLevel::default(), LogLevel::Info);
    }

    #[test]
    fn developer_mode_off_forces_info() {
        for level in LogLevel::ALL {
            assert_eq!(effective_log_level(false, level), LogLevel::Info);
        }
    }

    #[test]
    fn developer_mode_on_uses_the_configured_level() {
        for level in LogLevel::ALL {
            assert_eq!(effective_log_level(true, level), level);
        }
    }
}
