//! Publishing a video's freshly written slot files: renaming each scratch
//! file onto its final `{video_id}_{slot}.webp` name, all-or-nothing.

use std::path::{Path, PathBuf};

use super::paths::slot_path;

/// The rename that stopped [`publish_slots`]. The fields are kept apart so
/// each caller can word the failure in the message format it already reports.
#[derive(Debug)]
pub(crate) struct SlotPublishError {
    pub slot: usize,
    pub from: PathBuf,
    pub to: PathBuf,
    pub source: std::io::Error,
}

/// Renames `tmp_paths[i]` to slot `i`'s final path under `video_dir`, in
/// slot order. A video's thumbnails are all-or-nothing, so if any rename
/// fails, the finals this call already published are removed again and the
/// error is returned. Only this call's own files are removed, never a
/// pre-existing path that made a rename fail. Scratch files that were not
/// renamed yet are left where they are; removing them is the caller's job,
/// since only the caller knows which scratch names it used.
pub(crate) fn publish_slots(
    video_dir: &Path,
    video_id: &str,
    tmp_paths: &[PathBuf],
) -> Result<(), SlotPublishError> {
    let mut renamed_finals: Vec<PathBuf> = Vec::with_capacity(tmp_paths.len());
    for (i, tmp_path) in tmp_paths.iter().enumerate() {
        let final_path = slot_path(video_dir, video_id, i);
        if let Err(e) = std::fs::rename(tmp_path, &final_path) {
            for renamed in &renamed_finals {
                let _ = std::fs::remove_file(renamed);
            }
            return Err(SlotPublishError {
                slot: i,
                from: tmp_path.clone(),
                to: final_path,
                source: e,
            });
        }
        renamed_finals.push(final_path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thumbnail::worker::THUMBNAILS_PER_VIDEO;

    fn write_slot_tmps(dir: &Path, id: &str) -> Vec<PathBuf> {
        (0..THUMBNAILS_PER_VIDEO)
            .map(|i| {
                let tmp = dir.join(format!("{id}_{i}.webp.n-1.tmp"));
                std::fs::write(&tmp, format!("slot {i}")).unwrap();
                tmp
            })
            .collect()
    }

    /// The `io::Error` text a rename of a plain file onto `blocker` (an
    /// existing directory) produces on this machine, so the expectation
    /// doesn't hard-code an OS- and locale-dependent string.
    fn rename_onto_dir_error_text(blocker: &Path) -> String {
        let probe_dir = tempfile::tempdir().unwrap();
        let probe = probe_dir.path().join("probe.tmp");
        std::fs::write(&probe, b"probe").unwrap();
        std::fs::rename(&probe, blocker).unwrap_err().to_string()
    }

    #[test]
    fn moves_every_tmp_to_its_slot_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let tmps = write_slot_tmps(dir.path(), "vid");

        publish_slots(dir.path(), "vid", &tmps).unwrap();

        for (i, tmp) in tmps.iter().enumerate() {
            let slot = dir.path().join(format!("vid_{i}.webp"));
            assert_eq!(std::fs::read_to_string(&slot).unwrap(), format!("slot {i}"));
            assert!(!tmp.exists(), "tmp {i} must be gone");
        }
    }

    #[test]
    fn a_failure_removes_only_this_calls_finals() {
        let dir = tempfile::tempdir().unwrap();
        let tmps = write_slot_tmps(dir.path(), "vid");
        let blocker = dir.path().join("vid_3.webp");
        std::fs::create_dir(&blocker).unwrap();
        let io_text = rename_onto_dir_error_text(&blocker);

        let err = publish_slots(dir.path(), "vid", &tmps).unwrap_err();

        assert_eq!(err.slot, 3);
        assert_eq!(err.from, tmps[3]);
        assert_eq!(err.to, blocker);
        assert_eq!(err.source.to_string(), io_text);
        for i in [0, 1, 2] {
            assert!(
                !dir.path().join(format!("vid_{i}.webp")).exists(),
                "slot {i}"
            );
            assert!(!tmps[i].exists(), "tmp {i} was renamed away");
        }
        assert!(blocker.is_dir(), "a pre-existing path must be left alone");
        for i in [3, 4, 5] {
            assert!(
                tmps[i].exists(),
                "tmp {i} is left for the caller to clean up"
            );
        }
        for i in [4, 5] {
            assert!(
                !dir.path().join(format!("vid_{i}.webp")).exists(),
                "slot {i}"
            );
        }
    }

    #[test]
    fn an_empty_set_publishes_nothing() {
        let dir = tempfile::tempdir().unwrap();

        publish_slots(dir.path(), "vid", &[]).unwrap();

        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
