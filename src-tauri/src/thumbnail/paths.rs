//! Common helpers for building/moving/removing the per-video thumbnail
//! files (`thumbnails/<folder-subdir>/<video_id>_<slot>.webp`), so the
//! filename/directory-layout convention lives in exactly one place instead
//! of being re-derived at each call site (`scan/`, `watch/`, and several
//! `commands/` modules).
//!
//! Since #6, thumbnails are grouped into a subdirectory per registered
//! watch folder (`gb_core::paths::resolve_thumbnail_subdir`) rather than
//! sitting flat directly under `thumbnails/` -- this module is the only
//! place that combines that OS-independent subdirectory-name logic with an
//! actual filesystem path.

use std::path::{Path, PathBuf};

use crate::thumbnail::worker::THUMBNAILS_PER_VIDEO;

pub fn thumbnails_root(app_dir: &Path) -> PathBuf {
    app_dir.join("thumbnails")
}

/// The directory a video's 6 thumbnails are (or should be) stored in, given
/// the currently-registered `watch_folders` and the video's `file_path`.
///
/// Pure path arithmetic -- never touches the filesystem. A caller that's
/// about to *write* thumbnails into the returned directory is responsible
/// for `create_dir_all`-ing it first (as `generate_thumbnail_for_video`
/// already does).
pub fn video_thumbnail_dir(
    thumbnails_root: &Path,
    watch_folders: &[String],
    file_path: &str,
) -> PathBuf {
    let subdir = gb_core::paths::resolve_thumbnail_subdir(watch_folders, file_path);
    thumbnails_root.join(subdir)
}

pub fn slot_path(video_dir: &Path, video_id: &str, slot: usize) -> PathBuf {
    video_dir.join(format!("{video_id}_{slot}.webp"))
}

/// Moves whichever of `video_id`'s `THUMBNAILS_PER_VIDEO` slot files
/// actually exist under `old_dir` over to `new_dir`, one `rename` at a
/// time. A missing slot (e.g. generation for that video hasn't completed,
/// or previously failed) is silently skipped rather than treated as an
/// error -- this mirrors `generate_thumbnail_for_video`'s own all-or-
/// nothing-per-slot approach to thumbnail completeness elsewhere in this
/// module tree.
///
/// `new_dir` is only `create_dir_all`-ed if there's actually at least one
/// file to move into it, and only once per call (not once per slot).
/// A no-op (`true`) when `old_dir == new_dir`, since there's nothing to do
/// and creating/touching either directory would be pointless.
///
/// Each individual file's move failure is logged via `log::warn!` and does
/// not stop the remaining slots from being attempted -- the return value is
/// simply whether *every* existing slot was moved successfully.
fn move_thumbnail_files(video_id: &str, old_dir: &Path, new_dir: &Path) -> bool {
    if old_dir == new_dir {
        return true;
    }

    let existing_slots: Vec<usize> = (0..THUMBNAILS_PER_VIDEO)
        .filter(|&slot| slot_path(old_dir, video_id, slot).exists())
        .collect();
    if existing_slots.is_empty() {
        return true;
    }

    if let Err(e) = std::fs::create_dir_all(new_dir) {
        log::warn!(
            "failed to create thumbnail directory {} while moving thumbnails for video {video_id}: {e}",
            new_dir.display()
        );
        return false;
    }

    let mut all_moved = true;
    for slot in existing_slots {
        let old_path = slot_path(old_dir, video_id, slot);
        let new_path = slot_path(new_dir, video_id, slot);
        if let Err(e) = std::fs::rename(&old_path, &new_path) {
            log::warn!(
                "failed to move thumbnail {} -> {}: {e}",
                old_path.display(),
                new_path.display()
            );
            all_moved = false;
        }
    }
    all_moved
}

/// Moves `video_id`'s thumbnail files from wherever `old_file_path` (under
/// `watch_folders`) currently resolves to, to wherever `new_file_path`
/// resolves to -- used by `register_new_path`'s "reactivate a previously-
/// offline video whose file moved" path, where both `watch_folders` and the
/// video's `file_path` are looked up fresh from the DB before calling this.
pub fn move_video_thumbnails(
    thumbnails_root: &Path,
    watch_folders: &[String],
    video_id: &str,
    old_file_path: &str,
    new_file_path: &str,
) -> bool {
    let old_dir = video_thumbnail_dir(thumbnails_root, watch_folders, old_file_path);
    let new_dir = video_thumbnail_dir(thumbnails_root, watch_folders, new_file_path);
    move_thumbnail_files(video_id, &old_dir, &new_dir)
}

