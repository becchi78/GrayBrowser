//! Which network shares / drives the rows live on can be reached, and the
//! rewriting of `::LABEL\...` paths onto the drive that carries that volume
//! label.

use std::collections::HashMap;
use std::thread;

use gb_core::paths::extract_drive_root;
use gb_core::ports::folder_probe::FolderProbe;
use gb_core::ports::volume_label::{LabelResolution, VolumeLabelResolver};
use gb_core::volume_label::{collect_labels, label_key, rewrite_path};
use gb_core::wb_import_log::{VolumeLabelLog, VolumeLabelResult};

/// Whether each network share / drive that the rows live on can be reached,
/// decided once per share for one import. Every first access to a stopped NAS
/// share blocks for about 20 seconds, so without this a library with many rows
/// there would pay that wait again and again.
///
/// Deliberately not `ReachabilityRegistry`: that publishes its answers to the
/// UI as "unreachable watch folders", which an import's one-off check of
/// arbitrary share roots must not touch.
pub(super) struct ShareReachability {
    answers: HashMap<String, bool>,
}

impl ShareReachability {
    /// Probes every distinct share root among `paths` exactly once, all at the
    /// same time, so any number of stopped shares costs one wait, not one each.
    pub(super) fn probe<'a, P: FolderProbe>(
        probe: &P,
        paths: impl Iterator<Item = &'a str>,
    ) -> Self {
        let mut roots: Vec<String> = paths.filter_map(extract_drive_root).collect();
        roots.sort();
        roots.dedup();
        let results: Vec<bool> = thread::scope(|scope| {
            let handles: Vec<_> = roots
                .iter()
                .map(|root| scope.spawn(move || probe.is_reachable(root)))
                .collect();
            handles
                .into_iter()
                .zip(&roots)
                .map(|(h, root)| {
                    // A panicking probe must not take the import thread down
                    // with it (the UI would never hear the import ended);
                    // fall back to checking that share's files one by one.
                    h.join().unwrap_or_else(|_| {
                        log::warn!(
                            "wb import: probing share {root} panicked; checking its files as usual"
                        );
                        true
                    })
                })
                .collect()
        });
        Self {
            answers: roots.into_iter().zip(results).collect(),
        }
    }

    /// A path that is not drive-letter/UNC shaped (e.g. relative) cannot be
    /// attributed to a share and is treated as reachable, i.e. checked as usual.
    pub(super) fn is_reachable_path(&self, path: &str) -> bool {
        extract_drive_root(path)
            .and_then(|root| self.answers.get(&root).copied())
            .unwrap_or(true)
    }

    /// Share roots found unreachable, in a stable order.
    pub(super) fn unreachable_roots(&self) -> Vec<&str> {
        let mut down: Vec<&str> = self
            .answers
            .iter()
            .filter(|(_, ok)| !**ok)
            .map(|(root, _)| root.as_str())
            .collect();
        down.sort_unstable();
        down
    }
}

/// What `resolve_volume_labels` did to the rows.
pub(super) struct VolumeLabelReport {
    pub(super) logs: Vec<VolumeLabelLog>,
    pub(super) resolved_rows: u32,
    pub(super) unresolved_rows: u32,
    pub(super) unresolved_labels: Vec<String>,
}

/// Rewrites `::LABEL\...` row paths onto the connected drive of that label,
/// asking `resolver` once per distinct label. A row whose label has no single
/// drive keeps its `::` path, so it is registered offline under the name the
/// `.wb` gave it rather than under a guessed drive.
pub(super) fn resolve_volume_labels<V: VolumeLabelResolver>(
    rows: &mut [gb_core::ports::wb_source::WbMovieRow],
    resolver: &V,
) -> VolumeLabelReport {
    let labels = collect_labels(rows.iter().map(|r| r.movie_path.as_str()));
    let mut roots: HashMap<String, String> = HashMap::new();
    let mut report = VolumeLabelReport {
        logs: Vec::with_capacity(labels.len()),
        resolved_rows: 0,
        unresolved_rows: 0,
        unresolved_labels: Vec::new(),
    };
    for (label, row_count) in labels {
        let result = match resolver.resolve(&label) {
            LabelResolution::Resolved(root) => {
                roots.insert(label_key(&label), root.clone());
                report.resolved_rows += row_count;
                VolumeLabelResult::Resolved(root)
            }
            other => {
                report.unresolved_rows += row_count;
                report.unresolved_labels.push(label.clone());
                log::warn!("wb import: volume label {label:?} not resolved: {other:?}");
                match other {
                    LabelResolution::Ambiguous => VolumeLabelResult::Ambiguous,
                    _ => VolumeLabelResult::NotConnected,
                }
            }
        };
        report.logs.push(VolumeLabelLog {
            label,
            rows: row_count,
            result,
        });
    }
    if !roots.is_empty() {
        for row in rows.iter_mut() {
            if let Some(path) = rewrite_path(&row.movie_path, &roots) {
                row.movie_path = path;
            }
        }
    }
    report
}

/// Import-log line for a share whose rows were registered without touching it.
pub(super) fn format_unreachable_share(root: &str) -> String {
    format!("到達できない共有のため、この共有上の行のファイル確認を飛ばしました: {root}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gb_core::testing::fake_folder_probe::FakeFolderProbe;

    #[test]
    fn a_path_that_is_not_on_a_share_is_reachable_without_a_probe() {
        let probe = FakeFolderProbe::default();
        let shares = ShareReachability::probe(
            &probe,
            ["relative.mp4", r" rel\x.mp4", r"\server"].into_iter(),
        );

        assert!(shares.is_reachable_path("relative.mp4"));
        assert!(shares.is_reachable_path(r" rel\x.mp4"));
        assert!(shares.is_reachable_path(r"\server"));
        assert!(probe.calls().is_empty());
    }
}
