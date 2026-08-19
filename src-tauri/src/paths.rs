//! Thin adapter around `gb_core::paths::resolve_app_dir`: the only
//! OS-touching part is reading the current executable's path.

use std::path::PathBuf;

pub fn app_data_dir() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    gb_core::paths::resolve_app_dir(&exe)
        .ok_or_else(|| anyhow::anyhow!("executable path {:?} has no parent directory", exe))
}
