//! Where a video's thumbnails live under `thumbnails/`: the per-registered-folder
//! subdirectory name and the resolution of a file path to it.

use xxhash_rust::xxh64::xxh64;

use crate::paths::{normalize_folder_path, owning_folder};

/// Seed for `thumbnail_folder_subdir`'s `xxh64` hash. A fixed constant
/// (rather than `0`-by-convention like `hash::quick_hash`'s
/// `QUICK_HASH_SEED`) only to give this call site its own named constant --
/// there's no requirement that it match any other hash in the codebase,
/// since subdirectory names never need to compare equal to a `quick_hash`/
/// `full_hash` value.
const THUMBNAIL_SUBDIR_HASH_SEED: u64 = 0;

/// Derives the 16-hex-digit subdirectory name a registered folder's
/// thumbnails are grouped under (`thumbnails/<this>/`), so a folder can be
/// identified by a short, filesystem-safe, collision-resistant name even
/// though `settings.watch_folders` has no numeric/UUID id of its own -- the
/// path string *is* the folder's identity (see module-level context in the
/// call sites under `src-tauri/src/thumbnail/`).
///
/// Deterministic and pure: the same `folder_path` (up to the
/// case/trailing-separator differences `normalize_folder_path` already
/// treats as equivalent everywhere else in `paths.rs`, e.g.
/// `folder_paths_conflict`) always yields the same subdirectory name, with
/// no dependency on registration order or any other mutable state -- unlike
/// an incrementing id, this survives folders being added/removed/re-added
/// without ever needing a migration of its own.
pub fn thumbnail_folder_subdir(folder_path: &str) -> String {
    let normalized = normalize_folder_path(folder_path).to_lowercase();
    let hash = xxh64(normalized.as_bytes(), THUMBNAIL_SUBDIR_HASH_SEED);
    format!("{hash:016x}")
}

/// Fixed subdirectory name for thumbnails belonging to a video whose
/// `file_path` doesn't currently fall under any registered watch folder
/// (e.g. the folder was removed from `settings.watch_folders` after the
/// video was scanned, but the video row itself hasn't been cleaned up yet).
/// `thumbnail_folder_subdir` always returns a lowercase 16-hex-digit string,
/// which can never equal this name, so the two namespaces never collide.
pub const THUMBNAIL_UNASSIGNED_SUBDIR: &str = "_unassigned";

