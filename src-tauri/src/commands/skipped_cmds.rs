//! Commands acting on the "unregistered files" list (`skipped_files`).

use std::path::Path;
use std::sync::Arc;

use gb_core::ports::file_rename::{FileRenamer, RenameError};
use gb_core::rename_validation::validate_new_file_name;
use serde::Serialize;
use tauri::State;

use crate::adapters;
use crate::adapters::long_path;
use crate::db::{queries, Db};
use crate::events::{CatalogNotifier, TauriCatalogNotifier};
use crate::scan;
use crate::thumbnail::{self, ThumbnailQueueHandle};

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct RenameOutcome {
    pub new_file_name: String,
    /// `false` when the file was renamed but registering it failed; the next
    /// scan or watch event registers it, so the caller only needs to tell
    /// the user to expect that.
    pub registered: bool,
}

/// Renames an unregistered file in place and registers it as a normal video.
///
/// Order matters: the record is deleted only after the rename succeeded (a
/// failed rename must leave it so the user can retry), and registration comes
/// last and is allowed to fail without undoing the rename -- the file then has
/// a valid name and the regular scan picks it up.
fn rename_skipped_file_impl(
    db: &Db,
    renamer: &dyn FileRenamer,
    notifier: &dyn CatalogNotifier,
    register: impl Fn(&Path) -> anyhow::Result<()>,
    id: i64,
    input: &str,
) -> Result<RenameOutcome, String> {
    let row = queries::get_skipped_file(&db.read_pool, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "この未登録ファイルはすでに一覧にありません。".to_string())?;

    let new_name = validate_new_file_name(&row.file_name, input).map_err(|e| e.to_string())?;

    let old_path = Path::new(&row.file_path);
    // The parent comes from the recorded path, never from the input, so a
    // rename cannot leave the original folder.
    let parent = old_path
        .parent()
        .ok_or_else(|| "元のファイルの場所を特定できません。".to_string())?;
    let new_path = parent.join(&new_name);

    renamer
        .rename_no_overwrite(old_path, &new_path)
        .map_err(|e| match e {
            RenameError::TargetExists => "同じ名前のファイルがすでにあります。".to_string(),
            RenameError::SourceMissing => "元のファイルが見つかりません。".to_string(),
            RenameError::Failed { message } => {
                log::warn!("rename of {} failed: {message}", row.file_path);
                format!("名前を変更できませんでした（使用中の可能性があります）: {message}")
            }
        })?;
    log::info!("renamed unregistered file {} -> {new_name}", row.file_path);

    let delete_result = {
        let conn = db.writer.lock().unwrap();
        queries::delete_skipped_file(&conn, id)
    };
    if let Err(e) = &delete_result {
        log::error!("failed to delete skipped record {id} after rename: {e}");
    }

    let registered = match register(&new_path) {
        Ok(()) => true,
        Err(e) => {
            log::error!(
                "renamed {} but registering it failed: {e}",
                new_path.display()
            );
            false
        }
    };
    notifier.notify_changed();

    if delete_result.is_err() {
        return Err(
            "名前は変更しましたが、未登録の記録を更新できませんでした。再起動後に確認してください。"
                .to_string(),
        );
    }
    Ok(RenameOutcome {
        new_file_name: new_name,
        registered,
    })
}

