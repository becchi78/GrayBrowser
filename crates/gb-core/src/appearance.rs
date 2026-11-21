//! Theme and color-mode choices. The stored strings are also the values of the
//! `data-theme` / `data-mode` attributes the frontend sets, so parsing and
//! formatting live in one place and never drift from the menu or the settings.

/// Visual design of the whole window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Console,
    Fluent,
    Cinema,
}

impl Theme {
    /// Display order of the menu: the default first.
    pub const ALL: [Theme; 3] = [Theme::Console, Theme::Fluent, Theme::Cinema];

    pub fn as_str(self) -> &'static str {
        match self {
            Theme::Console => "console",
            Theme::Fluent => "fluent",
            Theme::Cinema => "cinema",
        }
    }

    /// Only exact known values are accepted; anything else is `None` so a
    /// caller decides between "fall back to default" (reading a stored value)
    /// and "reject" (a command argument).
    pub fn parse(s: &str) -> Option<Theme> {
        Theme::ALL.into_iter().find(|t| t.as_str() == s)
    }
}

/// Light/dark choice. `System` follows the OS setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    System,
    Light,
    Dark,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::System, Mode::Light, Mode::Dark];

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::System => "system",
            Mode::Light => "light",
            Mode::Dark => "dark",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.as_str() == s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_console_and_system() {
        assert_eq!(Theme::default(), Theme::Console);
        assert_eq!(Mode::default(), Mode::System);
    }

    #[test]
    fn themes_are_listed_default_first() {
        assert_eq!(
            Theme::ALL.map(Theme::as_str),
            ["console", "fluent", "cinema"]
        );
        assert_eq!(Theme::ALL[0], Theme::default());
    }

    #[test]
    fn every_value_round_trips_through_its_string() {
        for t in Theme::ALL {
            assert_eq!(Theme::parse(t.as_str()), Some(t));
        }
        for m in Mode::ALL {
            assert_eq!(Mode::parse(m.as_str()), Some(m));
        }
    }

    #[test]
    fn unknown_empty_and_differently_cased_values_are_rejected() {
        for bad in ["", "Fluent", "default", " console", "console "] {
            assert_eq!(Theme::parse(bad), None, "{bad:?}");
        }
        for bad in ["", "System", "auto", "dark "] {
            assert_eq!(Mode::parse(bad), None, "{bad:?}");
        }
    }
}
