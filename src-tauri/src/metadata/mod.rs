//! Stateless metadata-probing queue: no persisted job table, same
//! philosophy as `thumbnail`'s -- "resume after restart" falls out
//! for free from re-enumerating `videos` rows with `probed_at IS NULL` every
//! time this runs.

pub mod worker;

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use gb_core::ports::ffmpeg::FfmpegAdapter;

use crate::db::Db;
use crate::events::CatalogNotifier;
use crate::reachability::ReachabilityRegistry;

/// Fewer workers than the thumbnail pool's 4 -- an ffprobe metadata query is
/// far cheaper than a full frame extraction+encode, and running both pools
/// at full size right after a scan would otherwise contend harder than
/// necessary on the single writer lock.
const WORKER_THREAD_COUNT: usize = 2;

/// Re-enumerates videos missing metadata and spawns a small fixed worker
/// pool to probe them. Fire-and-forget, mirroring
/// `thumbnail::enqueue_missing_thumbnails`'s shape (same fixed-pool-pulls-
/// from-shared-iterator structure, same per-item `notify_changed` -- no new
/// event name, reusing the established "frontend just re-fetches" pattern).
/// Metadata probing doesn't need pause/resume control the way thumbnail
/// generation does, so unlike `ThumbnailQueueHandle` there's no pause flag
/// here.
///
/// Like the thumbnail queue, videos under an unreachable watch folder are
/// skipped without spending a retry, and are picked up again by the follow-up
/// run when the folder returns.
pub fn enqueue_missing_metadata_probes<F, N>(
    db: Db,
    ffmpeg: Arc<F>,
    notifier: Arc<N>,
    reachability: ReachabilityRegistry,
) where
    F: FfmpegAdapter + 'static,
    N: CatalogNotifier + 'static,
{
    thread::spawn(move || run_metadata_queue(db, ffmpeg, notifier, reachability));
}

