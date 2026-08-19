//! Windows `\\?\` ("extended-length"/verbatim) path prefixing, so a video
//! living deeper than `MAX_PATH` (260 chars) can still be opened, walked,
//! stat'd, and passed to ffmpeg/ffprobe. This must
//! work without requiring the user to flip the system-wide
//! `LongPathsEnabled` registry setting, which is off by default on stock
//! Windows. **gb-core never sees this** -- `\\?\` is a Win32 API detail,
//! not something a "logical" file path should carry (the
//! quick_hash+file_size path-follow, and the NAS diff-scan's prefix
//! matching against `inaccessible_dirs`, both compare plain path strings;
//! see `strip_long_path_prefix` below).
//!
//! **Where this is applied:** immediately before the raw OS/process call
//! that would otherwise be subject to `MAX_PATH` (`File::open`,
//! `WalkDir::new`'s root, `std::fs::metadata`/`read_dir`,
//! `notify::Watcher::watch`'s folder argument, ffmpeg/ffprobe's CLI
//! arguments) -- never earlier. The DB's `file_path` column, `discovered_
//! paths`/`inaccessible_dirs` (`gb_core::reconcile`), and every
//! `gb_core::ports::watcher::WatchEvent::path` stay in plain (unprefixed)
//! form throughout; call `strip_long_path_prefix` right after receiving a
//! path back from an OS API that was itself given a prefixed root (a
//! `WalkDir` entry, a `notify` event), so a prefix never leaks into
//! anything that gets persisted or textually compared elsewhere.

use std::path::{Path, PathBuf};

/// Converts an absolute path to its `\\?\`-prefixed verbatim form.
/// Idempotent (an already-prefixed path is returned unchanged) and safe to
/// call unconditionally on every OS-call boundary regardless of whether the
/// path actually needs it -- ffmpeg/ffprobe and every `std::fs`/`WalkDir`
/// call used here accept `\\?\` paths just as well as short ones (confirmed
/// empirically).
///
/// Relative paths are returned unchanged: `\\?\` requires a fully-qualified
/// absolute path, and mechanically prepending it to a relative one would
/// produce a broken path rather than a working long one. This is a
/// defensive no-op, not an expected input -- every caller in this codebase
/// only ever passes paths that originated from a `WalkDir` root (the
/// user-configured, always-absolute watch folder), a DB `file_path` column,
/// or a `notify` event, all of which are absolute by construction.
pub fn to_long_path(path: &Path) -> PathBuf {
    let Some(s) = path.to_str() else {
        return path.to_path_buf(); // non-UTF-8; can't safely rewrite as a string, pass through
    };

    if s.starts_with(r"\\?\") {
        return path.to_path_buf();
    }

    if let Some(rest) = s.strip_prefix(r"\\") {
        // UNC: \\server\share\... -> \\?\UNC\server\share\... (not
        // \\?\\\server\share -- \\?\UNC\ is the documented special-cased
        // form for UNC verbatim paths).
        return PathBuf::from(format!(r"\\?\UNC\{rest}"));
    }

    let bytes = s.as_bytes();
    let is_drive_absolute = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    if is_drive_absolute {
        PathBuf::from(format!(r"\\?\{s}"))
    } else {
        path.to_path_buf()
    }
}

/// The inverse of `to_long_path`. Restores a `\\?\`- or `\\?\UNC\`-prefixed
/// path to its plain form; a path that isn't prefixed is returned
/// unchanged (idempotent, same as `to_long_path`). Used right after reading
/// a path back from an OS API that was given a prefixed root (a `WalkDir`
/// entry, a `notify` watch event) so the prefix never leaks into anything
/// stored in the DB or compared against a plain path elsewhere.
pub fn strip_long_path_prefix(path: &Path) -> PathBuf {
    let Some(s) = path.to_str() else {
        return path.to_path_buf();
    };

    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_long_path_prefixes_a_drive_letter_absolute_path() {
        let got = to_long_path(Path::new(r"C:\foo\bar.mp4"));
        assert_eq!(got, PathBuf::from(r"\\?\C:\foo\bar.mp4"));
    }

    #[test]
    fn to_long_path_prefixes_a_unc_path_with_the_unc_form() {
        let got = to_long_path(Path::new(r"\\nas\share\foo\bar.mp4"));
        // Must be \\?\UNC\..., not \\?\\\nas\share\... -- a naive
        // "just prepend \\?\" would double the leading backslashes instead
        // of producing the documented UNC verbatim form.
        assert_eq!(got, PathBuf::from(r"\\?\UNC\nas\share\foo\bar.mp4"));
        assert!(got.to_str().unwrap().starts_with(r"\\?\UNC\"));
    }

    #[test]
    fn to_long_path_is_idempotent_on_an_already_prefixed_drive_path() {
        let once = to_long_path(Path::new(r"C:\foo\bar.mp4"));
        let twice = to_long_path(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn to_long_path_is_idempotent_on_an_already_prefixed_unc_path() {
        let once = to_long_path(Path::new(r"\\nas\share\foo\bar.mp4"));
        let twice = to_long_path(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn to_long_path_does_not_mangle_a_relative_path() {
        let got = to_long_path(Path::new(r"foo\bar.mp4"));
        assert_eq!(got, PathBuf::from(r"foo\bar.mp4"));
        assert!(!got.to_str().unwrap().starts_with(r"\\?\"));
    }

    #[test]
    fn strip_long_path_prefix_restores_a_drive_letter_path() {
        let got = strip_long_path_prefix(Path::new(r"\\?\C:\foo\bar.mp4"));
        assert_eq!(got, PathBuf::from(r"C:\foo\bar.mp4"));
    }

    #[test]
    fn strip_long_path_prefix_restores_a_unc_path_to_its_double_backslash_form() {
        let got = strip_long_path_prefix(Path::new(r"\\?\UNC\nas\share\foo\bar.mp4"));
        assert_eq!(got, PathBuf::from(r"\\nas\share\foo\bar.mp4"));
    }

    #[test]
    fn strip_long_path_prefix_is_a_no_op_on_an_unprefixed_path() {
        let got = strip_long_path_prefix(Path::new(r"C:\foo\bar.mp4"));
        assert_eq!(got, PathBuf::from(r"C:\foo\bar.mp4"));
    }

    #[test]
    fn to_long_path_then_strip_round_trips_for_drive_and_unc_paths() {
        for raw in [r"C:\foo\bar.mp4", r"\\nas\share\foo\bar.mp4"] {
            let prefixed = to_long_path(Path::new(raw));
            let restored = strip_long_path_prefix(&prefixed);
            assert_eq!(restored, PathBuf::from(raw), "round-trip failed for {raw}");
        }
    }
}
