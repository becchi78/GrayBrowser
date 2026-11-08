//! Per-row file state for the import: restoring a stored path whose `?`
//! stands for characters the old WhiteBrowser lost, and whether the file is
//! on disk (online, with its real quick_hash) or an offline placeholder.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use gb_core::path_restore::restore_path;
use gb_core::ports::dir_lister::DirLister;

use crate::adapters::long_path;

/// Resolved per-row scan-metadata used to build each row's `NewVideo`.
pub(super) struct ResolvedFileState {
    pub(super) status: &'static str,
    pub(super) quick_hash: String,
    pub(super) mtime: i64,
    pub(super) file_size: u64,
}

/// Determines whether a `.wb` row's `movie_path` exists on this machine
/// right now, and if so, computes its real quick_hash/mtime/file_size so an
/// imported row is immediately scan-consistent instead of waiting for a
/// future full scan to backfill it.
///
/// Every failure mode -- the path doesn't exist, its metadata can't be read,
/// its mtime can't be represented, or hashing it fails (locked/permission
/// error) -- converges on the same `"offline"` placeholder
/// (`quick_hash=""`, `mtime=0`, `file_size=0`). This is a deliberate design
/// choice, not merely "the error path was easiest": that placeholder is
/// inert for the drive-still-attached-at-the-same-path reconnection case,
/// and the moment a real scan/watch/poll rediscovers *this exact path*
/// again, `gb_core::reconcile::classify_discovered_file` sees
/// `mtime=0`/`file_size=0` mismatch the real values and classifies it
/// `NeedsRehash` -- so the placeholder self-heals into a real quick_hash
/// without any dedicated "wb-imported, needs backfill" bookkeeping. The
/// known limitation this accepts: quick_hash-based auto-follow to a *new*
/// path after a drive-letter change cannot work for a row still carrying
/// this placeholder, since it never had a real quick_hash to match against
/// in the first place.
///
/// `share_reachable == false` short-circuits to the same placeholder without
/// touching the filesystem: asking a stopped NAS share costs about 20 seconds
/// per first access.
pub(super) fn resolve_file_state(movie_path: &str, share_reachable: bool) -> ResolvedFileState {
    let offline = || ResolvedFileState {
        status: "offline",
        quick_hash: String::new(),
        mtime: 0,
        file_size: 0,
    };
    if !share_reachable {
        return offline();
    }

    let long_movie_path = long_path::to_long_path(Path::new(movie_path));
    let metadata = match fs::metadata(&long_movie_path) {
        Ok(m) => m,
        Err(_) => return offline(),
    };
    let file_size = metadata.len();
    let mtime = match crate::scan::mtime_from_metadata(&metadata) {
        Ok(secs) => secs,
        Err(e) => {
            log::warn!("wb import: {movie_path}: {e}; treating as offline placeholder");
            return offline();
        }
    };
    match crate::scan::hash_file(movie_path, file_size) {
        Ok(hash) => ResolvedFileState {
            status: "online",
            quick_hash: hash.to_string(),
            mtime,
            file_size,
        },
        Err(e) => {
            log::warn!(
                "wb import: {movie_path}: failed to compute quick_hash: {e}; \
                 treating as offline placeholder"
            );
            offline()
        }
    }
}

/// Restores a stored path containing `?` when -- and only when -- that path
/// does not exist as written. Rows that exist, or have no `?`, are left
/// exactly as before (no listing happens for them). Nothing is restored on an
/// unreachable share: neither the existence check nor the listing may touch it.
pub(super) fn restore_missing_path(
    movie_path: &str,
    share_reachable: bool,
    lister: &impl DirLister,
) -> Option<String> {
    if !share_reachable || !movie_path.contains('?') {
        return None;
    }
    if fs::metadata(long_path::to_long_path(Path::new(movie_path))).is_ok() {
        return None;
    }
    restore_path(movie_path, lister)
}

/// Remembers each folder's listing -- including "could not be listed" -- for
/// one import. Listing a stopped NAS share blocks for about 20 seconds, so
/// without this every row under that share would pay the wait again.
pub(super) struct CachingDirLister<'a, D: DirLister> {
    inner: &'a D,
    listings: Mutex<HashMap<String, Option<Vec<String>>>>,
}

impl<'a, D: DirLister> CachingDirLister<'a, D> {
    pub(super) fn new(inner: &'a D) -> Self {
        Self {
            inner,
            listings: Mutex::new(HashMap::new()),
        }
    }
}

impl<D: DirLister> DirLister for CachingDirLister<'_, D> {
    fn list_names(&self, dir: &str) -> Option<Vec<String>> {
        // The lock is held across the inner call on purpose: this runs on the
        // import thread only, so there is no contention to avoid, and it
        // keeps two callers from both paying for the same slow listing.
        let mut listings = self.listings.lock().unwrap();
        if let Some(cached) = listings.get(dir) {
            return cached.clone();
        }
        let names = self.inner.list_names(dir);
        listings.insert(dir.to_string(), names.clone());
        names
    }
}
