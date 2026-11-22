//! The diagnostics report a developer pastes into a bug report. Paths inside
//! it are masked so the user's Windows account name does not leak.

use crate::release_status::ReleaseStatus;

/// Everything the report shows. Values that could not be determined are
/// `None` and printed as "unknown" rather than left out, so the report always
/// has the same lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsInfo {
    pub app_version: String,
    pub release_status: ReleaseStatus,
    pub schema_version: Option<i64>,
    pub ffmpeg_version: Option<String>,
    pub ffprobe_version: Option<String>,
    pub webview_version: Option<String>,
    pub os: String,
    pub db_path: String,
    pub data_dir: String,
    pub log_dir: String,
}

/// Replaces a path prefix (e.g. the user's `%LOCALAPPDATA%` folder) with a
/// placeholder in the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaskRule {
    pub prefix: String,
    pub placeholder: String,
}

fn is_separator(b: u8) -> bool {
    b == b'\\' || b == b'/'
}

/// Replaces every occurrence of each rule's prefix in `text` with its
/// placeholder. Matching ignores ASCII case because Windows paths do, and the
/// longest prefix wins so `...\AppData\Local` is reported as
/// `%LOCALAPPDATA%` rather than as `%USERPROFILE%\AppData\Local`. A match
/// must end at a path separator or the end of the text (or the prefix itself
/// ends with a separator), so `C:\Users\alice` does not match inside
/// `C:\Users\alice2`.
///
/// Whatever the rules miss (unset variables, another spelling, another drive)
/// is caught afterwards by replacing the account folder of any
/// `<drive>:\Users\<name>` path with `<user>`.
pub fn mask_paths(text: &str, rules: &[MaskRule]) -> String {
    mask_user_profiles(&mask_by_rules(text, rules))
}

