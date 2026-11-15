//! Small helpers shared by the thumbnail and metadata queues. The queues
//! themselves stay separate: they differ in what they list, claim and record.

use std::thread;

use gb_core::ports::ffmpeg::FfmpegAdapter;

/// Whether both `ffmpeg` and `ffprobe` were found; a failed check counts as
/// "not available".
pub(crate) fn ffmpeg_and_ffprobe_available<F: FfmpegAdapter + ?Sized>(ffmpeg: &F) -> bool {
    matches!(
        ffmpeg.check_available(),
        Ok(a) if a.ffmpeg_version.is_some() && a.ffprobe_version.is_some()
    )
}

/// Runs `worker_count` threads, each running a worker built by
/// `make_worker`, and returns once all of them have finished. A worker that
/// panicked is ignored so the others still get joined.
pub(crate) fn run_workers_to_completion<W>(worker_count: usize, mut make_worker: impl FnMut() -> W)
where
    W: FnOnce() + Send + 'static,
{
    let handles: Vec<_> = (0..worker_count)
        .map(|_| thread::spawn(make_worker()))
        .collect();
    for handle in handles {
        let _ = handle.join();
    }
}