#[tauri::command(async)]
pub fn rename_skipped_file(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    queue: State<'_, ThumbnailQueueHandle>,
    id: i64,
    new_name: String,
) -> Result<RenameOutcome, String> {
    let thumbnails_root = crate::paths::app_data_dir()
        .map(|dir| thumbnail::paths::thumbnails_root(&dir))
        .map_err(|e| e.to_string())?;
    let notifier = Arc::new(TauriCatalogNotifier::new(app));
    let register_db = db.inner().clone();
    let register_root = thumbnails_root.clone();
    // Same stat-then-`process_detected_file` path as the file watcher, so the
    // outcome is identical whichever of the two sees the renamed file first.
    let register = move |path: &Path| -> anyhow::Result<()> {
        let metadata = std::fs::metadata(long_path::to_long_path(path))?;
        let mtime = scan::mtime_from_metadata(&metadata).map_err(anyhow::Error::msg)?;
        match scan::process_detected_file(
            &register_db,
            &register_root,
            path,
            metadata.len(),
            mtime,
        )? {
            scan::ProcessOutcome::SkippedUnreadable => {
                anyhow::bail!("the file is locked or unreadable")
            }
            _ => Ok(()),
        }
    };

    let outcome = rename_skipped_file_impl(
        &db,
        &adapters::file_rename::RealFileRenamer,
        notifier.as_ref(),
        register,
        id,
        &new_name,
    )?;

    if outcome.registered {
        // Same "resume after registration" passes `start_scan` runs, so the
        // new video gets its thumbnails/duration without waiting for a scan.
        let _ = std::fs::create_dir_all(&thumbnails_root);
        thumbnail::enqueue_missing_thumbnails(
            db.inner().clone(),
            thumbnails_root,
            queue.inner().clone(),
            Arc::new(adapters::ffmpeg::RealFfmpegAdapter),
            Arc::clone(&notifier),
        );
        crate::metadata::enqueue_missing_metadata_probes(
            db.inner().clone(),
            Arc::new(adapters::ffmpeg::RealFfmpegAdapter),
            notifier,
        );
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::init_temp_db;
    use crate::events::FakeCatalogNotifier;
    use gb_core::scan_pipeline::NewSkippedFile;
    use gb_core::testing::fake_file_rename::FakeFileRenamer;
    use std::cell::RefCell;

    const OLD_PATH: &str = "C:/videos/movie\u{2460}.mp4";

    fn setup(existing: &[&str]) -> (tempfile::TempDir, Db, i64, FakeFileRenamer) {
        let (dir, db) = init_temp_db();
        queries::upsert_skipped_file(
            &db.writer.lock().unwrap(),
            &NewSkippedFile {
                file_path: OLD_PATH.to_string(),
                file_name: "movie\u{2460}.mp4".to_string(),
                reason: "machine_dependent_char",
                detected_char: '\u{2460}',
            },
        )
        .unwrap();
        let id = queries::list_skipped_files(&db.read_pool).unwrap()[0].id;
        let mut files = vec![OLD_PATH];
        files.extend_from_slice(existing);
        (dir, db, id, FakeFileRenamer::with_files(&files))
    }

    fn remaining(db: &Db) -> usize {
        queries::list_skipped_files(&db.read_pool).unwrap().len()
    }

    #[test]
    fn success_renames_deletes_the_record_registers_and_notifies() {
        let (_d, db, id, renamer) = setup(&[]);
        let notifier = FakeCatalogNotifier::default();
        let registered = RefCell::new(Vec::new());

        let out = rename_skipped_file_impl(
            &db,
            &renamer,
            &notifier,
            |p| {
                registered.borrow_mut().push(p.to_path_buf());
                Ok(())
            },
            id,
            " movie1.mp4 ",
        )
        .unwrap();

        assert_eq!(
            out,
            RenameOutcome {
                new_file_name: "movie1.mp4".into(),
                registered: true
            }
        );
        assert!(renamer.exists(Path::new("C:/videos/movie1.mp4")));
        assert!(!renamer.exists(Path::new(OLD_PATH)));
        assert_eq!(remaining(&db), 0);
        assert_eq!(
            registered.borrow().as_slice(),
            [Path::new("C:/videos").join("movie1.mp4")]
        );
        assert_eq!(notifier.calls(), 1);
    }

    #[test]
    fn rejected_input_touches_nothing() {
        for input in [
            "",
            "movie\u{2460}.mp4",
            "movie\u{2461}.mp4",
            "..\\x.mp4",
            "a.txt",
            "con.mp4",
        ] {
            let (_d, db, id, renamer) = setup(&[]);
            let notifier = FakeCatalogNotifier::default();
            let called = RefCell::new(false);

            let r = rename_skipped_file_impl(
                &db,
                &renamer,
                &notifier,
                |_| {
                    *called.borrow_mut() = true;
                    Ok(())
                },
                id,
                input,
            );

            assert!(r.is_err(), "{input:?}");
            assert!(renamer.calls.lock().unwrap().is_empty(), "{input:?}");
            assert_eq!(remaining(&db), 1, "{input:?}");
            assert!(!*called.borrow());
            assert_eq!(notifier.calls(), 0);
        }
    }

    #[test]
    fn unknown_id_is_an_error() {
        let (_d, db, id, renamer) = setup(&[]);
        let notifier = FakeCatalogNotifier::default();
        let r = rename_skipped_file_impl(&db, &renamer, &notifier, |_| Ok(()), id + 99, "a.mp4");
        assert!(r.is_err());
        assert!(renamer.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn existing_target_keeps_the_record_and_both_files() {
        let (_d, db, id, renamer) = setup(&["C:/videos/Movie1.mp4"]);
        let notifier = FakeCatalogNotifier::default();
        let called = RefCell::new(false);

        let r = rename_skipped_file_impl(
            &db,
            &renamer,
            &notifier,
            |_| {
                *called.borrow_mut() = true;
                Ok(())
            },
            id,
            "movie1.mp4",
        );

        assert!(r.unwrap_err().contains("同じ名前"));
        assert_eq!(remaining(&db), 1);
        assert!(renamer.exists(Path::new(OLD_PATH)));
        assert!(renamer.exists(Path::new("C:/videos/Movie1.mp4")));
        assert!(!*called.borrow());
        assert_eq!(notifier.calls(), 0);
    }

    #[test]
    fn rename_failure_keeps_the_record() {
        let (_d, db, id, renamer) = setup(&[]);
        *renamer.forced_error.lock().unwrap() = Some(RenameError::Failed {
            message: "locked".into(),
        });
        let notifier = FakeCatalogNotifier::default();
        let called = RefCell::new(false);

        let r = rename_skipped_file_impl(
            &db,
            &renamer,
            &notifier,
            |_| {
                *called.borrow_mut() = true;
                Ok(())
            },
            id,
            "movie1.mp4",
        );

        assert!(r.unwrap_err().contains("locked"));
        assert_eq!(remaining(&db), 1);
        assert!(!*called.borrow());
        assert_eq!(notifier.calls(), 0);
    }

    #[test]
    fn missing_source_keeps_the_record() {
        let (_d, db, id, renamer) = setup(&[]);
        *renamer.forced_error.lock().unwrap() = Some(RenameError::SourceMissing);
        let notifier = FakeCatalogNotifier::default();
        let r = rename_skipped_file_impl(&db, &renamer, &notifier, |_| Ok(()), id, "movie1.mp4");
        assert!(r.unwrap_err().contains("見つかりません"));
        assert_eq!(remaining(&db), 1);
    }

    #[test]
    fn registration_failure_still_drops_the_record_and_reports_renamed() {
        let (_d, db, id, renamer) = setup(&[]);
        let notifier = FakeCatalogNotifier::default();

        let out = rename_skipped_file_impl(
            &db,
            &renamer,
            &notifier,
            |_| anyhow::bail!("locked"),
            id,
            "movie1.mp4",
        )
        .unwrap();

        assert_eq!(
            out,
            RenameOutcome {
                new_file_name: "movie1.mp4".into(),
                registered: false
            }
        );
        assert!(renamer.exists(Path::new("C:/videos/movie1.mp4")));
        assert_eq!(remaining(&db), 0);
        assert_eq!(notifier.calls(), 1);
    }
}
