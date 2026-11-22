//! The stages `run`'s `setup` walks through, in order. Three orderings are
//! enforced by types, because the later stage needs a value only the earlier
//! one returns: every stage after `init_storage` before the database is open
//! (`Db`), the thumbnail preparation before the resume passes
//! (`PreparedThumbnailsRoot`),
//! and the reachability callbacks before the recheck and the watchers
//! (`CallbacksRegistered`). The rest -- where the startup log falls among the
//! `manage` calls, and the resume passes before the follow-up is built -- is
//! held only by the order `setup` calls these in.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gb_core::migrations::SchemaTooNew;
use tauri::Manager;

use crate::{
    adapters, db, dedup, events, logging, metadata, offline_sync, paths, reachability, thumbnail,
    watch,
};

/// Same error type as `setup` itself, so every `?` converts exactly as it did
/// when it was written directly in `setup`.
pub(crate) type SetupResult<T> = Result<T, Box<dyn std::error::Error>>;

/// Everything up to an open database. Logging and the legacy-layout move have
/// to happen before `db::init`, and the database is only handed out from
/// here, so nothing can use it before they are done.
pub(crate) fn init_storage() -> SetupResult<(PathBuf, db::Db, logging::LogLevelControl)> {
    let app_dir = paths::app_data_dir()?;
    std::fs::create_dir_all(&app_dir)?;
    // Before db::init so a migration failure is captured in app.log
    // too, not just wherever Tauri prints its own startup error.
    let log_control = logging::LogLevelControl::new(logging::init(&app_dir));
    // Before db::init: moves any pre-existing double-nested install
    // data (see `paths::migrate_legacy_nested_app_dir`) into place
    // first, so db::init opens the real database rather than
    // creating a fresh empty one.
    paths::migrate_legacy_nested_app_dir(&app_dir)?;
    let db = db::init(&app_dir.join("app.db"))
        .inspect_err(|e| log::error!("failed to initialize database: {e}"))
        // An `anyhow::Error` converted straight into a `Box<dyn Error>` can no
        // longer be downcast to its inner type, so the one error `setup`
        // reports specially is unwrapped here.
        .map_err(|e| match e.downcast::<SchemaTooNew>() {
            Ok(too_new) => Box::new(too_new) as Box<dyn std::error::Error>,
            Err(other) => other.into(),
        })?;
    Ok((app_dir, db, log_control))
}

/// Registers the log-level control and applies the level in force. Logging
/// starts at `info` before the database is open, so the stored level can only
/// be applied here.
pub(crate) fn manage_log_level(app: &tauri::App, db: &db::Db, control: logging::LogLevelControl) {
    match db.read_pool.get() {
        Ok(conn) => {
            if let Err(e) =
                crate::commands::log_cmds::apply_effective_log_level(&conn, |l| control.apply(l))
            {
                log::warn!("could not apply the stored log level: {e}");
            }
        }
        Err(e) => log::warn!("could not apply the stored log level: {e}"),
    }
    app.manage(control);
}

const SCHEMA_TOO_NEW_TITLE: &str = "GrayBrowser を起動できません";