/// Replaces `<drive>:<sep>Users<sep><name>` (either separator, any case) with
/// `<drive>:\Users\<user>`; `<name>` runs to the next separator, line break,
/// quote or end of text. A `\\?\` prefix in front is left as it is.
fn mask_user_profiles(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if let Some(end) = user_profile_end(bytes, i) {
            out.push(bytes[i] as char);
            out.push_str(r":\Users\<user>");
            i = end;
            continue;
        }
        let ch = text[i..].chars().next().expect("i < len");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// If a user-profile path starts at `i`, the index just past its account
/// folder name.
fn user_profile_end(bytes: &[u8], i: usize) -> Option<usize> {
    let rest = &bytes[i..];
    if rest.len() < 10
        || !rest[0].is_ascii_alphabetic()
        || rest[1] != b':'
        || !is_separator(rest[2])
        || !rest[3..8].eq_ignore_ascii_case(b"users")
        || !is_separator(rest[8])
    {
        return None;
    }
    let name_len = rest[9..]
        .iter()
        .position(|&b| is_separator(b) || matches!(b, b'\r' | b'\n' | b'"' | b'\''))
        .unwrap_or(rest.len() - 9);
    (name_len > 0).then_some(i + 9 + name_len)
}

fn mask_by_rules(text: &str, rules: &[MaskRule]) -> String {
    let mut ordered: Vec<&MaskRule> = rules.iter().filter(|r| !r.prefix.is_empty()).collect();
    ordered.sort_by_key(|r| std::cmp::Reverse(r.prefix.len()));

    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let hit = ordered.iter().find(|r| {
            let p = r.prefix.as_bytes();
            if bytes.len() - i < p.len() || !bytes[i..i + p.len()].eq_ignore_ascii_case(p) {
                return false;
            }
            p.last().is_some_and(|&b| is_separator(b))
                || bytes.get(i + p.len()).is_none_or(|&b| is_separator(b))
        });
        match hit {
            Some(rule) => {
                out.push_str(&rule.placeholder);
                i += rule.prefix.len();
            }
            None => {
                // `i` is always on a char boundary: it only advances by a
                // whole char or by a whole (valid UTF-8) prefix.
                let ch = text[i..].chars().next().expect("i < len");
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    out
}

fn describe_release_status(status: ReleaseStatus) -> &'static str {
    match status {
        ReleaseStatus::Preview => "preview (not yet published)",
        ReleaseStatus::Published => "published stable",
        ReleaseStatus::Unknown => "unknown",
    }
}

fn or_unknown(value: Option<&str>) -> &str {
    value.unwrap_or("unknown")
}

/// Plain text, one `label: value` line per fact, with `rules` applied to the
/// whole report so no field can leak a path.
pub fn format_report(info: &DiagnosticsInfo, rules: &[MaskRule]) -> String {
    let schema = info
        .schema_version
        .map_or_else(|| "unknown".to_string(), |v| v.to_string());
    let lines = [
        "GrayBrowser diagnostics".to_string(),
        format!("App version: {}", info.app_version),
        format!(
            "Release status: {}",
            describe_release_status(info.release_status)
        ),
        format!("DB schema version: {schema}"),
        format!("FFmpeg: {}", or_unknown(info.ffmpeg_version.as_deref())),
        format!("FFprobe: {}", or_unknown(info.ffprobe_version.as_deref())),
        format!("WebView2: {}", or_unknown(info.webview_version.as_deref())),
        format!("OS: {}", info.os),
        format!("Database: {}", info.db_path),
        format!("Data folder: {}", info.data_dir),
        format!("Log folder: {}", info.log_dir),
    ];
    mask_paths(&lines.join("\n"), rules)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(prefix: &str, placeholder: &str) -> MaskRule {
        MaskRule {
            prefix: prefix.to_string(),
            placeholder: placeholder.to_string(),
        }
    }

    fn info() -> DiagnosticsInfo {
        DiagnosticsInfo {
            app_version: "0.18.1".into(),
            release_status: ReleaseStatus::Published,
            schema_version: Some(17),
            ffmpeg_version: Some("ffmpeg version 7.0".into()),
            ffprobe_version: Some("ffprobe version 7.0".into()),
            webview_version: Some("130.0.2849.80".into()),
            os: "windows x86_64".into(),
            db_path: r"C:\Users\alice\AppData\Local\GrayBrowser\app.db".into(),
            data_dir: r"C:\Users\alice\AppData\Local\GrayBrowser".into(),
            log_dir: r"C:\Users\alice\AppData\Local\GrayBrowser\logs".into(),
        }
    }

    #[test]
    fn the_report_states_the_release_status() {
        let mut preview = info();
        preview.release_status = ReleaseStatus::Preview;
        assert!(
            format_report(&preview, &[]).contains("Release status: preview (not yet published)")
        );
        assert!(format_report(&info(), &[]).contains("Release status: published stable"));
        let mut unknown = info();
        unknown.release_status = ReleaseStatus::Unknown;
        assert!(format_report(&unknown, &[]).contains("Release status: unknown"));
    }

    fn rules() -> Vec<MaskRule> {
        vec![
            rule(r"C:\Users\alice", "%USERPROFILE%"),
            rule(r"C:\Users\alice\AppData\Local", "%LOCALAPPDATA%"),
        ]
    }

    #[test]
    fn the_longest_matching_prefix_wins_regardless_of_rule_order() {
        let masked = mask_paths(r"C:\Users\alice\AppData\Local\x", &rules());
        assert_eq!(masked, r"%LOCALAPPDATA%\x");
        let mut reversed = rules();
        reversed.reverse();
        assert_eq!(
            mask_paths(r"C:\Users\alice\AppData\Local\x", &reversed),
            r"%LOCALAPPDATA%\x"
        );
    }

    #[test]
    fn matching_ignores_ascii_case() {
        assert_eq!(
            mask_paths(r"c:\users\ALICE\Desktop", &rules()),
            r"%USERPROFILE%\Desktop"
        );
    }

    #[test]
    fn every_occurrence_is_replaced() {
        let text = r"C:\Users\alice\a and C:\Users\alice\b";
        assert_eq!(
            mask_paths(text, &rules()),
            r"%USERPROFILE%\a and %USERPROFILE%\b"
        );
    }

    #[test]
    fn text_without_a_match_and_empty_rules_are_unchanged() {
        assert_eq!(mask_paths(r"D:\videos", &rules()), r"D:\videos");
        assert_eq!(mask_paths(r"C:\Users\alice", &[]), r"C:\Users\<user>");
        assert_eq!(mask_paths("", &rules()), "");
    }

    #[test]
    fn an_empty_prefix_rule_is_ignored() {
        assert_eq!(mask_paths("abc", &[rule("", "X")]), "abc");
    }

    #[test]
    fn non_ascii_user_names_are_masked_and_other_text_is_preserved() {
        let rules = [rule(r"C:\Users\太郎", "%USERPROFILE%")];
        assert_eq!(
            mask_paths(r"動画 C:\Users\太郎\あ.mp4", &rules),
            r"動画 %USERPROFILE%\あ.mp4"
        );
    }

    #[test]
    fn a_prefix_only_matches_whole_path_segments() {
        let rules = [rule(r"C:\Users\alice", "%USERPROFILE%")];
        assert_eq!(
            mask_by_rules(r"C:\Users\alice2\x", &rules),
            r"C:\Users\alice2\x"
        );
        assert_eq!(mask_by_rules(r"C:\Users\alice", &rules), "%USERPROFILE%");
        assert_eq!(
            mask_by_rules(r"C:\Users\alice\x", &rules),
            r"%USERPROFILE%\x"
        );
        assert_eq!(
            mask_by_rules("C:/Users/alice/x", &rules),
            "C:/Users/alice/x"
        );
    }

    #[test]
    fn a_prefix_ending_in_a_separator_still_matches() {
        let rules = [rule(r"C:\Users\alice\AppData\Local\", r"%LOCALAPPDATA%\")];
        assert_eq!(
            mask_by_rules(r"C:\Users\alice\AppData\Local\GrayBrowser", &rules),
            r"%LOCALAPPDATA%\GrayBrowser"
        );
    }

    #[test]
    fn a_similarly_named_account_is_masked_by_the_fallback() {
        let rules = [rule(r"C:\Users\alice", "%USERPROFILE%")];
        assert_eq!(
            mask_paths(r"C:\Users\alice2\x", &rules),
            r"C:\Users\<user>\x"
        );
    }

    #[test]
    fn the_fallback_masks_without_any_rules_on_any_drive() {
        assert_eq!(
            mask_paths(r"D:\Users\bob\Videos", &[]),
            r"D:\Users\<user>\Videos"
        );
        assert_eq!(mask_paths(r"d:\users\BOB", &[]), r"d:\Users\<user>");
    }

    #[test]
    fn the_fallback_handles_forward_slashes_and_the_long_path_prefix() {
        assert_eq!(mask_paths("C:/Users/bob/x", &[]), r"C:\Users\<user>/x");
        assert_eq!(
            mask_paths(r"\\?\C:\Users\bob\x", &[]),
            r"\\?\C:\Users\<user>\x"
        );
    }

    #[test]
    fn the_fallback_masks_japanese_and_spaced_names() {
        assert_eq!(mask_paths(r"C:\Users\太郎\あ", &[]), r"C:\Users\<user>\あ");
        assert_eq!(
            mask_paths(r"C:\Users\John Smith\x", &[]),
            r"C:\Users\<user>\x"
        );
    }

    #[test]
    fn the_fallback_leaves_other_text_alone() {
        for s in [
            r"C:\Users",
            r"C:\Users\",
            r"C:\Usersx\a",
            r"D:\videos\Users\a",
        ] {
            assert_eq!(mask_paths(s, &[]), s, "{s}");
        }
    }

    #[test]
    fn trailing_separator_rules_and_no_rules_at_all_still_hide_the_name() {
        let rules = [rule(r"C:\Users\alice\", r"%USERPROFILE%\")];
        let report = format_report(&info(), &rules);
        assert!(!report.contains("alice"), "{report}");
        let report = format_report(&info(), &[]);
        assert!(!report.contains("alice"), "{report}");
    }

    #[test]
    fn the_report_never_contains_the_user_name() {
        let report = format_report(&info(), &rules());
        assert!(!report.contains("alice"), "{report}");
        assert!(report.contains(r"Database: %LOCALAPPDATA%\GrayBrowser\app.db"));
        assert!(report.contains(r"Log folder: %LOCALAPPDATA%\GrayBrowser\logs"));
    }

    #[test]
    fn the_report_lists_every_fact() {
        let report = format_report(&info(), &rules());
        for needle in [
            "App version: 0.18.1",
            "DB schema version: 17",
            "FFmpeg: ffmpeg version 7.0",
            "FFprobe: ffprobe version 7.0",
            "WebView2: 130.0.2849.80",
            "OS: windows x86_64",
        ] {
            assert!(report.contains(needle), "missing {needle}");
        }
    }

    #[test]
    fn undetermined_values_are_printed_as_unknown() {
        let mut i = info();
        i.schema_version = None;
        i.ffmpeg_version = None;
        i.ffprobe_version = None;
        i.webview_version = None;
        let report = format_report(&i, &[]);
        assert!(report.contains("DB schema version: unknown"));
        assert!(report.contains("FFmpeg: unknown"));
        assert!(report.contains("FFprobe: unknown"));
        assert!(report.contains("WebView2: unknown"));
    }
}
