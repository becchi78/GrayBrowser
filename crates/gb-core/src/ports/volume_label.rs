//! Volume-label lookup port: finds the drive a volume label currently belongs
//! to, so a `::LABEL\...` path from a `.wb` can be turned back into a
//! drive-letter path. The trait is OS-independent; the real implementation
//! (in `src-tauri::adapters::volume_label`) enumerates the connected drives.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelResolution {
    /// Exactly one connected drive carries the label; holds its root (`E:\`).
    Resolved(String),
    /// No connected drive carries the label.
    NotConnected,
    /// Several connected drives carry the label, so none can be picked safely.
    Ambiguous,
}

pub trait VolumeLabelResolver: Send + Sync {
    /// Compares `label` with the drives' labels case-insensitively, as Windows
    /// does.
    fn resolve(&self, label: &str) -> LabelResolution;
}
