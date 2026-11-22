//! What the developer-mode scan report needs from the running app: the report
//! of the most recent manual scan (kept in memory only, so it is gone after a
//! restart) and the tallies of the thumbnail and metadata workers.

use std::sync::{Arc, Mutex};

use gb_core::scan_report::{ScanCounts, ScanReport, WorkCounter};

use super::ScanSummary;

/// Thumbnail and metadata generation run after the scan, on their own threads,
/// started from several places (a scan, startup, an import). A process-wide
/// tally is the one place they can all add to without every entry point
/// having to pass a handle down; the scan command resets it when a scan
/// starts so the report covers the work that scan triggered.
pub static THUMBNAIL_WORK: WorkCounter = WorkCounter::new();
pub static METADATA_WORK: WorkCounter = WorkCounter::new();

#[derive(Clone, Default)]
pub struct LastScanReport(Arc<Mutex<Option<ScanReport>>>);

impl LastScanReport {
    pub fn store(&self, report: ScanReport) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(report);
    }

    pub fn get(&self) -> Option<ScanReport> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// The part of the report known when the scan ends. The worker tallies and the
/// database-backed failure lists are added when the report is read, because
/// they keep changing after the scan returns.
pub fn report_from_summary(summary: &ScanSummary, folders_scanned: usize) -> ScanReport {
    ScanReport {
        folders: u32::try_from(folders_scanned).unwrap_or(u32::MAX),
        counts: ScanCounts {
            scanned: summary.scanned,
            registered: summary.registered,
            reconciled: summary.reconciled,
            unchanged: summary.unchanged,
            skipped: summary.skipped,
            went_offline: summary.went_offline,
            reactivated: summary.reactivated,
            collisions: summary.collisions,
        },
        timings: summary.timings,
        unreadable: summary.unreadable.clone(),
        ..ScanReport::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn nothing_is_stored_until_a_scan_has_run() {
        let last = LastScanReport::default();
        assert!(last.get().is_none());
        last.store(ScanReport {
            folders: 3,
            ..ScanReport::default()
        });
        assert_eq!(last.get().unwrap().folders, 3);
    }

    #[test]
    fn a_later_scan_replaces_the_stored_report() {
        let last = LastScanReport::default();
        let clone = last.clone();
        last.store(ScanReport {
            folders: 1,
            ..ScanReport::default()
        });
        clone.store(ScanReport {
            folders: 2,
            ..ScanReport::default()
        });
        assert_eq!(last.get().unwrap().folders, 2);
    }

    #[test]
    fn the_report_carries_the_summary_counts_and_timings() {
        let mut summary = ScanSummary {
            scanned: 5,
            registered: 2,
            went_offline: 1,
            ..ScanSummary::default()
        };
        summary.timings.register = Duration::from_millis(40);
        summary.unreadable.push("C:/x/locked.mp4".into());

        let report = report_from_summary(&summary, 2);

        assert_eq!(report.folders, 2);
        assert_eq!(report.counts.scanned, 5);
        assert_eq!(report.counts.registered, 2);
        assert_eq!(report.counts.went_offline, 1);
        assert_eq!(report.timings.register, Duration::from_millis(40));
        assert_eq!(report.unreadable.items, vec!["C:/x/locked.mp4".to_string()]);
    }
}