/// The body of `enqueue_missing_metadata_probes`, split out so tests can run
/// it synchronously.
fn run_metadata_queue<F, N>(
    db: Db,
    ffmpeg: Arc<F>,
    notifier: Arc<N>,
    reachability: ReachabilityRegistry,
) where
    F: FfmpegAdapter + 'static,
    N: CatalogNotifier + 'static,
{
    {
        let available = matches!(
            ffmpeg.check_available(),
            Ok(a) if a.ffmpeg_version.is_some() && a.ffprobe_version.is_some()
        );
        if !available {
            log::warn!(
                "ffmpeg/ffprobe not found on PATH -- skipping metadata probing for this run"
            );
            return;
        }

        let pending = match worker::list_videos_missing_metadata(&db) {
            Ok(pending) => pending,
            Err(e) => {
                log::error!("failed to list videos missing metadata: {e}");
                return;
            }
        };
        if pending.is_empty() {
            return;
        }

        let watch_folders = match db.read_pool.get() {
            Ok(conn) => match crate::db::queries::get_watch_folders(&conn) {
                Ok(folders) => folders,
                Err(e) => {
                    log::error!("failed to read watch_folders before probing metadata: {e}");
                    return;
                }
            },
            Err(e) => {
                log::error!("failed to acquire a DB connection to read watch_folders: {e}");
                return;
            }
        };
        let watch_folders = Arc::new(watch_folders);

        let worker_count = WORKER_THREAD_COUNT.min(pending.len());
        let pending: Arc<std::sync::Mutex<std::vec::IntoIter<(String, PathBuf)>>> =
            Arc::new(std::sync::Mutex::new(pending.into_iter()));

        let handles: Vec<_> = (0..worker_count)
            .map(|_| {
                let pending = Arc::clone(&pending);
                let db = db.clone();
                let ffmpeg = Arc::clone(&ffmpeg);
                let notifier = Arc::clone(&notifier);
                let reachability = reachability.clone();
                let watch_folders = Arc::clone(&watch_folders);
                thread::spawn(move || loop {
                    let next = pending.lock().unwrap().next();
                    let Some((id, path)) = next else { break };
                    // The first question about a folder may block for as long
                    // as the share takes to time out; that happens here on
                    // the worker, and later videos of the folder get the
                    // cached answer.
                    let path_str = path.to_string_lossy();
                    if reachability.is_path_unreachable(&path_str, &watch_folders) {
                        log::info!(
                            "metadata worker: skipping {id}, its folder is unreachable right now"
                        );
                        continue;
                    }
                    match worker::probe_metadata_for_video(ffmpeg.as_ref(), &db, &id, &path) {
                        Ok(()) => notifier.notify_changed(),
                        Err(e) if reachability.confirm_path_unreachable(&path_str, &watch_folders) => {
                            // The share went away during the attempt, so the
                            // failure says nothing about the video: give the
                            // retry back (the worker already counted it).
                            log::warn!(
                                "metadata worker: {id} failed because its folder became unreachable: {e}"
                            );
                            if let Err(refund_err) = crate::db::queries::refund_metadata_attempt(
                                &db.writer.lock().unwrap(),
                                &id,
                            ) {
                                log::warn!(
                                    "failed to refund the metadata attempt for {id}: {refund_err}"
                                );
                            }
                        }
                        Err(e) => {
                            log::warn!("metadata worker: giving up on video {id}: {e}");
                            // A failure raises `metadata_attempts`, which can
                            // move the video into the "generation failures"
                            // list; without this a bulk re-probe that fails
                            // everywhere would never refresh that list.
                            notifier.notify_changed();
                        }
                    }
                })
            })
            .collect();

        for handle in handles {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{init_temp_db, insert_test_video};
    use gb_core::ports::ffmpeg::{FfmpegAvailability, FfmpegError};
    use gb_core::testing::fake_ffmpeg::{FakeCall, FakeFfmpegAdapter};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use crate::events::NoopReachabilityNotifier;
    use gb_core::testing::fake_folder_probe::FakeFolderProbe;

    fn registry_over(probe: Arc<FakeFolderProbe>) -> ReachabilityRegistry {
        ReachabilityRegistry::new(probe, Arc::new(NoopReachabilityNotifier))
    }

    fn all_reachable() -> ReachabilityRegistry {
        registry_over(Arc::new(FakeFolderProbe::default()))
    }

    struct CountingNotifier(AtomicUsize);

    impl CatalogNotifier for CountingNotifier {
        fn notify_changed(&self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn a_failed_probe_also_notifies_the_catalog_changed() {
        let (_db_dir, db) = init_temp_db();
        insert_test_video(&db, "vid-fail", "C:/videos/fail.mp4");
        let notifier = Arc::new(CountingNotifier(AtomicUsize::new(0)));
        let ffmpeg = Arc::new(FakeFfmpegAdapter {
            availability: Ok(FfmpegAvailability {
                ffmpeg_version: Some("7.0".into()),
                ffprobe_version: Some("7.0".into()),
            }),
            metadata: Err(FfmpegError::NonZeroExit {
                status: 1,
                stderr: "corrupt file".into(),
            }),
            ..Default::default()
        });

        enqueue_missing_metadata_probes(db.clone(), ffmpeg, Arc::clone(&notifier), all_reachable());

        // The queue is fire-and-forget on a detached thread, so poll for the
        // notification instead of joining.
        let deadline = Instant::now() + Duration::from_secs(10);
        while notifier.0.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(notifier.0.load(Ordering::SeqCst), 1);
    }

    const NAS: &str = "W:/video";

    fn failing_probe_ffmpeg() -> Arc<FakeFfmpegAdapter> {
        Arc::new(FakeFfmpegAdapter {
            availability: Ok(FfmpegAvailability {
                ffmpeg_version: Some("7.0".into()),
                ffprobe_version: Some("7.0".into()),
            }),
            metadata: Err(FfmpegError::NonZeroExit {
                status: 1,
                stderr: "network error".into(),
            }),
            ..Default::default()
        })
    }

    fn metadata_attempts_of(db: &Db, id: &str) -> i64 {
        db.writer
            .lock()
            .unwrap()
            .query_row(
                "SELECT metadata_attempts FROM videos WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn register_nas_video(db: &Db, id: &str) {
        insert_test_video(db, id, &format!("{NAS}/{id}.mp4"));
        crate::db::queries::set_watch_folders(
            &db.writer.lock().unwrap(),
            &[NAS.to_string(), "C:/videos".to_string()],
        )
        .unwrap();
    }

    #[test]
    fn videos_in_an_unreachable_folder_are_skipped_without_spending_attempts() {
        let (_db_dir, db) = init_temp_db();
        register_nas_video(&db, "nas-1");
        register_nas_video(&db, "nas-2");
        insert_test_video(&db, "local-1", "C:/videos/local-1.mp4");
        let probe = Arc::new(FakeFolderProbe::with_unreachable(&[NAS]));
        let ffmpeg = failing_probe_ffmpeg();
        let notifier = Arc::new(CountingNotifier(AtomicUsize::new(0)));

        run_metadata_queue(
            db.clone(),
            Arc::clone(&ffmpeg),
            Arc::clone(&notifier),
            registry_over(Arc::clone(&probe)),
        );

        assert_eq!(metadata_attempts_of(&db, "nas-1"), 0);
        assert_eq!(metadata_attempts_of(&db, "nas-2"), 0);
        // The reachable folder's video is still attempted (and fails here).
        assert_eq!(metadata_attempts_of(&db, "local-1"), 1);
        assert_eq!(probe.call_count_for(NAS), 1, "probed once, then cached");
        let nas_probes = ffmpeg
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| matches!(c, FakeCall::ProbeMetadata(p) if p.starts_with(NAS)))
            .count();
        assert_eq!(nas_probes, 0, "nothing was opened on the NAS");
    }

    #[test]
    fn a_failure_while_the_folder_just_went_down_does_not_spend_an_attempt() {
        let (_db_dir, db) = init_temp_db();
        register_nas_video(&db, "nas-1");
        let probe = Arc::new(FakeFolderProbe::default());
        let registry = registry_over(Arc::clone(&probe));
        // Known reachable, so the worker goes ahead; the share then drops.
        assert!(registry.is_reachable(NAS));
        probe.set_reachable(NAS, false);

        run_metadata_queue(
            db.clone(),
            failing_probe_ffmpeg(),
            Arc::new(CountingNotifier(AtomicUsize::new(0))),
            registry.clone(),
        );

        assert_eq!(metadata_attempts_of(&db, "nas-1"), 0);
        assert_eq!(registry.unreachable_folders(), vec![NAS.to_string()]);
    }

    #[test]
    fn a_failure_in_a_reachable_folder_still_spends_an_attempt() {
        let (_db_dir, db) = init_temp_db();
        register_nas_video(&db, "nas-1");

        run_metadata_queue(
            db.clone(),
            failing_probe_ffmpeg(),
            Arc::new(CountingNotifier(AtomicUsize::new(0))),
            all_reachable(),
        );

        assert_eq!(metadata_attempts_of(&db, "nas-1"), 1);
    }
}
