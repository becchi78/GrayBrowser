//! Whether a recorded file is verifiably gone, shared by the commands that
//! list or act on files missing from disk.

use std::path::Path;

use gb_core::ports::path_existence::PathExistence;
use gb_core::presence::{classify_presence, Presence};

use crate::reachability::ReachabilityRegistry;

/// Whether the file recorded at `file_path` is verifiably gone. A path under
/// an unreachable watch folder is `Unknown` without any file-system access:
/// stat-ing a dead share blocks for a long time and could not prove anything.
pub(crate) fn presence_of(
    file_path: &str,
    watch_folders: &[String],
    reachability: &ReachabilityRegistry,
    fs: &dyn PathExistence,
) -> Presence {
    if reachability.is_path_unreachable(file_path, watch_folders) {
        return classify_presence(true, false, false);
    }
    let path = Path::new(file_path);
    let file_exists = fs.exists(path);
    // Only a missing file needs the parent checked.
    let parent_exists = file_exists || path.parent().is_some_and(|parent| fs.exists(parent));
    classify_presence(false, parent_exists, file_exists)
}