/// The text for the dialog shown when the database is newer than this app.
/// Kept free of any dialog or path lookup so the wording can be tested.
pub(crate) fn schema_too_new_message(err: &SchemaTooNew, app_dir: &Path) -> String {
    [
        format!(
            "データベースがこのアプリより新しい版で作られています(データベース: 版{}、このアプリ: 版{})。",
            err.db_version, err.app_version
        ),
        "データを守るため、起動を中止します。".to_string(),
        String::new(),
        "アプリを最新の版に更新してください。".to_string(),
        "Draft版から安定版へ戻した場合は、移行前のバックアップから復元できます。".to_string(),
        format!(
            "バックアップフォルダ: {}",
            db::backup::backups_dir(app_dir).display()
        ),
        "復元の手順は docs/backup.md を参照してください。".to_string(),
    ]
    .join("
")
}

/// Shows the guidance when `err` is a too-new database. Returns whether it
/// was one; other startup errors are left to the caller's usual handling.
pub(crate) fn report_schema_too_new(
    app: &tauri::App,
    err: &(dyn std::error::Error + 'static),
) -> bool {
    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};

    let Some(too_new) = err.downcast_ref::<SchemaTooNew>() else {
        return false;
    };
    let message = match paths::app_data_dir() {
        Ok(dir) => schema_too_new_message(too_new, &dir),
        Err(_) => schema_too_new_message(too_new, Path::new("")),
    };
    log::error!("refusing to start: {too_new}");
    app.dialog()
        .message(message)
        .title(SCHEMA_TOO_NEW_TITLE)
        .kind(MessageDialogKind::Error)
        .blocking_show();
    true
}

/// State shared with the commands, created and registered together so the
/// registration order stays in one place.
pub(crate) struct SharedState {
    queue: thumbnail::ThumbnailQueueHandle,
    dedup_state: dedup::DuplicateGroupsState,
    reachability: reachability::ReachabilityRegistry,
}

pub(crate) fn manage_shared_state(app: &tauri::App, db: &db::Db) -> SharedState {
    app.manage(db.clone());

    let queue = thumbnail::ThumbnailQueueHandle::default();
    app.manage(queue.clone());

    let dedup_state = dedup::DuplicateGroupsState::default();
    app.manage(dedup_state.clone());

    // Which watch folders cannot be reached right now. Nothing is
    // probed here: the first probe of a folder happens on whichever
    // background thread needs the answer first, because a dead
    // network share blocks for ~20s and `setup` must not.
    let reachability = reachability::ReachabilityRegistry::new(
        Arc::new(adapters::folder_probe::RealFolderProbe),
        Arc::new(events::TauriReachabilityNotifier::new(app.handle().clone())),
    );
    app.manage(reachability.clone());

    SharedState {
        queue,
        dedup_state,
        reachability,
    }
}

pub(crate) fn log_startup(app_dir: &Path) {
    println!("graybrowser starting; app dir: {}", app_dir.display());
    log::info!("graybrowser starting; app dir: {}", app_dir.display());
}

/// The thumbnail root once `prepare_thumbnails` has migrated it and cleared
/// its leftovers. The field is private, so code outside this module cannot
/// make one, and here only `prepare_thumbnails` does; a path built some other
/// way cannot stand in for it.
pub(crate) struct PreparedThumbnailsRoot(PathBuf);

impl PreparedThumbnailsRoot {
    fn as_path(&self) -> &Path {
        &self.0
    }
}

/// Brings the thumbnail directory into its current layout and clears what an
/// earlier run left half-written. The root is only handed out from here, so
/// the resume passes cannot start on a layout that is still being migrated.
pub(crate) fn prepare_thumbnails(
    db: &db::Db,
    app_dir: &Path,
) -> SetupResult<PreparedThumbnailsRoot> {
    // One-time migration from the old flat thumbnail layout (every
    // `<video_id>_<slot>.webp` directly under `thumbnails/`) to
    // per-registered-folder subdirectories. Runs after db::init
    // (needs the DB to look up each leftover file's owning video)
    // and before enqueue_missing_thumbnails (which must see the
    // post-migration layout, not race it). Thumbnails are
    // regenerable, so a failure here is logged and startup
    // continues -- never `?`, unlike the DB migration in `init_storage`.
    let thumbnails_root = thumbnail::paths::thumbnails_root(app_dir);
    std::fs::create_dir_all(&thumbnails_root)?;
    if let Err(e) =
        thumbnail::migration::migrate_flat_thumbnails_to_folder_subdirs(db, &thumbnails_root)
    {
        log::error!("thumbnail layout migration failed: {e}");
    }

    // Nothing is generating yet, so every leftover .tmp is an orphan.
    // Clearing them before the queue starts keeps stalled videos from
    // carrying stale partial output into regeneration.
    let removed = thumbnail::remove_orphan_tmp_files(&thumbnails_root);
    if removed > 0 {
        log::info!("removed {removed} orphan thumbnail .tmp file(s)");
    }
    Ok(PreparedThumbnailsRoot(thumbnails_root))
}

/// What the background stages need. The fields are private and `new` takes a
/// `PreparedThumbnailsRoot`, so a `Services` -- and with it every stage from
/// the resume passes on -- exists only after `prepare_thumbnails` has run.
pub(crate) struct Services {
    app_handle: tauri::AppHandle,
    db: db::Db,
    thumbnails_root: PreparedThumbnailsRoot,
    queue: thumbnail::ThumbnailQueueHandle,
    dedup_state: dedup::DuplicateGroupsState,
    reachability: reachability::ReachabilityRegistry,
    notifier: Arc<events::TauriCatalogNotifier<tauri::Wry>>,
}

impl Services {
    pub(crate) fn new(
        app_handle: tauri::AppHandle,
        db: db::Db,
        shared: SharedState,
        notifier: Arc<events::TauriCatalogNotifier<tauri::Wry>>,
        thumbnails_root: PreparedThumbnailsRoot,
    ) -> Self {
        Self {
            app_handle,
            db,
            thumbnails_root,
            queue: shared.queue,
            dedup_state: shared.dedup_state,
            reachability: shared.reachability,
            notifier,
        }
    }
}

pub(crate) fn start_resume_passes(s: &Services) {
    // Startup resume: re-enumerate videos missing a
    // thumbnail and kick off generation in the background.
    thumbnail::enqueue_missing_thumbnails(
        s.db.clone(),
        s.thumbnails_root.as_path().to_path_buf(),
        s.queue.clone(),
        Arc::new(adapters::ffmpeg::RealFfmpegAdapter),
        Arc::clone(&s.notifier),
        s.reachability.clone(),
    );

    // Startup resume: same stateless philosophy as the
    // thumbnail queue above -- re-enumerate videos missing metadata
    // (`probed_at IS NULL`) and kick off probing in the background.
    metadata::enqueue_missing_metadata_probes(
        s.db.clone(),
        Arc::new(adapters::ffmpeg::RealFfmpegAdapter),
        Arc::clone(&s.notifier),
        s.reachability.clone(),
    );

    // Startup resume: same
    // stateless philosophy as the two resume passes above --
    // duplicate detection isn't persisted as its own job either, it
    // just re-derives everything from `videos`/`path_collisions` on
    // every run.
    dedup::refresh_duplicate_groups(
        s.db.clone(),
        s.dedup_state.clone(),
        Arc::new(events::TauriDedupNotifier::new(s.app_handle.clone())),
        s.reachability.clone(),
    );
}

pub(crate) fn build_follow_up(s: &Services) -> Arc<watch::follow_up::FollowUp> {
    // Files the watchers/pollers register arrive with no thumbnail or
    // metadata; the follow-up re-runs the same enqueue pair a scan
    // does, once the burst of registrations settles.
    let db = s.db.clone();
    let thumbnails_root = s.thumbnails_root.as_path().to_path_buf();
    let queue = s.queue.clone();
    let notifier = Arc::clone(&s.notifier);
    let reachability = s.reachability.clone();
    Arc::new(watch::follow_up::FollowUp::new(move || {
        thumbnail::enqueue_missing_thumbnails(
            db.clone(),
            thumbnails_root.clone(),
            queue.clone(),
            Arc::new(adapters::ffmpeg::RealFfmpegAdapter),
            Arc::clone(&notifier),
            reachability.clone(),
        );
        metadata::enqueue_missing_metadata_probes(
            db.clone(),
            Arc::new(adapters::ffmpeg::RealFfmpegAdapter),
            Arc::clone(&notifier),
            reachability.clone(),
        );
    }))
}

/// Proof that the reachability callbacks are registered. The follow-up is
/// carried through it so `start_watching`, which needs the follow-up, can only
/// run once the callbacks are in place and no recovery can be missed.
pub(crate) struct CallbacksRegistered {
    follow_up: Arc<watch::follow_up::FollowUp>,
}

pub(crate) fn wire_folder_state(
    s: &Services,
    follow_up: Arc<watch::follow_up::FollowUp>,
) -> CallbacksRegistered {
    // A folder coming back resumes the work that was skipped while
    // it was down. The registry and the follow-up reference each
    // other, which is fine: both live for the whole process.
    //
    // The stored status follows the confirmed transitions too: the
    // folder's videos go offline when it is lost, and its own diff
    // scan brings back the files that are still there when it returns.
    // Both only enqueue; the worker does the writes and the scan.
    let folder_sync = Arc::new(offline_sync::FolderStateSync::start(
        offline_sync::SyncContext {
            db: s.db.clone(),
            thumbnails_root: s.thumbnails_root.as_path().to_path_buf(),
            notifier: s.notifier.clone(),
            follow_up: Arc::clone(&follow_up),
            reachability: s.reachability.clone(),
            refresh_duplicates: {
                let db = s.db.clone();
                let reachability = s.reachability.clone();
                let handle = s.app_handle.clone();
                let dedup_state = s.dedup_state.clone();
                Arc::new(move || {
                    dedup::refresh_duplicate_groups(
                        db.clone(),
                        dedup_state.clone(),
                        Arc::new(events::TauriDedupNotifier::new(handle.clone())),
                        reachability.clone(),
                    );
                })
            },
        },
    ));
    {
        let follow_up = Arc::clone(&follow_up);
        let folder_sync = Arc::clone(&folder_sync);
        s.reachability.set_on_recovered(move |folder| {
            follow_up.trigger();
            folder_sync.on_recovered(folder);
        });
    }
    {
        let folder_sync = Arc::clone(&folder_sync);
        s.reachability
            .set_on_unreachable(move |folder| folder_sync.on_unreachable(folder));
    }
    CallbacksRegistered { follow_up }
}

/// Starts the periodic recheck and the watchers, in that order. Returns the
/// manager for the caller to `manage`, since later folder-list changes
/// reconfigure the same instance.
pub(crate) fn start_watching(
    s: &Services,
    registered: CallbacksRegistered,
) -> SetupResult<watch::RealtimeWatchManager> {
    // Detached on purpose: it runs for the life of the process.
    let _ = s
        .reachability
        .start_recheck(reachability::DEFAULT_RECHECK_INTERVAL);
    // Local folders get realtime
    // `notify` watching, network folders get startup diff-scan +
    // periodic polling. `reconfigure` classifies each registered
    // folder by drive type and is re-invoked (with the manager kept
    // alive across calls) whenever `pick_watch_folders` changes the
    // folder list -- see commands::settings_cmds::pick_watch_folders.
    let watch_manager =
        watch::RealtimeWatchManager::new(registered.follow_up, s.reachability.clone());
    let (watch_folders, nas_poll_interval_secs) = {
        let conn = s.db.read_pool.get()?;
        (
            db::queries::get_watch_folders(&conn)?,
            db::queries::get_nas_poll_interval_secs(&conn)?,
        )
    };
    watch::reconfigure_real_watch_manager(
        &s.app_handle,
        &s.db,
        &watch_manager,
        s.thumbnails_root.as_path(),
        &watch_folders,
        nas_poll_interval_secs,
    );
    Ok(watch_manager)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_too_new_message_names_both_versions_and_the_backup_folder() {
        let err = SchemaTooNew {
            db_version: 20,
            app_version: 13,
        };
        let dir = Path::new("data");
        let msg = schema_too_new_message(&err, dir);
        assert!(msg.contains("版20"));
        assert!(msg.contains("版13"));
        assert!(msg.contains(&db::backup::backups_dir(dir).display().to_string()));
        assert!(msg.contains("docs/backup.md"));
        assert!(msg.lines().all(|l| l == l.trim_start()));
    }

    #[test]
    fn schema_too_new_survives_the_conversion_init_storage_applies() {
        let err = anyhow::Error::new(SchemaTooNew {
            db_version: 2,
            app_version: 1,
        });
        let boxed: Box<dyn std::error::Error> = match err.downcast::<SchemaTooNew>() {
            Ok(t) => Box::new(t),
            Err(o) => o.into(),
        };
        assert!(boxed.downcast_ref::<SchemaTooNew>().is_some());
    }
}
