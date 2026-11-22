//! Thin adapter around `gb_core::paths::resolve_app_dir`: the only
//! OS-touching parts are reading the current executable's path and the
//! process's command line / environment for a data folder override.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use gb_core::paths::{find_data_dir_arg, resolve_data_dir_override, DataDirError};

/// Environment variable naming the data folder; `--data-dir` wins over it.
pub const DATA_DIR_ENV: &str = "GB_DATA_DIR";

/// Read once at startup. Unset (never initialised) behaves like `Some(None)`:
/// no override, so code and tests that never call `init_data_dir_override`
/// get the default folder.
static DATA_DIR_OVERRIDE: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Validates an override from explicit inputs, so tests do not depend on the
/// process's real arguments or environment. `is_file` is injected for the
/// same reason.
pub fn resolve_override_from(
    args: &[String],
    env: Option<&str>,
    is_file: impl Fn(&Path) -> bool,
) -> Result<Option<PathBuf>, DataDirError> {
    let arg = find_data_dir_arg(args)?;
    let resolved = resolve_data_dir_override(arg.as_deref(), env)?;
    match resolved {
        Some(path) if is_file(&path) => Err(DataDirError::IsFile),
        other => Ok(other),
    }
}

/// Reads the real command line and environment once and keeps the result for
/// `app_data_dir`. A repeated call keeps the first result. An invalid request
/// is returned as an error and nothing is stored, so the caller can stop
/// instead of continuing on the real data folder.
pub fn init_data_dir_override() -> Result<(), DataDirError> {
    let args: Vec<String> = std::env::args().collect();
    let env = std::env::var(DATA_DIR_ENV).ok();
    let resolved = resolve_override_from(&args, env.as_deref(), Path::is_file)?;
    let _ = DATA_DIR_OVERRIDE.set(resolved);
    Ok(())
}

/// Whether a data folder override is in force.
pub fn data_dir_overridden() -> bool {
    DATA_DIR_OVERRIDE.get().is_some_and(Option::is_some)
}

pub fn app_data_dir() -> anyhow::Result<PathBuf> {
    if let Some(Some(dir)) = DATA_DIR_OVERRIDE.get() {
        return Ok(dir.clone());
    }
    let exe = std::env::current_exe()?;
    gb_core::paths::resolve_app_dir(&exe)
        .ok_or_else(|| anyhow::anyhow!("executable path {:?} has no parent directory", exe))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_arg_and_no_env_means_no_override() {
        assert_eq!(
            resolve_override_from(&strs(&["app.exe"]), None, |_| false),
            Ok(None)
        );
    }

    #[test]
    fn the_argument_beats_the_environment() {
        let got = resolve_override_from(
            &strs(&["app.exe", "--data-dir", r"D:\arg"]),
            Some(r"D:\env"),
            |_| false,
        );
        assert_eq!(got, Ok(Some(PathBuf::from(r"D:\arg"))));
    }

    #[test]
    fn the_environment_is_used_without_an_argument() {
        let got = resolve_override_from(&strs(&["app.exe"]), Some(r"D:\env"), |_| false);
        assert_eq!(got, Ok(Some(PathBuf::from(r"D:\env"))));
    }

    #[test]
    fn an_invalid_request_is_an_error_not_a_fallback() {
        assert_eq!(
            resolve_override_from(&strs(&["app.exe", "--data-dir=rel"]), None, |_| false),
            Err(DataDirError::NotAbsolute)
        );
        assert_eq!(
            resolve_override_from(&strs(&["app.exe", "--data-dir"]), None, |_| false),
            Err(DataDirError::MissingValue)
        );
    }

    #[test]
    fn an_existing_file_is_refused() {
        let got = resolve_override_from(&strs(&["--data-dir", r"D:\a.txt"]), None, |_| true);
        assert_eq!(got, Err(DataDirError::IsFile));
    }

    #[test]
    fn a_real_file_is_refused_with_the_real_check() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"x").unwrap();
        let got = resolve_override_from(
            &strs(&["--data-dir", &file.display().to_string()]),
            None,
            Path::is_file,
        );
        assert_eq!(got, Err(DataDirError::IsFile));
        // A folder that does not exist yet is fine: startup creates it.
        let missing = dir.path().join("new");
        let got = resolve_override_from(
            &strs(&["--data-dir", &missing.display().to_string()]),
            None,
            Path::is_file,
        );
        assert_eq!(got, Ok(Some(missing)));
    }
}