/// Moves `video_id`'s thumbnail files directly between the subdirectories
/// for `old_folder_path` and `new_folder_path`, identified by the folder
/// path strings themselves rather than by searching `watch_folders` -- used
/// by `rename_watch_folder`, where by the time this runs `old_folder_path`
/// has already been replaced by `new_folder_path` in `settings.
/// watch_folders`, so it can no longer be found there.
pub fn move_video_thumbnails_between_folders(
    thumbnails_root: &Path,
    video_id: &str,
    old_folder_path: &str,
    new_folder_path: &str,
) -> bool {
    let old_dir = thumbnails_root.join(gb_core::paths::thumbnail_folder_subdir(old_folder_path));
    let new_dir = thumbnails_root.join(gb_core::paths::thumbnail_folder_subdir(new_folder_path));
    move_thumbnail_files(video_id, &old_dir, &new_dir)
}

/// Removes the entire thumbnail subdirectory for `folder_path` -- used by
/// `remove_watch_folder`, since every video under the removed folder is
/// deleted from the DB along with it, so there's no per-video cleanup left
/// to do; the whole subdirectory can go at once.
///
/// A subdirectory that doesn't exist (e.g. no thumbnail was ever generated
/// for any video under this folder) counts as success, not failure.
pub fn remove_folder_thumbnail_dir(thumbnails_root: &Path, folder_path: &str) -> bool {
    let dir = thumbnails_root.join(gb_core::paths::thumbnail_folder_subdir(folder_path));
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(e) => {
            log::warn!(
                "failed to remove thumbnail directory {} for folder {folder_path}: {e}",
                dir.display()
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_slot(dir: &Path, video_id: &str, slot: usize, contents: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(slot_path(dir, video_id, slot), contents).unwrap();
    }

    fn slot_contents(dir: &Path, video_id: &str, slot: usize) -> String {
        std::fs::read_to_string(slot_path(dir, video_id, slot)).unwrap()
    }

    // --- video_thumbnail_dir ------------------------------------------------

    #[test]
    fn video_thumbnail_dir_resolves_under_the_matching_registered_folder() {
        let root = Path::new(r"C:\app\thumbnails");
        let folders = vec![r"C:\Videos".to_string()];
        let dir = video_thumbnail_dir(root, &folders, r"C:\Videos\a.mp4");
        assert_eq!(
            dir,
            root.join(gb_core::paths::thumbnail_folder_subdir(r"C:\Videos"))
        );
    }

    #[test]
    fn video_thumbnail_dir_falls_back_to_unassigned() {
        let root = Path::new(r"C:\app\thumbnails");
        let dir = video_thumbnail_dir(root, &[], r"C:\Videos\a.mp4");
        assert_eq!(dir, root.join(gb_core::paths::THUMBNAIL_UNASSIGNED_SUBDIR));
    }

    // --- move_video_thumbnails -----------------------------------------

    #[test]
    fn move_video_thumbnails_moves_all_six_slots_and_creates_the_destination() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("thumbnails");
        let old_folder = r"C:\Videos".to_string();
        let new_folder = r"D:\Movies".to_string();
        let old_dir = root.join(gb_core::paths::thumbnail_folder_subdir(&old_folder));
        for slot in 0..THUMBNAILS_PER_VIDEO {
            write_slot(&old_dir, "vid-1", slot, &format!("slot-{slot}"));
        }

        let moved = move_video_thumbnails(
            &root,
            &[old_folder.clone(), new_folder.clone()],
            "vid-1",
            r"C:\Videos\a.mp4",
            r"D:\Movies\a.mp4",
        );

        assert!(moved);
        let new_dir = root.join(gb_core::paths::thumbnail_folder_subdir(&new_folder));
        assert!(new_dir.is_dir());
        for slot in 0..THUMBNAILS_PER_VIDEO {
            assert!(!slot_path(&old_dir, "vid-1", slot).exists());
            assert_eq!(
                slot_contents(&new_dir, "vid-1", slot),
                format!("slot-{slot}")
            );
        }
    }

    #[test]
    fn move_video_thumbnails_ignores_missing_slots_and_still_moves_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("thumbnails");
        let old_folder = r"C:\Videos".to_string();
        let new_folder = r"D:\Movies".to_string();
        let old_dir = root.join(gb_core::paths::thumbnail_folder_subdir(&old_folder));
        // Only slots 0 and 2 exist -- e.g. generation never finished for
        // this video.
        write_slot(&old_dir, "vid-partial", 0, "slot-0");
        write_slot(&old_dir, "vid-partial", 2, "slot-2");

        let moved = move_video_thumbnails(
            &root,
            &[old_folder, new_folder.clone()],
            "vid-partial",
            r"C:\Videos\a.mp4",
            r"D:\Movies\a.mp4",
        );

        assert!(
            moved,
            "moving only the slots that actually exist should still count as fully moved"
        );
        let new_dir = root.join(gb_core::paths::thumbnail_folder_subdir(&new_folder));
        assert_eq!(slot_contents(&new_dir, "vid-partial", 0), "slot-0");
        assert_eq!(slot_contents(&new_dir, "vid-partial", 2), "slot-2");
        assert!(!slot_path(&new_dir, "vid-partial", 1).exists());
    }

    #[test]
    fn move_video_thumbnails_is_a_true_no_op_when_source_and_destination_resolve_the_same() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("thumbnails");
        let folder = r"C:\Videos".to_string();
        let dir = root.join(gb_core::paths::thumbnail_folder_subdir(&folder));
        write_slot(&dir, "vid-same", 0, "slot-0");

        let moved = move_video_thumbnails(
            &root,
            &[folder],
            "vid-same",
            r"C:\Videos\a.mp4",
            r"C:\Videos\a-renamed.mp4",
        );

        assert!(moved);
        assert_eq!(slot_contents(&dir, "vid-same", 0), "slot-0");
    }

    #[test]
    fn move_video_thumbnails_succeeds_as_a_no_op_when_nothing_exists_yet() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("thumbnails");

        let moved = move_video_thumbnails(
            &root,
            &[r"C:\Videos".to_string(), r"D:\Movies".to_string()],
            "vid-none",
            r"C:\Videos\a.mp4",
            r"D:\Movies\a.mp4",
        );

        assert!(moved);
        assert!(
            !root
                .join(gb_core::paths::thumbnail_folder_subdir(r"D:\Movies"))
                .exists(),
            "the destination directory must not be created when there was nothing to move into it"
        );
    }

    // --- move_video_thumbnails_between_folders ---------------------------

    #[test]
    fn move_video_thumbnails_between_folders_moves_all_six_slots() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("thumbnails");
        let old_dir = root.join(gb_core::paths::thumbnail_folder_subdir(r"C:\Old"));
        for slot in 0..THUMBNAILS_PER_VIDEO {
            write_slot(&old_dir, "vid-rn", slot, &format!("slot-{slot}"));
        }

        let moved =
            move_video_thumbnails_between_folders(&root, "vid-rn", r"C:\Old", r"C:\Renamed");

        assert!(moved);
        let new_dir = root.join(gb_core::paths::thumbnail_folder_subdir(r"C:\Renamed"));
        for slot in 0..THUMBNAILS_PER_VIDEO {
            assert!(!slot_path(&old_dir, "vid-rn", slot).exists());
            assert_eq!(
                slot_contents(&new_dir, "vid-rn", slot),
                format!("slot-{slot}")
            );
        }
    }

    // --- remove_folder_thumbnail_dir --------------------------------------

    #[test]
    fn remove_folder_thumbnail_dir_removes_an_existing_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("thumbnails");
        let dir = root.join(gb_core::paths::thumbnail_folder_subdir(r"C:\Videos"));
        write_slot(&dir, "vid-1", 0, "slot-0");
        write_slot(&dir, "vid-2", 0, "slot-0");
        assert!(dir.exists());

        let removed = remove_folder_thumbnail_dir(&root, r"C:\Videos");

        assert!(removed);
        assert!(!dir.exists());
    }

    #[test]
    fn remove_folder_thumbnail_dir_succeeds_when_the_directory_never_existed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("thumbnails");

        let removed = remove_folder_thumbnail_dir(&root, r"C:\NeverScanned");

        assert!(removed);
    }
}
