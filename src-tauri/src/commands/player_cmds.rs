use std::path::Path;
use std::sync::Arc;

use gb_core::ports::file_probe::{FileCheckError, FileProbe};
use gb_core::ports::player::PlayerLauncher;
use tauri::State;

use crate::adapters;
use crate::db::{queries, Db};
use crate::dedup::{self, DuplicateGroupsState};
use crate::events::{CatalogNotifier, TauriCatalogNotifier, TauriDedupNotifier};
use crate::reachability::ReachabilityRegistry;

pub(crate) const WENT_OFFLINE_MSG: &str = "ファイルが見つからないため、オフラインにしました";

/// What the play path needs besides the launcher, bundled so a test can
/// swap every piece for a fake.
pub(crate) struct PlayDeps<'a> {
    pub db: &'a Db,
    pub reachability: &'a ReachabilityRegistry,
    pub files: &'a dyn FileProbe,
    pub notifier: &'a dyn CatalogNotifier,
    /// Recomputes the duplicate groups; the panel is not driven by the
    /// catalog-changed event.
    pub refresh_duplicates: &'a dyn Fn(),
}

/// `override_player` is always `None`: the OS default file association is
/// the only supported way to pick a player (there is no player-picker UI).
///
/// The existence check runs first so an unreachable path (e.g. a powered-off
/// NAS) never reaches `ShellExecuteExW`: that call would block for ~20s and
/// then pop up a Windows "network error" dialog the app cannot suppress.
fn launch_video(
    deps: &PlayDeps,
    launcher: &impl PlayerLauncher,
    video_path: &Path,
) -> Result<(), String> {
    match deps.files.check(video_path) {
        Ok(()) => launcher.launch(video_path, None).map_err(|e| e.to_string()),
        Err(FileCheckError::NotFound(msg)) => {
            if offline_if_share_reachable(deps, video_path) {
                Err(WENT_OFFLINE_MSG.to_string())
            } else {
                Err(msg)
            }
        }
        Err(FileCheckError::Other(msg)) => Err(msg),
    }
}

/// An unreachable share also reports "not found", so the video is marked
/// offline only after the root of its watch folder is probed again and
/// answers. Only this one video is touched. Returns whether the video is
/// (now) offline; any failure along the way leaves the state alone and
/// returns `false`, so the caller shows the plain error.
fn offline_if_share_reachable(deps: &PlayDeps, video_path: &Path) -> bool {
    let path = video_path.to_string_lossy();
    let row = match deps
        .db
        .read_pool
        .get()
        .map_err(|e| e.to_string())
        .and_then(|conn| queries::find_video_by_path(&conn, &path).map_err(|e| e.to_string()))
    {
        Ok(Some(row)) => row,
        Ok(None) => return false,
        Err(e) => {
            log::warn!("could not look up {path} after it was not found: {e}");
            return false;
        }
    };
    let watch_folders = match queries::read_watch_folders(&deps.db.read_pool) {
        Ok(folders) => folders,
        Err(e) => {
            log::warn!("could not read the watch folders: {e}");
            return false;
        }
    };
    if gb_core::paths::owning_folder(&path, &watch_folders).is_none()
        || deps
            .reachability
            .confirm_path_unreachable(&path, &watch_folders)
    {
        return false;
    }
    if row.status == "offline" {
        return true;
    }
    let changed = {
        let mut conn = deps.db.writer.lock().unwrap();
        queries::mark_videos_offline(&mut conn, std::slice::from_ref(&row.id))
    };
    match changed {
        Ok(n) => {
            if n > 0 {
                log::info!("marked {path} offline: not found at play time");
                deps.notifier.notify_changed();
                (deps.refresh_duplicates)();
            }
            true
        }
        Err(e) => {
            log::warn!("could not mark {path} offline: {e}");
            false
        }
    }
}

