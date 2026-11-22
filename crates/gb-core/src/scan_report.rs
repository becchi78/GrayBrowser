//! The scan diagnostics report a developer reads to see where a scan spent its
//! time and which files failed. Collecting the numbers is the scan's job; this
//! module holds the shapes, the arithmetic on them, and the text.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::diagnostics::{mask_paths, MaskRule};

/// How many files of one kind a report lists. A folder of unreadable files
/// (a dropped share) could otherwise put tens of thousands of paths in memory
/// and on the screen; the rest are only counted.
pub const MAX_LISTED_FILES: usize = 200;

/// Wall-clock time of the three phases of a manual scan, summed over every
/// folder scanned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StageTimings {
    /// Walking the folders and reading each file's metadata.
    pub enumeration: Duration,
    /// Name validation, hashing and the database write for each video file.
    pub register: Duration,
    /// Deciding which known videos have disappeared.
    pub reconcile: Duration,
}

impl StageTimings {
    pub fn total(&self) -> Duration {
        self.enumeration + self.register + self.reconcile
    }
}

/// `part` as a percentage of `total`; 0 when `total` is zero so an empty scan
/// does not print NaN.
pub fn percent_of(part: Duration, total: Duration) -> f64 {
    if total.is_zero() {
        0.0
    } else {
        part.as_secs_f64() / total.as_secs_f64() * 100.0
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanCounts {
    pub scanned: u32,
    pub registered: u32,
    pub reconciled: u32,
    pub unchanged: u32,
    pub skipped: u32,
    pub went_offline: u32,
    pub reactivated: u32,
    pub collisions: u32,
}

/// A list that keeps the first `MAX_LISTED_FILES` entries and counts the rest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CappedList {
    pub items: Vec<String>,
    pub overflow: u32,
}

impl CappedList {
    pub fn push(&mut self, item: String) {
        if self.items.len() < MAX_LISTED_FILES {
            self.items.push(item);
        } else {
            self.overflow = self.overflow.saturating_add(1);
        }
    }

    pub fn total(&self) -> usize {
        self.items.len() + self.overflow as usize
    }
}

/// A file that could not be registered, with the reason in plain words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedFile {
    pub path: String,
    pub reason: String,
}

/// Count, failures and total time of one kind of background work.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkStats {
    pub count: u64,
    pub failures: u64,
    pub total: Duration,
}

/// Lock-free tally the worker threads add to, so measuring a job costs two
/// clock reads and three atomic adds and never a lock.
#[derive(Debug, Default)]
pub struct WorkCounter {
    count: AtomicU64,
    failures: AtomicU64,
    nanos: AtomicU64,
}

