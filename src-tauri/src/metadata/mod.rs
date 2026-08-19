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
pub fn enqueue_missing_metadata_probes<F, N>(db: Db, ffmpeg: Arc<F>, notifier: Arc<N>)
where
    F: FfmpegAdapter + 'static,
    N: CatalogNotifier + 'static,
{
    thread::spawn(move || {
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

        let worker_count = WORKER_THREAD_COUNT.min(pending.len());
        let pending: Arc<std::sync::Mutex<std::vec::IntoIter<(String, PathBuf)>>> =
            Arc::new(std::sync::Mutex::new(pending.into_iter()));

        let handles: Vec<_> = (0..worker_count)
            .map(|_| {
                let pending = Arc::clone(&pending);
                let db = db.clone();
                let ffmpeg = Arc::clone(&ffmpeg);
                let notifier = Arc::clone(&notifier);
                thread::spawn(move || loop {
                    let next = pending.lock().unwrap().next();
                    let Some((id, path)) = next else { break };
                    match worker::probe_metadata_for_video(ffmpeg.as_ref(), &db, &id, &path) {
                        Ok(()) => notifier.notify_changed(),
                        Err(e) => {
                            log::warn!("metadata worker: giving up on video {id}: {e}");
                        }
                    }
                })
            })
            .collect();

        for handle in handles {
            let _ = handle.join();
        }
    });
}
