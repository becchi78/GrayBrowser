//! Pure logic for `::LABEL\rest` paths found in a `.wb`: a volume label standing
//! where a drive letter would be. Looking up which drive carries a label is the
//! `VolumeLabelResolver` port's job; this module only parses and rewrites paths.

use std::collections::HashMap;

const SEPARATORS: [char; 2] = ['\\', '/'];

/// Splits `::LABEL\rest` into `("LABEL", "\rest")`. The rest keeps its leading
/// separator and is empty for a bare `::LABEL`. `None` when the path does not
/// start with `::` or the label is empty.
pub fn split_volume_label_path(path: &str) -> Option<(&str, &str)> {
    let body = path.strip_prefix("::")?;
    let end = body.find(SEPARATORS).unwrap_or(body.len());
    let (label, rest) = body.split_at(end);
    if label.is_empty() {
        return None;
    }
    Some((label, rest))
}

/// Puts `rest` (as returned by `split_volume_label_path`) under `root`
/// (`E:\` or `E:`), giving `E:\rest`.
pub fn apply_drive_root(rest: &str, root: &str) -> String {
    let drive = root.trim_end_matches(SEPARATORS);
    if rest.is_empty() {
        return format!("{drive}\\");
    }
    format!("{drive}{rest}")
}

/// Key under which labels are compared: Windows treats labels
/// case-insensitively.
pub fn label_key(label: &str) -> String {
    label.to_lowercase()
}

/// The distinct labels among `paths`, in first-seen order and first-seen
/// spelling, each with how many paths use it. Paths without a label are
/// ignored.
pub fn collect_labels<'a>(paths: impl Iterator<Item = &'a str>) -> Vec<(String, u32)> {
    let mut labels: Vec<(String, u32)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for path in paths {
        let Some((label, _)) = split_volume_label_path(path) else {
            continue;
        };
        match index.get(&label_key(label)) {
            Some(&i) => labels[i].1 += 1,
            None => {
                index.insert(label_key(label), labels.len());
                labels.push((label.to_string(), 1));
            }
        }
    }
    labels
}

/// `path` with its `::LABEL` replaced by the drive root found for that label in
/// `roots` (keyed by `label_key`). `None` when the path has no label or its
/// label has no root.
pub fn rewrite_path(path: &str, roots: &HashMap<String, String>) -> Option<String> {
    let (label, rest) = split_volume_label_path(path)?;
    let root = roots.get(&label_key(label))?;
    Some(apply_drive_root(rest, root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_label_and_rest() {
        assert_eq!(
            split_volume_label_path(r"::ESD-USB\動画\a.avi"),
            Some(("ESD-USB", r"\動画\a.avi"))
        );
        assert_eq!(
            split_volume_label_path("::ESD-USB/a.avi"),
            Some(("ESD-USB", "/a.avi"))
        );
    }

    #[test]
    fn a_label_without_a_separator_has_an_empty_rest() {
        assert_eq!(split_volume_label_path("::ESD-USB"), Some(("ESD-USB", "")));
    }

    #[test]
    fn labels_may_contain_spaces() {
        assert_eq!(
            split_volume_label_path(r"::MY DISK\a.avi"),
            Some(("MY DISK", r"\a.avi"))
        );
    }

    #[test]
    fn rejects_empty_labels_and_other_shapes() {
        assert_eq!(split_volume_label_path("::"), None);
        assert_eq!(split_volume_label_path(r"::\a.avi"), None);
        assert_eq!(split_volume_label_path(r"E:\a.avi"), None);
        assert_eq!(split_volume_label_path(r"\nas\share\a.avi"), None);
        assert_eq!(split_volume_label_path(r":ESD\a.avi"), None);
        assert_eq!(split_volume_label_path(""), None);
    }

    #[test]
    fn applies_a_drive_root() {
        assert_eq!(apply_drive_root(r"\a\b.avi", r"E:\"), r"E:\a\b.avi");
        assert_eq!(apply_drive_root(r"\a\b.avi", "E:"), r"E:\a\b.avi");
        assert_eq!(apply_drive_root("/a.avi", r"E:\"), "E:/a.avi");
        assert_eq!(apply_drive_root("", r"E:\"), r"E:\");
    }

    #[test]
    fn collects_distinct_labels_case_insensitively_with_counts() {
        let paths = [
            r"::ESD-USB\a.avi",
            r"D:\b.avi",
            r"::esd-usb\c.avi",
            r"::OTHER\d.avi",
            "::",
            r"::ESD-USB\e.avi",
        ];
        assert_eq!(
            collect_labels(paths.into_iter()),
            vec![("ESD-USB".to_string(), 3), ("OTHER".to_string(), 1)]
        );
        assert!(collect_labels([r"D:\a.avi"].into_iter()).is_empty());
    }

    #[test]
    fn rewrites_only_paths_whose_label_has_a_root() {
        let roots = HashMap::from([(label_key("ESD-USB"), r"E:\".to_string())]);
        assert_eq!(
            rewrite_path(r"::esd-usb\a.avi", &roots),
            Some(r"E:\a.avi".to_string())
        );
        assert_eq!(rewrite_path(r"::OTHER\a.avi", &roots), None);
        assert_eq!(rewrite_path(r"D:\a.avi", &roots), None);
    }
}