impl WorkCounter {
    pub const fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            failures: AtomicU64::new(0),
            nanos: AtomicU64::new(0),
        }
    }

    pub fn record(&self, elapsed: Duration, succeeded: bool) {
        self.count.fetch_add(1, Ordering::Relaxed);
        if !succeeded {
            self.failures.fetch_add(1, Ordering::Relaxed);
        }
        let nanos = u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX);
        self.nanos.fetch_add(nanos, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> WorkStats {
        WorkStats {
            count: self.count.load(Ordering::Relaxed),
            failures: self.failures.load(Ordering::Relaxed),
            total: Duration::from_nanos(self.nanos.load(Ordering::Relaxed)),
        }
    }

    pub fn reset(&self) {
        self.count.store(0, Ordering::Relaxed);
        self.failures.store(0, Ordering::Relaxed);
        self.nanos.store(0, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    pub folders: u32,
    pub counts: ScanCounts,
    pub timings: StageTimings,
    /// Files the scan could not open or read; they leave no other trace.
    pub unreadable: CappedList,
    pub thumbnails: WorkStats,
    pub metadata: WorkStats,
    /// Files kept out of the catalog (`skipped_files`), with the stored reason.
    pub skipped: Vec<FailedFile>,
    /// Videos whose thumbnail or metadata generation ran out of retries.
    pub thumbnail_failures: Vec<String>,
    pub metadata_failures: Vec<String>,
}

pub const UNREADABLE_REASON: &str = "could not be opened or read during the scan";

/// `1.234 s` from one second up, whole milliseconds below.
pub fn format_duration(d: Duration) -> String {
    if d >= Duration::from_secs(1) {
        format!("{:.3} s", d.as_secs_f64())
    } else {
        format!("{} ms", d.as_millis())
    }
}

fn stage_line(label: &str, part: Duration, total: Duration) -> String {
    format!(
        "  {label}: {} ({:.1}%)",
        format_duration(part),
        percent_of(part, total)
    )
}

fn work_line(label: &str, w: &WorkStats) -> String {
    let average = u32::try_from(w.count)
        .ok()
        .filter(|&n| n > 0)
        .map(|n| format!(", average {}", format_duration(w.total / n)))
        .unwrap_or_default();
    format!(
        "{label}: {} processed, {} failed, {} in total{average}",
        w.count,
        w.failures,
        format_duration(w.total)
    )
}

fn push_list(out: &mut Vec<String>, title: &str, total: usize, lines: Vec<String>) {
    out.push(format!("{title}: {total}"));
    let shown = lines.len();
    out.extend(lines.into_iter().map(|l| format!("  {l}")));
    if total > shown {
        out.push(format!("  ... and {} more", total - shown));
    }
}

fn capped<T>(items: impl IntoIterator<Item = T>) -> Vec<T> {
    items.into_iter().take(MAX_LISTED_FILES).collect()
}

/// Plain text for pasting into a bug report. `rules` is applied to the whole
/// text, so a path in any list cannot leak the account name.
pub fn format_report(report: &ScanReport, rules: &[MaskRule]) -> String {
    let t = &report.timings;
    let total = t.total();
    let c = &report.counts;
    let mut out = vec![
        "GrayBrowser scan report".to_string(),
        format!("Folders scanned: {}", report.folders),
        format!(
            "Files: scanned={} registered={} reconciled={} unchanged={} skipped={} \
             went_offline={} reactivated={} collisions={}",
            c.scanned,
            c.registered,
            c.reconciled,
            c.unchanged,
            c.skipped,
            c.went_offline,
            c.reactivated,
            c.collisions
        ),
        "Stage timings:".to_string(),
        stage_line("Enumeration", t.enumeration, total),
        stage_line("Verify, hash and register", t.register, total),
        stage_line("Missing-video check", t.reconcile, total),
        format!("  Total: {}", format_duration(total)),
        work_line("Thumbnails", &report.thumbnails),
        work_line("Metadata", &report.metadata),
    ];
    push_list(
        &mut out,
        "Unreadable files",
        report.unreadable.total(),
        report
            .unreadable
            .items
            .iter()
            .map(|p| format!("{p} - {UNREADABLE_REASON}"))
            .collect(),
    );
    push_list(
        &mut out,
        "Unregistered files",
        report.skipped.len(),
        capped(
            report
                .skipped
                .iter()
                .map(|f| format!("{} - {}", f.path, f.reason)),
        ),
    );
    push_list(
        &mut out,
        "Thumbnail generation failures",
        report.thumbnail_failures.len(),
        capped(report.thumbnail_failures.iter().cloned()),
    );
    push_list(
        &mut out,
        "Metadata failures",
        report.metadata_failures.len(),
        capped(report.metadata_failures.iter().cloned()),
    );
    mask_paths(&out.join("\n"), rules)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn total_is_the_sum_of_the_stages() {
        let t = StageTimings {
            enumeration: ms(100),
            register: ms(700),
            reconcile: ms(200),
        };
        assert_eq!(t.total(), ms(1000));
        assert_eq!(StageTimings::default().total(), Duration::ZERO);
    }

    #[test]
    fn percentages_follow_the_total_and_survive_zero() {
        assert_eq!(percent_of(ms(250), ms(1000)), 25.0);
        assert_eq!(percent_of(ms(1000), ms(1000)), 100.0);
        assert_eq!(percent_of(ms(5), Duration::ZERO), 0.0);
        assert_eq!(percent_of(Duration::ZERO, Duration::ZERO), 0.0);
    }

    #[test]
    fn a_capped_list_keeps_the_first_entries_and_counts_the_rest() {
        let mut list = CappedList::default();
        for i in 0..MAX_LISTED_FILES + 7 {
            list.push(format!("f{i}"));
        }
        assert_eq!(list.items.len(), MAX_LISTED_FILES);
        assert_eq!(list.items[0], "f0");
        assert_eq!(list.overflow, 7);
        assert_eq!(list.total(), MAX_LISTED_FILES + 7);
    }

    #[test]
    fn a_list_at_the_cap_has_no_overflow() {
        let mut list = CappedList::default();
        for i in 0..MAX_LISTED_FILES {
            list.push(format!("f{i}"));
        }
        assert_eq!(list.overflow, 0);
        list.push("one more".into());
        assert_eq!(list.overflow, 1);
    }

    #[test]
    fn the_counter_sums_counts_failures_and_time_and_resets() {
        let c = WorkCounter::new();
        c.record(ms(10), true);
        c.record(ms(30), false);
        let s = c.snapshot();
        assert_eq!((s.count, s.failures, s.total), (2, 1, ms(40)));
        c.reset();
        assert_eq!(c.snapshot(), WorkStats::default());
    }

    #[test]
    fn the_counter_is_correct_across_threads() {
        let c = WorkCounter::new();
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    for _ in 0..250 {
                        c.record(Duration::from_micros(1), true);
                    }
                });
            }
        });
        let s = c.snapshot();
        assert_eq!(s.count, 1000);
        assert_eq!(s.total, Duration::from_micros(1000));
    }

    #[test]
    fn durations_switch_unit_at_one_second() {
        assert_eq!(format_duration(ms(999)), "999 ms");
        assert_eq!(format_duration(ms(1000)), "1.000 s");
        assert_eq!(format_duration(ms(1234)), "1.234 s");
        assert_eq!(format_duration(Duration::ZERO), "0 ms");
    }

    fn sample() -> ScanReport {
        let mut unreadable = CappedList::default();
        unreadable.push(r"C:\Users\alice\Videos\locked.mp4".into());
        ScanReport {
            folders: 2,
            counts: ScanCounts {
                scanned: 10,
                registered: 4,
                ..Default::default()
            },
            timings: StageTimings {
                enumeration: ms(250),
                register: ms(500),
                reconcile: ms(250),
            },
            unreadable,
            thumbnails: WorkStats {
                count: 4,
                failures: 1,
                total: ms(400),
            },
            metadata: WorkStats::default(),
            skipped: vec![FailedFile {
                path: r"C:\Users\alice\Videos\a\u{2460}.mp4".into(),
                reason: "machine_dependent_char".into(),
            }],
            thumbnail_failures: vec![r"C:\Users\alice\Videos\bad.mp4".into()],
            metadata_failures: Vec::new(),
        }
    }

    #[test]
    fn the_text_shows_stages_with_their_share_and_the_counts() {
        let text = format_report(&sample(), &[]);
        assert!(text.contains("Folders scanned: 2"));
        assert!(text.contains("scanned=10 registered=4"));
        assert!(text.contains("Enumeration: 250 ms (25.0%)"));
        assert!(text.contains("Verify, hash and register: 500 ms (50.0%)"));
        assert!(text.contains("Total: 1.000 s"));
        assert!(text.contains("Thumbnails: 4 processed, 1 failed, 400 ms in total, average 100 ms"));
        assert!(text.contains("Metadata: 0 processed, 0 failed, 0 ms in total\n"));
    }

    #[test]
    fn the_text_lists_failed_files_with_their_reason() {
        let text = format_report(&sample(), &[]);
        // The user name is masked, so the path is checked in its masked form.
        assert!(text.contains(&format!(
            "Unreadable files: 1\n  C:\\Users\\<user>\\Videos\\locked.mp4 - {UNREADABLE_REASON}"
        )));
        assert!(text.contains("machine_dependent_char"));
        assert!(text.contains("Thumbnail generation failures: 1"));
        assert!(text.contains("Metadata failures: 0"));
    }

    #[test]
    fn user_names_in_every_list_are_masked() {
        let text = format_report(&sample(), &[]);
        assert!(!text.contains("alice"));
        assert!(text.contains(r"C:\Users\<user>\Videos\locked.mp4"));
    }

    #[test]
    fn overflowing_lists_say_how_many_more() {
        let mut r = ScanReport::default();
        for i in 0..MAX_LISTED_FILES + 3 {
            r.unreadable.push(format!("C:\\v\\{i}.mp4"));
            r.thumbnail_failures.push(format!("C:\\t\\{i}.mp4"));
        }
        let text = format_report(&r, &[]);
        assert!(text.contains(&format!("Unreadable files: {}", MAX_LISTED_FILES + 3)));
        assert_eq!(text.matches("... and 3 more").count(), 2);
        assert!(!text.contains(&format!("C:\\v\\{MAX_LISTED_FILES}.mp4")));
    }

    #[test]
    fn an_empty_report_is_well_formed() {
        let text = format_report(&ScanReport::default(), &[]);
        assert!(text.contains("Enumeration: 0 ms (0.0%)"));
        assert!(text.contains("Unreadable files: 0"));
        assert!(!text.contains("NaN"));
    }
}