/// Resolves the thumbnail subdirectory `file_path` belongs to, given the
/// currently-registered `watch_folders`: the subdirectory of whichever
/// registered folder `file_path` falls under, or
/// `THUMBNAIL_UNASSIGNED_SUBDIR` if none does.
///
/// The folder is chosen by `owning_folder`, the same rule that decides which
/// videos a newly added folder takes over. If the two disagreed, moving
/// thumbnails on folder registration would skip videos that the placement
/// rule later files elsewhere. `owning_folder` also resolves transient nested
/// registrations to the most specific folder.
pub fn resolve_thumbnail_subdir(watch_folders: &[String], file_path: &str) -> String {
    match owning_folder(file_path, watch_folders) {
        Some(folder) => thumbnail_folder_subdir(folder),
        None => THUMBNAIL_UNASSIGNED_SUBDIR.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- thumbnail_folder_subdir -----------------------------------------

    #[test]
    fn thumbnail_folder_subdir_returns_16_lowercase_hex_digits() {
        let subdir = thumbnail_folder_subdir(r"C:\Videos");
        assert_eq!(subdir.len(), 16);
        assert!(subdir
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn thumbnail_folder_subdir_is_the_same_regardless_of_casing() {
        assert_eq!(
            thumbnail_folder_subdir(r"C:\Videos"),
            thumbnail_folder_subdir(r"c:\videos")
        );
    }

    #[test]
    fn thumbnail_folder_subdir_is_the_same_regardless_of_a_trailing_separator() {
        assert_eq!(
            thumbnail_folder_subdir(r"C:\Videos"),
            thumbnail_folder_subdir(r"C:\Videos\")
        );
    }

    #[test]
    fn thumbnail_folder_subdir_differs_for_different_folders() {
        assert_ne!(
            thumbnail_folder_subdir(r"C:\Videos"),
            thumbnail_folder_subdir(r"D:\Movies")
        );
    }

    // --- resolve_thumbnail_subdir -------------------------------------------

    #[test]
    fn resolve_thumbnail_subdir_returns_unassigned_when_no_folder_matches() {
        let folders = vec![r"C:\Videos".to_string()];
        assert_eq!(
            resolve_thumbnail_subdir(&folders, r"D:\Other\a.mp4"),
            THUMBNAIL_UNASSIGNED_SUBDIR
        );
    }

    #[test]
    fn resolve_thumbnail_subdir_returns_unassigned_for_an_empty_watch_folder_list() {
        assert_eq!(
            resolve_thumbnail_subdir(&[], r"C:\Videos\a.mp4"),
            THUMBNAIL_UNASSIGNED_SUBDIR
        );
    }

    #[test]
    fn resolve_thumbnail_subdir_returns_the_matching_folders_hash() {
        let folders = vec![r"C:\Videos".to_string()];
        assert_eq!(
            resolve_thumbnail_subdir(&folders, r"C:\Videos\a.mp4"),
            thumbnail_folder_subdir(r"C:\Videos")
        );
    }

    #[test]
    fn resolve_thumbnail_subdir_does_not_assign_the_folder_itself() {
        // A file cannot be the folder, and `owning_folder` only counts
        // strict descendants, so the same rule must not file it anywhere.
        let folders = vec![r"C:\Videos".to_string()];
        assert_eq!(
            resolve_thumbnail_subdir(&folders, r"C:\Videos"),
            THUMBNAIL_UNASSIGNED_SUBDIR
        );
    }

    #[test]
    fn resolve_thumbnail_subdir_folds_forward_slashes_like_owning_folder() {
        // The path comparison treats `/` and `\` as the same separator, so a
        // `/`-separated path under a registered folder is that folder's. The
        // folder-name hash is unchanged, so existing thumbnails stay valid.
        let folders = vec![r"C:\Videos".to_string()];
        assert_eq!(
            resolve_thumbnail_subdir(&folders, "C:/Videos/a.mp4"),
            thumbnail_folder_subdir(r"C:\Videos")
        );
    }

    #[test]
    fn resolve_thumbnail_subdir_is_case_insensitive() {
        let folders = vec![r"C:\Videos".to_string()];
        assert_eq!(
            resolve_thumbnail_subdir(&folders, r"c:\VIDEOS\a.mp4"),
            thumbnail_folder_subdir(r"C:\Videos")
        );
    }

    #[test]
    fn resolve_thumbnail_subdir_agrees_with_owning_folder_for_nested_slash_folders() {
        let folders = vec!["D:/v/".to_string(), "D:/v/sub".to_string()];
        for path in [
            r"D:\v\a.mp4",
            r"D:\v\sub\a.mp4",
            "D:/v/sub/a.mp4",
            r"D:\v2\a.mp4",
        ] {
            let expected = owning_folder(path, &folders)
                .map(thumbnail_folder_subdir)
                .unwrap_or_else(|| THUMBNAIL_UNASSIGNED_SUBDIR.to_string());
            assert_eq!(resolve_thumbnail_subdir(&folders, path), expected, "{path}");
        }
        assert_eq!(
            resolve_thumbnail_subdir(&folders, r"D:\v\sub\a.mp4"),
            thumbnail_folder_subdir("D:/v/sub")
        );
        assert_eq!(
            resolve_thumbnail_subdir(&folders, r"D:\v\a.mp4"),
            thumbnail_folder_subdir("D:/v/")
        );
    }

    #[test]
    fn resolve_thumbnail_subdir_picks_the_most_specific_match_among_nested_entries() {
        // Ordinarily `folder_paths_conflict` prevents both a parent and a
        // child folder from being registered at the same time, but this
        // exercises the defensive "most specific wins" tie-break for the
        // rare transient case where it happens anyway (e.g. mid-rename).
        let folders = vec![r"C:\Videos".to_string(), r"C:\Videos\Sub".to_string()];
        assert_eq!(
            resolve_thumbnail_subdir(&folders, r"C:\Videos\Sub\a.mp4"),
            thumbnail_folder_subdir(r"C:\Videos\Sub")
        );
    }
}
