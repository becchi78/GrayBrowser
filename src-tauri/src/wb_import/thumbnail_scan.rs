//! Enumerates filenames in the legacy WhiteBrowser thumbnail folder, for
//! matching against `.wb` `movie.hash` via
//! `gb_core::wb_import::match_thumbnail_files`.
//!
//! This is OS-dependent (`std::fs::read_dir`) but deliberately not put
//! behind a `ports`/adapter trait: it's a single, direct wrapper around one
//! stdlib call with no branching worth faking independently -- callers can
//! exercise the real thing against a `tempfile`-style directory in a unit
//! test just as easily as they could a fake, so the extra trait/adapter/fake
//! layer would add indirection without adding testability.

use std::path::Path;

/// Returns every filename (no path component) found directly inside
/// `folder`. Subdirectories are listed by their own name but not recursed
/// into -- legacy thumbnail folders are always flat. An entry whose name
/// isn't valid UTF-8 is skipped and logged rather
/// than failing the whole scan -- one oddly-named file must not block
/// migrating the rest of the library.
pub fn list_filenames(folder: &Path) -> std::io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        match entry.file_name().into_string() {
            Ok(name) => names.push(name),
            Err(os_name) => {
                log::warn!("skipping non-UTF-8 filename in legacy thumbnail folder: {os_name:?}");
            }
        }
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn unique_temp_dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "gb-thumbnail-scan-test-{}-{}-{}",
            std::process::id(),
            label,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn list_filenames_returns_every_file_in_the_folder() {
        let dir = unique_temp_dir("basic");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.jpg"), b"x").unwrap();
        fs::write(dir.join("b.jpg"), b"x").unwrap();

        let mut names = list_filenames(&dir).unwrap();
        names.sort();
        assert_eq!(names, vec!["a.jpg".to_string(), "b.jpg".to_string()]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_filenames_returns_empty_vec_for_an_empty_folder() {
        let dir = unique_temp_dir("empty");
        fs::create_dir_all(&dir).unwrap();

        let names = list_filenames(&dir).unwrap();
        assert!(names.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_filenames_errors_for_a_nonexistent_folder() {
        let dir = unique_temp_dir("does-not-exist");
        assert!(list_filenames(&dir).is_err());
    }

    #[test]
    fn list_filenames_lists_subdirectory_names_without_recursing() {
        let dir = unique_temp_dir("with-subdir");
        fs::create_dir_all(dir.join("subdir")).unwrap();
        fs::write(dir.join("subdir").join("nested.jpg"), b"x").unwrap();
        fs::write(dir.join("top.jpg"), b"x").unwrap();

        let mut names = list_filenames(&dir).unwrap();
        names.sort();
        assert_eq!(names, vec!["subdir".to_string(), "top.jpg".to_string()]);

        let _ = fs::remove_dir_all(&dir);
    }
}
