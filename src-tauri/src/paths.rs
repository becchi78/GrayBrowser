//! Thin adapter around `gb_core::paths::resolve_app_dir`: the only
//! OS-touching part is reading the current executable's path.

use std::path::{Path, PathBuf};

pub fn app_data_dir() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    gb_core::paths::resolve_app_dir(&exe)
        .ok_or_else(|| anyhow::anyhow!("executable path {:?} has no parent directory", exe))
}

/// One-time migration for installs made before `resolve_app_dir` stopped
/// double-nesting: earlier builds resolved the production NSIS installer's
/// `%LOCALAPPDATA%\GrayBrowser\GrayBrowser.exe` layout to a second
/// `GrayBrowser\GrayBrowser\` folder, so real user data (`app.db`,
/// `thumbnails/`, `logs/`) may still be sitting one level too deep. Moves
/// everything from `app_dir/GrayBrowser/` up into `app_dir` itself.
///
/// Only runs when `app_dir` has no `app.db` yet but the old nested folder
/// does -- i.e. exactly the "never migrated" case. A fresh install (neither
/// location has `app.db`) or an already-migrated install (the new location
/// already has it) both no-op.
///
/// Data-loss safety is the overriding design constraint here: every
/// non-database entry (`thumbnails/`, `logs/`, anything else found) is moved
/// best-effort -- a failure is logged and skipped rather than aborting the
/// whole migration, since that data is regenerable/disposable. The database
/// files themselves are the one part of this that must not silently fail:
/// if renaming `app.db` (or its `-wal`/`-shm` companions) errors out, this
/// function propagates the error and the caller aborts startup, rather than
/// risking `db::init` silently creating a fresh empty database next to
/// real user data that failed to move.
///
/// Callers must call this *after* anything that might pre-create a same-
/// named directory at the new location (notably `logging::init`, which
/// synchronously creates `app_dir/logs/`) -- and this function is written to
/// tolerate that: a same-named destination *directory* (as opposed to a
/// file) is merged into rather than skipped, so `logs/` still gets its
/// contents moved even though an empty `app_dir/logs/` already exists by
/// the time this runs.
pub fn migrate_legacy_nested_app_dir(app_dir: &Path) -> anyhow::Result<()> {
    let old_dir = app_dir.join("GrayBrowser");
    let old_db = old_dir.join("app.db");
    let new_db = app_dir.join("app.db");

    if new_db.exists() || !old_db.exists() {
        return Ok(());
    }

    log::info!(
        "migrating legacy double-nested app data from {} to {}",
        old_dir.display(),
        app_dir.display()
    );

    // Move every non-database entry first (thumbnails/, logs/, anything
    // else present), best-effort: this data is regenerable, so a failure
    // here must not block the far more important database move below.
    let mut all_moved = true;
    match std::fs::read_dir(&old_dir) {
        Ok(entries) => {
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(e) => {
                        log::warn!(
                            "failed to read a directory entry while migrating legacy app data: {e}"
                        );
                        all_moved = false;
                        continue;
                    }
                };
                let file_name = entry.file_name();
                if is_db_file_name(&file_name) {
                    continue;
                }
                let entry_path = entry.path();
                let dest = app_dir.join(&file_name);
                if dest.exists() {
                    // A directory at the destination doesn't necessarily
                    // mean this entry was already migrated -- it may just
                    // be an empty directory something else pre-created
                    // (e.g. `logging::init` creating `app_dir/logs/` ahead
                    // of this call) -- so merge into it entry-by-entry
                    // rather than skipping the whole thing. A same-named
                    // *file* at the destination, though, really does mean
                    // "already migrated", so that case is still skipped.
                    if dest.is_dir()
                        && entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
                        && !merge_directory_contents(&entry_path, &dest)
                    {
                        all_moved = false;
                    }
                    continue;
                }
                if let Err(e) = std::fs::rename(&entry_path, &dest) {
                    log::warn!("failed to migrate legacy app data entry {entry_path:?}: {e}");
                    all_moved = false;
                }
            }
        }
        Err(e) => {
            log::warn!("failed to read legacy app data directory {old_dir:?}: {e}");
            all_moved = false;
        }
    }

    // The database itself: must succeed, or startup is aborted rather than
    // risking a fresh empty database being created next to real data that
    // failed to move.
    std::fs::rename(&old_db, &new_db)?;
    for companion in ["app.db-wal", "app.db-shm"] {
        let old_companion = old_dir.join(companion);
        if old_companion.exists() {
            std::fs::rename(&old_companion, app_dir.join(companion))?;
        }
    }

    if all_moved {
        // Non-recursive: silently does nothing if the folder isn't actually
        // empty (e.g. some non-database entry failed to move above), which
        // doubles as a safety net even though `all_moved` should already
        // reflect that.
        let _ = std::fs::remove_dir(&old_dir);
    } else {
        log::warn!(
            "legacy app data folder {} was not fully migrated and was left in place",
            old_dir.display()
        );
    }

    Ok(())
}

/// Moves every entry directly inside `src_dir` into the already-existing
/// `dest_dir`, one `rename` at a time -- the fallback used by
/// `migrate_legacy_nested_app_dir` when the destination is a directory that
/// already exists (so the whole `src_dir` can't just be renamed into place
/// as a single unit). Deliberately shallow (one level): the only known case
/// this needs to handle is a flat directory of files (`logs/`'s log files),
/// not arbitrarily nested subtrees.
///
/// Same best-effort semantics as the caller: a same-named destination file
/// is treated as already migrated and skipped, any other failure is logged
/// via `log::warn!` and the rest of the entries are still attempted. Returns
/// whether every entry was accounted for (moved or already present) -- if
/// so, `src_dir` is removed (now empty); otherwise it's left in place for a
/// future run to retry, and removal is not attempted.
fn merge_directory_contents(src_dir: &Path, dest_dir: &Path) -> bool {
    let mut all_moved = true;
    match std::fs::read_dir(src_dir) {
        Ok(entries) => {
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(e) => {
                        log::warn!(
                            "failed to read a directory entry while merging legacy app data directory {src_dir:?}: {e}"
                        );
                        all_moved = false;
                        continue;
                    }
                };
                let dest = dest_dir.join(entry.file_name());
                if dest.exists() {
                    // Already migrated -- leave it alone rather than
                    // overwriting.
                    continue;
                }
                let entry_path = entry.path();
                if let Err(e) = std::fs::rename(&entry_path, &dest) {
                    log::warn!("failed to migrate legacy app data entry {entry_path:?}: {e}");
                    all_moved = false;
                }
            }
        }
        Err(e) => {
            log::warn!("failed to read legacy app data directory {src_dir:?}: {e}");
            all_moved = false;
        }
    }

    if all_moved {
        let _ = std::fs::remove_dir(src_dir);
    }

    all_moved
}

fn is_db_file_name(file_name: &std::ffi::OsStr) -> bool {
    file_name == "app.db" || file_name == "app.db-wal" || file_name == "app.db-shm"
}
