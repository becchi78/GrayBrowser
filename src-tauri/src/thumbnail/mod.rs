//! Stateless thumbnail generation queue: no persisted job table.
//! "Resume after restart" falls out for free from re-enumerating `videos`
//! rows still missing a `thumbnails/[id].webp` file every time this runs.

pub mod worker;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use gb_core::ports::ffmpeg::FfmpegAdapter;

use crate::db::Db;
use crate::events::CatalogNotifier;

const WORKER_THREAD_COUNT: usize = 4;
const PAUSE_POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Clone)]
pub struct ThumbnailQueueHandle {
    paused: Arc<AtomicBool>,
}

impl Default for ThumbnailQueueHandle {
    fn default() -> Self {
        Self {
            paused: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl ThumbnailQueueHandle {
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }
}

/// Re-enumerates videos missing a thumbnail and spawns a small fixed worker
/// pool to generate them. Fire-and-forget: runs on a background thread and
/// never blocks the caller (`.setup()` at startup, or `start_scan` after a
/// scan completes -- so a scan never freezes the UI).
///
/// `notifier` is threaded in only here, not into `worker.rs` -- `worker.rs`'s
/// `generate_thumbnail_for_video`/`list_videos_missing_thumbnails` keep their
/// existing pure/testable signatures unchanged, and the
/// `notify_changed` call sits in this orchestration
/// function's own per-worker loop instead, right after each individual
/// `generate_thumbnail_for_video` call succeeds -- one notification per
/// completed thumbnail, not one at the very end, so the grid fills in
/// incrementally (matching the "list first, thumbnails backfill"
/// experience) rather than freezing until the whole batch is done.
pub fn enqueue_missing_thumbnails<F, N>(
    db: Db,
    thumbnails_dir: PathBuf,
    queue: ThumbnailQueueHandle,
    ffmpeg: Arc<F>,
    notifier: Arc<N>,
) where
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
                "ffmpeg/ffprobe not found on PATH -- skipping thumbnail generation for this run"
            );
            return;
        }

        let pending = match worker::list_videos_missing_thumbnails(&db, &thumbnails_dir) {
            Ok(pending) => pending,
            Err(e) => {
                log::error!("failed to list videos missing thumbnails: {e}");
                return;
            }
        };
        if pending.is_empty() {
            return;
        }

        let worker_count = WORKER_THREAD_COUNT.min(pending.len());
        let pending = Arc::new(std::sync::Mutex::new(pending.into_iter()));

        let handles: Vec<_> = (0..worker_count)
            .map(|_| {
                let pending = Arc::clone(&pending);
                let db = db.clone();
                let thumbnails_dir = thumbnails_dir.clone();
                let queue = queue.clone();
                let ffmpeg = Arc::clone(&ffmpeg);
                let notifier = Arc::clone(&notifier);
                thread::spawn(move || loop {
                    while queue.is_paused() {
                        thread::sleep(PAUSE_POLL_INTERVAL);
                    }
                    let next = pending.lock().unwrap().next();
                    let Some((id, path)) = next else { break };
                    match worker::generate_thumbnail_for_video(
                        ffmpeg.as_ref(),
                        &db,
                        &thumbnails_dir,
                        &id,
                        &path,
                    ) {
                        Ok(()) => notifier.notify_changed(),
                        Err(e) => {
                            log::warn!("thumbnail worker: giving up on video {id}: {e}");
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