/// Runs on a blocking worker so a slow shell launch or share probe can never
/// freeze the UI thread (a sync command would execute on the main thread).
#[tauri::command]
pub async fn play_video(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    reachability: State<'_, ReachabilityRegistry>,
    dedup_state: State<'_, DuplicateGroupsState>,
    file_path: String,
) -> Result<(), String> {
    let db = db.inner().clone();
    let reachability = reachability.inner().clone();
    let dedup_state = dedup_state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let notifier = TauriCatalogNotifier::new(app.clone());
        let refresh_duplicates = {
            let db = db.clone();
            let reachability = reachability.clone();
            let app = app.clone();
            move || {
                dedup::refresh_duplicate_groups(
                    db.clone(),
                    dedup_state.clone(),
                    Arc::new(TauriDedupNotifier::new(app.clone())),
                    reachability.clone(),
                );
            }
        };
        let deps = PlayDeps {
            db: &db,
            reachability: &reachability,
            files: &adapters::file_probe::RealFileProbe,
            notifier: &notifier,
            refresh_duplicates: &refresh_duplicates,
        };
        let launcher = adapters::player::RealPlayerLauncher::new(app);
        match launch_video(&deps, &launcher, Path::new(&file_path)) {
            Ok(()) => {
                log::info!("launched external player for {file_path}");
                Ok(())
            }
            Err(e) => {
                log::warn!("failed to launch external player for {file_path}: {e}");
                Err(e)
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::FakeCatalogNotifier;
    use crate::test_support::{registry_over, LibraryEnv};
    use gb_core::ports::player::LaunchError;
    use gb_core::testing::fake_file_probe::FakeFileProbe;
    use gb_core::testing::fake_folder_probe::FakeFolderProbe;
    use gb_core::testing::fake_player::FakePlayerLauncher;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn status_of(env: &LibraryEnv, id: &str) -> String {
        queries::find_video_by_id(&env.db.read_pool, id)
            .unwrap()
            .unwrap()
            .status
    }

    struct Outcome {
        result: Result<(), String>,
        notified: usize,
        dedup_refreshes: usize,
        launches: usize,
    }

    /// Runs the play path against `path` with `files` as the file system and
    /// `probe` as the share state.
    fn play_with(
        env: &LibraryEnv,
        files: &FakeFileProbe,
        probe: FakeFolderProbe,
        launcher: &FakePlayerLauncher,
        path: &Path,
    ) -> Outcome {
        let reachability = registry_over(Arc::new(probe));
        let notifier = FakeCatalogNotifier::default();
        let dedup_refreshes = AtomicUsize::new(0);
        let refresh = || {
            dedup_refreshes.fetch_add(1, Ordering::SeqCst);
        };
        let deps = PlayDeps {
            db: &env.db,
            reachability: &reachability,
            files,
            notifier: &notifier,
            refresh_duplicates: &refresh,
        };
        let result = launch_video(&deps, launcher, path);
        Outcome {
            result,
            notified: notifier.calls(),
            dedup_refreshes: dedup_refreshes.load(Ordering::SeqCst),
            launches: launcher.calls.lock().unwrap().len(),
        }
    }

    fn play(
        env: &LibraryEnv,
        files: &FakeFileProbe,
        probe: FakeFolderProbe,
        path: &Path,
    ) -> Outcome {
        let launcher = FakePlayerLauncher {
            result: Ok(()),
            ..Default::default()
        };
        play_with(env, files, probe, &launcher, path)
    }

    fn not_found(path: &Path) -> FakeFileProbe {
        FakeFileProbe::with_error(
            &path.to_string_lossy(),
            FileCheckError::NotFound("os says no".into()),
        )
    }

    #[test]
    fn present_file_is_launched_and_nothing_changes() {
        let env = LibraryEnv::new();
        let path = env.add_video_with("a", "online", b"x");
        let out = play(
            &env,
            &FakeFileProbe::default(),
            FakeFolderProbe::default(),
            &path,
        );
        assert!(out.result.is_ok());
        assert_eq!(out.launches, 1);
        assert_eq!(out.notified, 0);
        assert_eq!(status_of(&env, "a"), "online");
    }

    #[test]
    fn launch_failure_is_propagated_as_a_string_error() {
        let env = LibraryEnv::new();
        let path = env.add_video_with("a", "online", b"x");
        let launcher = FakePlayerLauncher {
            result: Err(LaunchError::Spawn("no association".into())),
            ..Default::default()
        };
        let out = play_with(
            &env,
            &FakeFileProbe::default(),
            FakeFolderProbe::default(),
            &launcher,
            &path,
        );
        assert!(out.result.unwrap_err().contains("no association"));
        assert_eq!(status_of(&env, "a"), "online");
    }

    #[test]
    fn missing_file_on_a_reachable_share_goes_offline_and_notifies() {
        let env = LibraryEnv::new();
        let path = env.add_video_with("a", "online", b"x");
        env.add_video_with("b", "online", b"y");
        let out = play(&env, &not_found(&path), FakeFolderProbe::default(), &path);
        assert_eq!(out.result, Err(WENT_OFFLINE_MSG.to_string()));
        assert_eq!(out.launches, 0);
        assert_eq!(out.notified, 1);
        assert_eq!(out.dedup_refreshes, 1);
        assert_eq!(status_of(&env, "a"), "offline");
        // Only the clicked video is touched.
        assert_eq!(status_of(&env, "b"), "online");
    }

    #[test]
    fn missing_file_on_an_unreachable_share_stays_online() {
        let env = LibraryEnv::new();
        let path = env.add_video_with("a", "online", b"x");
        let folder = env.folder.to_string_lossy().into_owned();
        let probe = FakeFolderProbe::with_unreachable(&[folder.as_str()]);
        let out = play(&env, &not_found(&path), probe, &path);
        assert_eq!(out.result, Err("os says no".to_string()));
        assert_eq!(out.launches, 0);
        assert_eq!(out.notified, 0);
        assert_eq!(out.dedup_refreshes, 0);
        assert_eq!(status_of(&env, "a"), "online");
    }

    #[test]
    fn errors_other_than_not_found_leave_the_state_alone() {
        let env = LibraryEnv::new();
        let path = env.add_video_with("a", "online", b"x");
        let files = FakeFileProbe::with_error(
            &path.to_string_lossy(),
            FileCheckError::Other("access denied".into()),
        );
        let out = play(&env, &files, FakeFolderProbe::default(), &path);
        assert_eq!(out.result, Err("access denied".to_string()));
        assert_eq!(out.notified, 0);
        assert_eq!(status_of(&env, "a"), "online");
    }

    #[test]
    fn path_outside_every_watch_folder_stays_online() {
        let env = LibraryEnv::new();
        let outside = env.dir.path().join("elsewhere").join("c.mp4");
        env.insert_row("c", &outside, "online", 1);
        let out = play(
            &env,
            &not_found(&outside),
            FakeFolderProbe::default(),
            &outside,
        );
        assert_eq!(out.result, Err("os says no".to_string()));
        assert_eq!(out.notified, 0);
        assert_eq!(status_of(&env, "c"), "online");
    }

    #[test]
    fn path_without_a_row_is_not_offlined() {
        let env = LibraryEnv::new();
        let path = env.folder.join("unregistered.mp4");
        let out = play(&env, &not_found(&path), FakeFolderProbe::default(), &path);
        assert_eq!(out.result, Err("os says no".to_string()));
        assert_eq!(out.notified, 0);
    }

    #[test]
    fn already_offline_video_reports_offline_without_notifying() {
        let env = LibraryEnv::new();
        let path = env.add_video_with("a", "offline", b"x");
        let out = play(&env, &not_found(&path), FakeFolderProbe::default(), &path);
        assert_eq!(out.result, Err(WENT_OFFLINE_MSG.to_string()));
        assert_eq!(out.notified, 0);
        assert_eq!(out.dedup_refreshes, 0);
        assert_eq!(status_of(&env, "a"), "offline");
    }
}
