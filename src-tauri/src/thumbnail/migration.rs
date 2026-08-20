//! One-time startup migration from the pre-#6 flat thumbnail layout
//! (`thumbnails/<video_id>_<slot>.webp`) to per-registered-folder
//! subdirectories (`thumbnails/<folder-hash>/<video_id>_<slot>.webp`, or
//! `thumbnails/_unassigned/...` for a video whose file no longer falls
//! under any registered folder).
//!
//! Same safety posture as `paths::migrate_legacy_nested_app_dir`: a
//! thumbnail is regenerable, so every per-video failure here is logged and
//! skipped rather than aborting startup. Intended to run once, after
//! `db::init` and before `enqueue_missing_thumbnails` -- but it's also
//! idempotent on its own: once every leftover flat file has been moved, the
//! top level of `thumbnails/` contains only subdirectories, so a single
//! `read_dir` pass finds nothing left to migrate and returns immediately.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::db::{queries, Db};
use crate::thumbnail::paths::video_thumbnail_dir;
use crate::thumbnail::worker::THUMBNAILS_PER_VIDEO;

#[derive(Default, Debug)]
pub struct MigrationSummary {
    pub videos_migrated: u32,
    pub videos_failed: u32,
}

pub fn migrate_flat_thumbnails_to_folder_subdirs(
    db: &Db,
    thumbnails_root: &Path,
) -> anyhow::Result<MigrationSummary> {
    let mut summary = MigrationSummary::default();

    // Only the top level is inspected -- an already-migrated `thumbnails/`
    // contains nothing but per-folder subdirectories at this level, so this
    // read_dir alone is enough to detect "nothing left to do" without ever
    // recursing into them.
    let entries = match std::fs::read_dir(thumbnails_root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(summary),
        Err(e) => return Err(e.into()),
    };

    let mut by_video_id: BTreeMap<String, Vec<(usize, PathBuf)>> = BTreeMap::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                log::warn!("failed to read a thumbnails directory entry during migration: {e}");
                continue;
            }
        };
        // Directories are the already-migrated per-folder layout and are
        // never recursed into; anything whose file type can't be
        // determined is skipped defensively rather than guessed at.
        let is_file = entry.file_type().map(|ft| ft.is_file()).unwrap_or(false);
        if !is_file {
            continue;
        }
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if let Some((video_id, slot)) = parse_flat_thumbnail_file_name(file_name) {
            by_video_id
                .entry(video_id)
                .or_default()
                .push((slot, entry.path()));
        }
    }

    if by_video_id.is_empty() {
        return Ok(summary);
    }

    let watch_folders = {
        let conn = db.read_pool.get()?;
        queries::get_watch_folders(&conn)?
    };

    for (video_id, slot_files) in by_video_id {
        let video = match queries::find_video_by_id(&db.read_pool, &video_id) {
            Ok(Some(video)) => video,
            Ok(None) => {
                log::warn!(
                    "thumbnail migration: no DB row for video_id {video_id}; leaving its \
                     leftover thumbnail file(s) in place"
                );
                summary.videos_failed += 1;
                continue;
            }
            Err(e) => {
                log::warn!("thumbnail migration: failed to look up video {video_id}: {e}");
                summary.videos_failed += 1;
                continue;
            }
        };

        let new_dir = video_thumbnail_dir(thumbnails_root, &watch_folders, &video.file_path);
        if let Err(e) = std::fs::create_dir_all(&new_dir) {
            log::warn!(
                "thumbnail migration: failed to create destination directory {} for video \
                 {video_id}: {e}",
                new_dir.display()
            );
            summary.videos_failed += 1;
            continue;
        }

        let mut all_moved = true;
        for (slot, old_path) in slot_files {
            let new_path = new_dir.join(format!("{video_id}_{slot}.webp"));
            if let Err(e) = std::fs::rename(&old_path, &new_path) {
                log::warn!(
                    "thumbnail migration: failed to move {} -> {}: {e}",
                    old_path.display(),
                    new_path.display()
                );
                all_moved = false;
            }
        }

        if all_moved {
            summary.videos_migrated += 1;
        } else {
            summary.videos_failed += 1;
        }
    }

    Ok(summary)
}

