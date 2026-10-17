//! Pure naming and retention logic for database backups. Producing the
//! backup file itself (`VACUUM INTO`, verification, directory listing) is the
//! `src-tauri::db::backup` layer's job.
//!
//! File names embed a UTC timestamp rather than relying on file mtimes, so
//! retention order survives the backups directory being copied or restored.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupKind {
    /// Taken automatically right before pending schema migrations run.
    Premigrate,
    /// Taken on explicit user request.
    Manual,
}

impl BackupKind {
    fn label(self) -> &'static str {
        match self {
            BackupKind::Premigrate => "premigrate",
            BackupKind::Manual => "manual",
        }
    }
}

const PREFIX: &str = "app-";
const SUFFIX: &str = ".db";

/// `app-<kind>-<YYYYMMDDTHHMMSSZ>.db`, with the timestamp in UTC. The
/// fixed-width timestamp makes names of one kind sort lexicographically in
/// chronological order.
pub fn backup_file_name(kind: BackupKind, unix_secs: i64) -> String {
    let (y, mo, d, h, mi, s) = civil_from_unix(unix_secs);
    format!(
        "{PREFIX}{}-{y:04}{mo:02}{d:02}T{h:02}{mi:02}{s:02}Z{SUFFIX}",
        kind.label()
    )
}

/// Inverse of [`backup_file_name`]. Returns `None` for any name not produced
/// by it, so unrelated files in the backups directory are never treated as
/// backups (and never deleted).
pub fn parse_backup_file_name(name: &str) -> Option<(BackupKind, i64)> {
    let rest = name.strip_prefix(PREFIX)?.strip_suffix(SUFFIX)?;
    let (kind_str, ts) = rest.split_once('-')?;
    let kind = match kind_str {
        "premigrate" => BackupKind::Premigrate,
        "manual" => BackupKind::Manual,
        _ => return None,
    };
    let b = ts.as_bytes();
    if b.len() != 16 || b[8] != b'T' || b[15] != b'Z' {
        return None;
    }
    let digits = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = ts.get(range)?;
        if part.bytes().all(|c| c.is_ascii_digit()) {
            part.parse().ok()
        } else {
            None
        }
    };
    let (y, mo, d) = (digits(0..4)?, digits(4..6)?, digits(6..8)?);
    let (h, mi, s) = (digits(9..11)?, digits(11..13)?, digits(13..15)?);
    if !(1..=12).contains(&mo) || d < 1 || d > days_in_month(y, mo) || h > 23 || mi > 59 || s > 59 {
        return None;
    }
    let secs = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s;
    Some((kind, secs))
}

/// Returns the names to delete so that at most `keep_premigrate` premigrate
/// and `keep_manual` manual backups remain, dropping the oldest first. The
/// two kinds are counted separately so a burst of manual backups can never
/// push out the pre-migration safety copies. Names that do not parse are
/// ignored.
pub fn select_for_deletion(
    names: &[String],
    keep_premigrate: usize,
    keep_manual: usize,
) -> Vec<String> {
    let mut doomed = Vec::new();
    for (kind, keep) in [
        (BackupKind::Premigrate, keep_premigrate),
        (BackupKind::Manual, keep_manual),
    ] {
        let mut of_kind: Vec<(i64, &String)> = names
            .iter()
            .filter_map(|n| match parse_backup_file_name(n) {
                Some((k, t)) if k == kind => Some((t, n)),
                _ => None,
            })
            .collect();
        // Newest first; the name is a deterministic tie-breaker.
        of_kind.sort_by(|a, b| b.cmp(a));
        doomed.extend(of_kind.into_iter().skip(keep).map(|(_, n)| n.clone()));
    }
    doomed
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(y) => 29,
        _ => 28,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_unix(secs: i64) -> (i64, i64, i64, i64, i64, i64) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(kind: BackupKind, secs: i64) -> String {
        backup_file_name(kind, secs)
    }

    #[test]
    fn formats_known_instants() {
        assert_eq!(
            backup_file_name(BackupKind::Manual, 0),
            "app-manual-19700101T000000Z.db"
        );
        // 2024-02-29 12:34:56 UTC (leap day)
        assert_eq!(
            backup_file_name(BackupKind::Premigrate, 1_709_210_096),
            "app-premigrate-20240229T123456Z.db"
        );
        assert_eq!(
            backup_file_name(BackupKind::Manual, 946_684_799),
            "app-manual-19991231T235959Z.db"
        );
    }

    #[test]
    fn name_round_trips() {
        for secs in [
            0,
            1,
            86_399,
            86_400,
            951_782_400,
            1_709_210_096,
            4_102_444_800,
        ] {
            for kind in [BackupKind::Premigrate, BackupKind::Manual] {
                let name = backup_file_name(kind, secs);
                assert_eq!(parse_backup_file_name(&name), Some((kind, secs)), "{name}");
            }
        }
    }

    #[test]
    fn rejects_malformed_names() {
        for bad in [
            "",
            "app.db",
            "app-manual-19700101T000000Z.db.bak",
            "app-other-19700101T000000Z.db",
            "app-manual-19700101T000000.db",
            "app-manual-1970010T0000000Z.db",
            "app-manual-19701301T000000Z.db",
            "app-manual-19700132T000000Z.db",
            "app-manual-19710229T000000Z.db",
            "app-manual-19700101T240000Z.db",
            "app-manual-19700101T006000Z.db",
            "app-manual-1970010aT000000Z.db",
            "manual-19700101T000000Z.db",
            "app-manual-+9700101T000000Z.db",
        ] {
            assert_eq!(parse_backup_file_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn keeps_newest_per_kind_and_deletes_oldest() {
        let names = vec![
            n(BackupKind::Premigrate, 100),
            n(BackupKind::Premigrate, 200),
            n(BackupKind::Premigrate, 300),
            n(BackupKind::Premigrate, 400),
            n(BackupKind::Manual, 10),
            n(BackupKind::Manual, 20),
        ];
        let doomed = select_for_deletion(&names, 3, 1);
        assert_eq!(
            doomed,
            vec![n(BackupKind::Premigrate, 100), n(BackupKind::Manual, 10)]
        );
    }

    #[test]
    fn kinds_have_independent_quotas() {
        let mut names: Vec<String> = (0..50).map(|i| n(BackupKind::Manual, 1000 + i)).collect();
        names.push(n(BackupKind::Premigrate, 1));
        let doomed = select_for_deletion(&names, 3, 10);
        assert_eq!(doomed.len(), 40);
        assert!(!doomed.contains(&n(BackupKind::Premigrate, 1)));
    }

    #[test]
    fn deletes_nothing_when_within_quota() {
        let names = vec![n(BackupKind::Premigrate, 1), n(BackupKind::Manual, 2)];
        assert!(select_for_deletion(&names, 3, 10).is_empty());
        assert!(select_for_deletion(&[], 3, 10).is_empty());
    }

    #[test]
    fn ignores_unrelated_names_and_is_input_order_independent() {
        let names = vec![
            n(BackupKind::Manual, 30),
            "notes.txt".to_string(),
            "app.db".to_string(),
            n(BackupKind::Manual, 10),
            n(BackupKind::Manual, 20),
        ];
        assert_eq!(
            select_for_deletion(&names, 3, 2),
            vec![n(BackupKind::Manual, 10)]
        );
        // Zero quota deletes every backup of that kind but never unrelated files.
        let all = select_for_deletion(&names, 3, 0);
        assert_eq!(all.len(), 3);
        assert!(!all.contains(&"notes.txt".to_string()));
    }
}
