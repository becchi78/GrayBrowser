//! Manual scan restricted to the watch folders that can currently be reached.

use crate::db::Db;
use crate::reachability::ReachabilityRegistry;
use crate::scan;

/// Splits `folders` into (reachable, unreachable), each in the original order.
///
/// Uses the registry's cached answer rather than forcing a fresh probe: the
/// background recheck keeps it current, so a share known to be down is skipped
/// at once, whereas a forced probe would block ~20s on it every scan. Only
/// folders never asked about before are probed here, one thread each, so
/// several cold shares cost one wait instead of one per share.
///
/// Folders already published as unreachable are decided from a non-blocking
/// snapshot without asking `is_reachable`: that call waits for a probe in
/// flight, and the background recheck keeps a down share mid-probe for ~20s
/// out of every minute, which would stall the scan on exactly the share it
/// should skip. The registry keys folders by exact string, so the snapshot is
/// matched the same way.
fn partition_by_reachability(
    folders: &[String],
    reachability: &ReachabilityRegistry,
) -> (Vec<String>, Vec<String>) {
    let known_down = reachability.unreachable_folders();
    let answers: Vec<bool> = std::thread::scope(|scope| {
        let handles: Vec<_> = folders
            .iter()
            .map(|folder| {
                let down = known_down.contains(folder);
                scope.spawn(move || !down && reachability.is_reachable(folder))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("reachability probe thread panicked"))
            .collect()
    });
    let mut reachable = Vec::new();
    let mut unreachable = Vec::new();
    for (folder, ok) in folders.iter().zip(answers) {
        if ok {
            reachable.push(folder.clone());
        } else {
            log::warn!("watch folder unreachable, skipping manual scan: {folder}");
            unreachable.push(folder.clone());
        }
    }
    (reachable, unreachable)
}

/// Scans only the folders that can be reached. A skipped folder is never
/// handed to `scan_folders`, so its videos are not judged against an empty
/// listing and cannot be flipped offline by an outage.
pub(crate) fn scan_reachable_folders(
    folders: &[String],
    reachability: &ReachabilityRegistry,
    db: &Db,
    thumbnails_root: &std::path::Path,
) -> anyhow::Result<scan::ScanSummary> {
    let (reachable, unreachable) = partition_by_reachability(folders, reachability);
    let mut summary = scan::scan_folders(&reachable, db, thumbnails_root, reachability)?;
    summary.unreachable_folders = unreachable;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::queries;
    use crate::test_support::{registry_over, strings};
    use std::sync::Arc;

    #[test]
    fn partition_keeps_reachable_folders_in_original_order() {
        use gb_core::testing::fake_folder_probe::FakeFolderProbe;
        let registry = registry_over(Arc::new(FakeFolderProbe::default()));
        let folders = strings(&["C:/c", "C:/a", "C:/b"]);

        let (reachable, unreachable) = partition_by_reachability(&folders, &registry);

        assert_eq!(reachable, folders);
        assert!(unreachable.is_empty());
    }

    #[test]
    fn partition_separates_unreachable_folders_keeping_order() {
        use gb_core::testing::fake_folder_probe::FakeFolderProbe;
        let registry = registry_over(Arc::new(FakeFolderProbe::with_unreachable(&[
            "W:/z", "W:/y",
        ])));
        let folders = strings(&["W:/z", "C:/a", "W:/y", "C:/b"]);

        let (reachable, unreachable) = partition_by_reachability(&folders, &registry);

        assert_eq!(reachable, strings(&["C:/a", "C:/b"]));
        assert_eq!(unreachable, strings(&["W:/z", "W:/y"]));
    }

    #[test]
    fn partition_probes_folders_that_were_never_checked_and_uses_the_cache_after() {
        use gb_core::testing::fake_folder_probe::FakeFolderProbe;
        let probe = Arc::new(FakeFolderProbe::with_unreachable(&["W:/n"]));
        let registry = registry_over(probe.clone());
        let folders = strings(&["C:/a", "W:/n"]);

        partition_by_reachability(&folders, &registry);
        // The share comes back, but the cached answer is what a scan trusts;
        // the background recheck is what notices the recovery.
        probe.set_reachable("W:/n", true);
        let (_, unreachable) = partition_by_reachability(&folders, &registry);

        assert_eq!(probe.call_count_for("C:/a"), 1);
        assert_eq!(probe.call_count_for("W:/n"), 1);
        assert_eq!(unreachable, strings(&["W:/n"]));
    }

    #[test]
    fn partition_probes_unknown_folders_concurrently() {
        use gb_core::ports::folder_probe::FolderProbe;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::{Duration, Instant};

        /// Each probe waits until every other probe is also in flight, so the
        /// test passes only if the probes overlap. The deadline turns a
        /// serial implementation into a failure instead of a hang.
        struct RendezvousProbe {
            expected: usize,
            in_flight: AtomicUsize,
        }
        impl FolderProbe for RendezvousProbe {
            fn is_reachable(&self, _folder: &str) -> bool {
                self.in_flight.fetch_add(1, Ordering::SeqCst);
                let deadline = Instant::now() + Duration::from_secs(10);
                while self.in_flight.load(Ordering::SeqCst) < self.expected {
                    if Instant::now() > deadline {
                        return false;
                    }
                    std::thread::yield_now();
                }
                true
            }
        }

        let folders = strings(&["W:/a", "W:/b", "W:/c"]);
        let registry = registry_over(Arc::new(RendezvousProbe {
            expected: folders.len(),
            in_flight: AtomicUsize::new(0),
        }));

        let (reachable, unreachable) = partition_by_reachability(&folders, &registry);

        assert_eq!(reachable, folders);
        assert!(unreachable.is_empty());
    }

    #[test]
    fn partition_does_not_wait_for_a_recheck_in_flight_on_a_known_down_folder() {
        use gb_core::ports::folder_probe::FolderProbe;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Condvar, Mutex};
        use std::time::{Duration, Instant};

        /// The first probe answers "down" at once; later probes block until
        /// released, like a recheck stuck on a dead share.
        struct BlockAfterFirstProbe {
            entered: AtomicUsize,
            released: Mutex<bool>,
            gate: Condvar,
        }
        impl FolderProbe for BlockAfterFirstProbe {
            fn is_reachable(&self, folder: &str) -> bool {
                if folder != "W:/down" {
                    return true;
                }
                if self.entered.fetch_add(1, Ordering::SeqCst) == 0 {
                    return false;
                }
                let mut released = self.released.lock().unwrap();
                while !*released {
                    released = self.gate.wait(released).unwrap();
                }
                false
            }
        }

        let probe = Arc::new(BlockAfterFirstProbe {
            entered: AtomicUsize::new(0),
            released: Mutex::new(false),
            gate: Condvar::new(),
        });
        let registry = registry_over(probe.clone());
        assert!(!registry.is_reachable("W:/down"));

        let rechecker = {
            let registry = registry.clone();
            std::thread::spawn(move || registry.refresh("W:/down"))
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        while probe.entered.load(Ordering::SeqCst) < 2 {
            assert!(Instant::now() < deadline, "the recheck never started");
            std::thread::yield_now();
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let folders = strings(&["C:/a", "W:/down"]);
        {
            let registry = registry.clone();
            std::thread::spawn(move || {
                tx.send(partition_by_reachability(&folders, &registry))
                    .unwrap();
            });
        }
        let result = rx.recv_timeout(Duration::from_secs(10));

        // Let the recheck finish whatever the outcome, so no thread is left stuck.
        *probe.released.lock().unwrap() = true;
        probe.gate.notify_all();
        rechecker.join().unwrap();

        let (reachable, unreachable) = result.expect("partition waited for the in-flight probe");
        assert_eq!(reachable, strings(&["C:/a"]));
        assert_eq!(unreachable, strings(&["W:/down"]));
    }

    #[test]
    fn scan_reachable_folders_with_everything_unreachable_scans_nothing() {
        use crate::db::test_support::init_temp_db;
        use gb_core::testing::fake_folder_probe::FakeFolderProbe;

        let (_d, db) = init_temp_db();
        let thumbs = tempfile::tempdir().unwrap();
        let registry = registry_over(Arc::new(FakeFolderProbe::with_unreachable(&[
            "W:/a", "W:/b",
        ])));

        let summary =
            scan_reachable_folders(&strings(&["W:/a", "W:/b"]), &registry, &db, thumbs.path())
                .unwrap();

        assert_eq!(summary.scanned, 0);
        assert_eq!(summary.went_offline, 0);
        assert_eq!(summary.unreachable_folders, strings(&["W:/a", "W:/b"]));
    }

    #[test]
    fn scan_reachable_folders_does_not_mark_videos_under_a_skipped_folder_offline() {
        use crate::db::test_support::init_temp_db;
        use gb_core::testing::fake_folder_probe::FakeFolderProbe;

        let (_d, db) = init_temp_db();
        let thumbs = tempfile::tempdir().unwrap();
        let down = tempfile::tempdir().unwrap();
        let up = tempfile::tempdir().unwrap();
        std::fs::write(down.path().join("a.mp4"), b"down content").unwrap();
        std::fs::write(up.path().join("b.mp4"), b"up content").unwrap();
        let down_path = down.path().to_string_lossy().to_string();
        let up_path = up.path().to_string_lossy().to_string();
        let folders = vec![down_path.clone(), up_path.clone()];
        scan::scan_folders(&folders, &db, thumbs.path(), &crate::scan::AssumeReachable).unwrap();

        // The share looks empty, as a dead mount can.
        std::fs::remove_file(down.path().join("a.mp4")).unwrap();
        let registry = registry_over(Arc::new(FakeFolderProbe::with_unreachable(&[
            down_path.as_str()
        ])));

        let summary = scan_reachable_folders(&folders, &registry, &db, thumbs.path()).unwrap();

        assert_eq!(summary.unreachable_folders, vec![down_path.clone()]);
        assert_eq!(summary.scanned, 1);
        assert_eq!(summary.went_offline, 0);
        let online = queries::list_online_videos_under(&db.read_pool, &down_path).unwrap();
        assert_eq!(online.len(), 1);
    }
}