/// Parses a flat-layout thumbnail file name
/// (`"<video_id>_<slot>.webp"`, `slot` in `0..THUMBNAILS_PER_VIDEO`) back
/// into its `(video_id, slot)` parts. Returns `None` for anything else
/// found sitting directly under `thumbnails/` (a `.webp.tmp` leftover from
/// an interrupted generation, an out-of-range slot number, or an unrelated
/// file) -- `video_id` is always a UUID v4 and therefore never contains an
/// underscore itself, so splitting on the *last* `_` unambiguously
/// separates it from the slot suffix.
fn parse_flat_thumbnail_file_name(file_name: &str) -> Option<(String, usize)> {
    let stem = file_name.strip_suffix(".webp")?;
    let (video_id, slot_str) = stem.rsplit_once('_')?;
    if video_id.is_empty() {
        return None;
    }
    let slot: usize = slot_str.parse().ok()?;
    if slot >= THUMBNAILS_PER_VIDEO {
        return None;
    }
    Some((video_id.to_string(), slot))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::queries as db_queries;
    use crate::db::test_support::{init_temp_db, insert_test_video};

    fn write_flat_thumbnail(thumbs_root: &Path, video_id: &str, slot: usize) {
        std::fs::create_dir_all(thumbs_root).unwrap();
        std::fs::write(
            thumbs_root.join(format!("{video_id}_{slot}.webp")),
            format!("slot-{slot}"),
        )
        .unwrap();
    }

    // --- parse_flat_thumbnail_file_name -----------------------------------

    #[test]
    fn parses_a_well_formed_flat_thumbnail_file_name() {
        assert_eq!(
            parse_flat_thumbnail_file_name("abc-123_0.webp"),
            Some(("abc-123".to_string(), 0))
        );
        assert_eq!(
            parse_flat_thumbnail_file_name("abc-123_5.webp"),
            Some(("abc-123".to_string(), 5))
        );
    }

    #[test]
    fn ignores_a_tmp_leftover_file() {
        assert_eq!(parse_flat_thumbnail_file_name("abc-123_0.webp.tmp"), None);
    }

    #[test]
    fn ignores_an_out_of_range_slot_number() {
        assert_eq!(parse_flat_thumbnail_file_name("abc-123_6.webp"), None);
    }

    #[test]
    fn ignores_an_unrelated_file_name() {
        assert_eq!(parse_flat_thumbnail_file_name("readme.txt"), None);
        assert_eq!(parse_flat_thumbnail_file_name("abc-123.webp"), None);
        assert_eq!(parse_flat_thumbnail_file_name("_0.webp"), None);
    }

    // --- migrate_flat_thumbnails_to_folder_subdirs ------------------------

    #[test]
    fn moves_a_fully_generated_videos_six_thumbnails_into_its_folder_subdir() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = tempfile::tempdir().unwrap();
        insert_test_video(&db, "vid-1", r"C:\Videos\a.mp4");
        {
            let conn = db.writer.lock().unwrap();
            db_queries::set_watch_folders(&conn, &[r"C:\Videos".to_string()]).unwrap();
        }
        for slot in 0..THUMBNAILS_PER_VIDEO {
            write_flat_thumbnail(thumbs_root.path(), "vid-1", slot);
        }

        let summary = migrate_flat_thumbnails_to_folder_subdirs(&db, thumbs_root.path()).unwrap();

        assert_eq!(summary.videos_migrated, 1);
        assert_eq!(summary.videos_failed, 0);
        let expected_dir = thumbs_root
            .path()
            .join(gb_core::paths::thumbnail_folder_subdir(r"C:\Videos"));
        for slot in 0..THUMBNAILS_PER_VIDEO {
            assert!(!thumbs_root
                .path()
                .join(format!("vid-1_{slot}.webp"))
                .exists());
            assert_eq!(
                std::fs::read_to_string(expected_dir.join(format!("vid-1_{slot}.webp"))).unwrap(),
                format!("slot-{slot}")
            );
        }
    }

    #[test]
    fn moves_a_partially_generated_videos_thumbnails_best_effort() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = tempfile::tempdir().unwrap();
        insert_test_video(&db, "vid-partial", r"C:\Videos\a.mp4");
        {
            let conn = db.writer.lock().unwrap();
            db_queries::set_watch_folders(&conn, &[r"C:\Videos".to_string()]).unwrap();
        }
        write_flat_thumbnail(thumbs_root.path(), "vid-partial", 0);
        write_flat_thumbnail(thumbs_root.path(), "vid-partial", 2);

        let summary = migrate_flat_thumbnails_to_folder_subdirs(&db, thumbs_root.path()).unwrap();

        assert_eq!(summary.videos_migrated, 1);
        assert_eq!(summary.videos_failed, 0);
        let expected_dir = thumbs_root
            .path()
            .join(gb_core::paths::thumbnail_folder_subdir(r"C:\Videos"));
        assert!(expected_dir.join("vid-partial_0.webp").exists());
        assert!(expected_dir.join("vid-partial_2.webp").exists());
        assert!(!expected_dir.join("vid-partial_1.webp").exists());
    }

    #[test]
    fn skips_leftover_files_for_a_video_id_missing_from_the_db_without_erroring() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = tempfile::tempdir().unwrap();
        // No insert_test_video call: this video_id was already deleted from
        // the DB, but its thumbnail files were never cleaned up.
        write_flat_thumbnail(thumbs_root.path(), "orphan-vid", 0);

        let summary = migrate_flat_thumbnails_to_folder_subdirs(&db, thumbs_root.path()).unwrap();

        assert_eq!(summary.videos_migrated, 0);
        assert_eq!(summary.videos_failed, 1);
        // Left in place, not deleted -- migration is not responsible for
        // cleaning up orphaned thumbnail files, only for relocating known
        // ones.
        assert!(thumbs_root.path().join("orphan-vid_0.webp").exists());
    }

    #[test]
    fn a_video_with_no_registered_watch_folder_lands_in_the_unassigned_subdir() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = tempfile::tempdir().unwrap();
        insert_test_video(&db, "vid-unassigned", r"C:\Videos\a.mp4");
        // No watch folders registered at all.
        write_flat_thumbnail(thumbs_root.path(), "vid-unassigned", 0);

        let summary = migrate_flat_thumbnails_to_folder_subdirs(&db, thumbs_root.path()).unwrap();

        assert_eq!(summary.videos_migrated, 1);
        let expected_dir = thumbs_root
            .path()
            .join(gb_core::paths::THUMBNAIL_UNASSIGNED_SUBDIR);
        assert!(expected_dir.join("vid-unassigned_0.webp").exists());
    }

    #[test]
    fn a_second_run_after_a_full_migration_is_a_no_op() {
        let (_db_dir, db) = init_temp_db();
        let thumbs_root = tempfile::tempdir().unwrap();
        insert_test_video(&db, "vid-1", r"C:\Videos\a.mp4");
        {
            let conn = db.writer.lock().unwrap();
            db_queries::set_watch_folders(&conn, &[r"C:\Videos".to_string()]).unwrap();
        }
        for slot in 0..THUMBNAILS_PER_VIDEO {
            write_flat_thumbnail(thumbs_root.path(), "vid-1", slot);
        }

        let first = migrate_flat_thumbnails_to_folder_subdirs(&db, thumbs_root.path()).unwrap();
        assert_eq!(first.videos_migrated, 1);

        let second = migrate_flat_thumbnails_to_folder_subdirs(&db, thumbs_root.path()).unwrap();
        assert_eq!(second.videos_migrated, 0);
        assert_eq!(second.videos_failed, 0);
    }

    #[test]
    fn a_missing_thumbnails_root_is_a_no_op_rather_than_an_error() {
        let (_db_dir, db) = init_temp_db();
        let tmp = tempfile::tempdir().unwrap();
        let thumbs_root = tmp.path().join("never-created");

        let summary = migrate_flat_thumbnails_to_folder_subdirs(&db, &thumbs_root).unwrap();

        assert_eq!(summary.videos_migrated, 0);
        assert_eq!(summary.videos_failed, 0);
    }
}
