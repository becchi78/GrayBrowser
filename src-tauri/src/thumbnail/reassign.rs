//! Moves already-generated thumbnails to where a changed watch-folder list
//! now says they belong.
//!
//! A video's thumbnail directory is derived from the watch folders at read
//! time (`paths::video_thumbnail_dir`), so registering a folder silently
//! changes where every video under it is looked up (from `_unassigned/` to
//! that folder's own subdirectory). Nothing moves the files by itself, which
//! leaves existing thumbnails unreachable: online videos fall back to
//! "generating" until the next scan, and offline videos -- whose legacy
//! thumbnail cannot be regenerated -- lose their image for good.

use std::path::Path;

use rusqlite::Connection;

use crate::db::queries;
use crate::thumbnail::paths::{move_thumbnail_files, slot_path, video_thumbnail_dir};
use crate::thumbnail::worker::THUMBNAILS_PER_VIDEO;

fn has_any_slot(dir: &Path, video_id: &str) -> bool {
    (0..THUMBNAILS_PER_VIDEO).any(|slot| slot_path(dir, video_id, slot).exists())
}

/// Moves the thumbnails of every video (any status) lying under a folder that
/// is in `new_folders` but not in `old_folders` from the directory they
/// resolved to under `old_folders` to the one they resolve to under
/// `new_folders`. Returns how many videos had files to move.
///
/// Best-effort: a failed move is logged by `move_thumbnail_files` and leaves
/// the files where they were, since thumbnails are regenerable and the watch
/// folder list is already saved by the time this runs. A video whose new
/// directory already holds any of its slots is left alone, so a thumbnail
/// generated after the folder was added is never overwritten by an older one.
pub fn reassign_thumbnails_after_folders_added(
    conn: &Connection,
    thumbnails_root: &Path,
    old_folders: &[String],
    new_folders: &[String],
) -> u32 {
    let added: Vec<String> = new_folders
        .iter()
        .filter(|f| !old_folders.contains(f))
        .cloned()
        .collect();
    if added.is_empty() {
        return 0;
    }

    let videos = match queries::list_video_ids_and_paths(conn) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("failed to list videos while reassigning thumbnails: {e}");
            return 0;
        }
    };

    let mut moved = 0;
    for (id, file_path) in videos {
        if !gb_core::paths::is_under_any_folder(&file_path, &added) {
            continue;
        }
        let old_dir = video_thumbnail_dir(thumbnails_root, old_folders, &file_path);
        let new_dir = video_thumbnail_dir(thumbnails_root, new_folders, &file_path);
        if old_dir == new_dir || !has_any_slot(&old_dir, &id) || has_any_slot(&new_dir, &id) {
            continue;
        }
        move_thumbnail_files(&id, &old_dir, &new_dir);
        moved += 1;
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{init_temp_db, insert_test_video};

    fn write_slots(dir: &Path, id: &str, slots: &[usize], content: &str) {
        std::fs::create_dir_all(dir).unwrap();
        for &slot in slots {
            std::fs::write(slot_path(dir, id, slot), content).unwrap();
        }
    }

    fn all_slots() -> Vec<usize> {
        (0..THUMBNAILS_PER_VIDEO).collect()
    }

    fn all_present(dir: &Path, id: &str) -> bool {
        (0..THUMBNAILS_PER_VIDEO).all(|s| slot_path(dir, id, s).exists())
    }

    fn folders(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn unassigned(root: &Path) -> std::path::PathBuf {
        root.join(gb_core::paths::THUMBNAIL_UNASSIGNED_SUBDIR)
    }

    fn run(db: &crate::db::Db, root: &Path, old: &[String], new: &[String]) -> u32 {
        let conn = db.writer.lock().unwrap();
        reassign_thumbnails_after_folders_added(&conn, root, old, new)
    }

    #[test]
    fn moves_all_six_slots_from_unassigned_into_the_new_folder_dir() {
        let (_d, db) = init_temp_db();
        let root = tempfile::tempdir().unwrap();
        insert_test_video(&db, "v1", r"D:\Vids\a.mp4");
        write_slots(&unassigned(root.path()), "v1", &all_slots(), "img");

        let new = folders(&[r"D:\Vids"]);
        assert_eq!(run(&db, root.path(), &[], &new), 1);

        let new_dir = video_thumbnail_dir(root.path(), &new, r"D:\Vids\a.mp4");
        assert!(all_present(&new_dir, "v1"));
        assert!(!has_any_slot(&unassigned(root.path()), "v1"));
    }

    #[test]
    fn offline_videos_keep_their_thumbnails() {
        let (_d, db) = init_temp_db();
        let root = tempfile::tempdir().unwrap();
        insert_test_video(&db, "off", r"D:\Vids\off.mp4");
        db.writer
            .lock()
            .unwrap()
            .execute("UPDATE videos SET status = 'offline' WHERE id = 'off'", [])
            .unwrap();
        write_slots(&unassigned(root.path()), "off", &all_slots(), "legacy");

        let new = folders(&[r"D:\Vids"]);
        assert_eq!(run(&db, root.path(), &[], &new), 1);

        let new_dir = video_thumbnail_dir(root.path(), &new, r"D:\Vids\off.mp4");
        assert!(all_present(&new_dir, "off"));
    }

    #[test]
    fn a_missing_slot_is_skipped_and_the_rest_still_move() {
        let (_d, db) = init_temp_db();
        let root = tempfile::tempdir().unwrap();
        insert_test_video(&db, "v1", r"D:\Vids\a.mp4");
        write_slots(&unassigned(root.path()), "v1", &[0, 1, 2], "img");

        let new = folders(&[r"D:\Vids"]);
        run(&db, root.path(), &[], &new);

        let new_dir = video_thumbnail_dir(root.path(), &new, r"D:\Vids\a.mp4");
        for s in [0, 1, 2] {
            assert!(slot_path(&new_dir, "v1", s).exists());
        }
        assert!(!slot_path(&new_dir, "v1", 3).exists());
    }

    #[test]
    fn existing_files_at_the_destination_are_not_overwritten() {
        let (_d, db) = init_temp_db();
        let root = tempfile::tempdir().unwrap();
        insert_test_video(&db, "v1", r"D:\Vids\a.mp4");
        write_slots(&unassigned(root.path()), "v1", &all_slots(), "old");
        let new = folders(&[r"D:\Vids"]);
        let new_dir = video_thumbnail_dir(root.path(), &new, r"D:\Vids\a.mp4");
        write_slots(&new_dir, "v1", &all_slots(), "fresh");

        assert_eq!(run(&db, root.path(), &[], &new), 0);

        assert_eq!(
            std::fs::read_to_string(slot_path(&new_dir, "v1", 0)).unwrap(),
            "fresh"
        );
        assert!(all_present(&unassigned(root.path()), "v1"));
    }

    #[test]
    fn videos_outside_the_added_folders_do_not_move() {
        let (_d, db) = init_temp_db();
        let root = tempfile::tempdir().unwrap();
        insert_test_video(&db, "in", r"D:\Vids\a.mp4");
        insert_test_video(&db, "out", r"E:\Other\b.mp4");
        write_slots(&unassigned(root.path()), "in", &all_slots(), "img");
        write_slots(&unassigned(root.path()), "out", &all_slots(), "img");

        assert_eq!(run(&db, root.path(), &[], &folders(&[r"D:\Vids"])), 1);

        assert!(all_present(&unassigned(root.path()), "out"));
        assert!(!has_any_slot(&unassigned(root.path()), "in"));
    }

    #[test]
    fn videos_under_an_already_registered_folder_do_not_move() {
        let (_d, db) = init_temp_db();
        let root = tempfile::tempdir().unwrap();
        insert_test_video(&db, "v1", r"D:\Vids\a.mp4");
        let old = folders(&[r"D:\Vids"]);
        let dir = video_thumbnail_dir(root.path(), &old, r"D:\Vids\a.mp4");
        write_slots(&dir, "v1", &all_slots(), "img");

        let new = folders(&[r"D:\Vids", r"E:\Other"]);
        assert_eq!(run(&db, root.path(), &old, &new), 0);
        assert!(all_present(&dir, "v1"));
    }

    #[test]
    fn no_added_folder_is_a_no_op() {
        let (_d, db) = init_temp_db();
        let root = tempfile::tempdir().unwrap();
        insert_test_video(&db, "v1", r"D:\Vids\a.mp4");
        write_slots(&unassigned(root.path()), "v1", &all_slots(), "img");
        let same = folders(&[r"D:\Vids"]);
        assert_eq!(run(&db, root.path(), &same, &same), 0);
        assert!(all_present(&unassigned(root.path()), "v1"));
    }
}
