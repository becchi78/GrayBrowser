//! Decides whether a recorded file is gone for good, without ever mistaking
//! "cannot reach it right now" for "it was deleted".

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Present,
    /// The location is reachable and its folder exists, but the file does not.
    Missing,
    /// Absence cannot be proven (offline share, vanished parent folder).
    Unknown,
}

/// `folder_unreachable`: the watch folder owning the file is currently down.
/// `parent_exists` / `file_exists`: results of stat-ing the parent folder and
/// the file itself; when `folder_unreachable` is set they were never checked
/// and are ignored.
///
/// A vanished parent folder is `Unknown` rather than `Missing`: an unmounted
/// drive or a renamed folder looks exactly like that, and a file hidden by
/// mistake could then not be found again from the list.
pub fn classify_presence(
    folder_unreachable: bool,
    parent_exists: bool,
    file_exists: bool,
) -> Presence {
    if folder_unreachable {
        return Presence::Unknown;
    }
    if file_exists {
        return Presence::Present;
    }
    if parent_exists {
        Presence::Missing
    } else {
        Presence::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_every_combination() {
        let cases = [
            // (unreachable, parent, file) -> expected
            (false, true, true, Presence::Present),
            (false, true, false, Presence::Missing),
            (false, false, true, Presence::Present),
            (false, false, false, Presence::Unknown),
            (true, true, true, Presence::Unknown),
            (true, true, false, Presence::Unknown),
            (true, false, true, Presence::Unknown),
            (true, false, false, Presence::Unknown),
        ];
        for (unreachable, parent, file, expected) in cases {
            assert_eq!(
                classify_presence(unreachable, parent, file),
                expected,
                "unreachable={unreachable} parent={parent} file={file}"
            );
        }
    }
}
