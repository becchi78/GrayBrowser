//! Decisions and wording for the developer-mode database tools: how the result
//! of `PRAGMA integrity_check` is read, whether there is room to run `VACUUM`,
//! and how sizes are shown.

/// SQLite reports at most this many problems per `integrity_check`; the cap
/// only keeps a pathological result from flooding the screen.
pub const MAX_INTEGRITY_MESSAGES: usize = 100;

/// Room kept free on top of the two copies `VACUUM` needs (see
/// `vacuum_space_check`), for the WAL and for the disk filling up meanwhile.
pub const VACUUM_SPACE_MARGIN_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityReport {
    pub ok: bool,
    /// Empty when `ok`; otherwise what SQLite reported, one problem per entry.
    pub messages: Vec<String>,
}

/// Reads the rows `PRAGMA integrity_check` returned. A healthy database
/// returns exactly one row, `ok`. Anything else -- including no rows at all,
/// which SQLite never returns -- is a failure, so an unexpected answer is
/// never shown as healthy.
pub fn format_integrity_result(rows: &[String]) -> IntegrityReport {
    if rows.len() == 1 && rows[0].trim().eq_ignore_ascii_case("ok") {
        return IntegrityReport {
            ok: true,
            messages: Vec::new(),
        };
    }
    let messages: Vec<String> = if rows.is_empty() {
        vec!["integrity_check returned no result".to_string()]
    } else {
        rows.iter().take(MAX_INTEGRITY_MESSAGES).cloned().collect()
    };
    IntegrityReport {
        ok: false,
        messages,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsufficientSpace {
    pub required: u64,
    pub available: u64,
}

/// The estimate assumes the database and its `backups` folder are on the same
/// drive, with the WAL emptied by a checkpoint just before the check. The
/// drive of SQLite's temporary files (TEMP) is not checked.
///
/// `VACUUM` needs room for a rebuilt copy of the database, and the backup
/// taken just before it needs another full copy, so twice the database size
/// plus a margin must be free. The check is made before the backup so a full
/// disk is refused up front instead of failing halfway.
pub fn vacuum_space_check(db_size: u64, free_space: u64) -> Result<(), InsufficientSpace> {
    let required = db_size
        .saturating_mul(2)
        .saturating_add(VACUUM_SPACE_MARGIN_BYTES);
    if free_space >= required {
        Ok(())
    } else {
        Err(InsufficientSpace {
            required,
            available: free_space,
        })
    }
}

/// Binary units with one decimal (`1.5 MB`), plain bytes below 1 KB.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// What `VACUUM` could give back: free pages times the page size.
pub fn reclaimable_bytes(page_size: u64, freelist_count: u64) -> u64 {
    page_size.saturating_mul(freelist_count)
}

/// The message shown when `vacuum_space_check` refuses.
pub fn describe_insufficient_space(e: &InsufficientSpace) -> String {
    format!(
        "ディスクの空き容量が足りないため、最適化を行いません(必要: {}、空き: {})。",
        format_bytes(e.required),
        format_bytes(e.available)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_single_ok_row_is_healthy() {
        let r = format_integrity_result(&rows(&["ok"]));
        assert!(r.ok);
        assert!(r.messages.is_empty());
        assert!(format_integrity_result(&rows(&[" OK "])).ok);
    }

    #[test]
    fn any_other_answer_is_a_failure() {
        let r = format_integrity_result(&rows(&["row 1 missing from index x", "page 5 is bad"]));
        assert!(!r.ok);
        assert_eq!(r.messages.len(), 2);
        // "ok" among other rows does not make it healthy.
        assert!(!format_integrity_result(&rows(&["ok", "problem"])).ok);
    }

    #[test]
    fn no_rows_is_not_shown_as_healthy() {
        let r = format_integrity_result(&[]);
        assert!(!r.ok);
        assert_eq!(r.messages.len(), 1);
    }

    #[test]
    fn messages_are_capped() {
        let many: Vec<String> = (0..MAX_INTEGRITY_MESSAGES + 50)
            .map(|i| format!("problem {i}"))
            .collect();
        let r = format_integrity_result(&many);
        assert_eq!(r.messages.len(), MAX_INTEGRITY_MESSAGES);
    }

    #[test]
    fn vacuum_needs_twice_the_database_plus_margin() {
        let db = 1000;
        let need = 2 * db + VACUUM_SPACE_MARGIN_BYTES;
        assert!(vacuum_space_check(db, need).is_ok());
        let err = vacuum_space_check(db, need - 1).unwrap_err();
        assert_eq!(err.required, need);
        assert_eq!(err.available, need - 1);
    }

    #[test]
    fn vacuum_space_check_handles_extremes() {
        assert!(vacuum_space_check(0, VACUUM_SPACE_MARGIN_BYTES).is_ok());
        assert!(vacuum_space_check(0, 0).is_err());
        assert!(vacuum_space_check(u64::MAX, u64::MAX - 1).is_err());
        assert!(vacuum_space_check(u64::MAX, u64::MAX).is_ok());
    }

    #[test]
    fn bytes_are_formatted_in_binary_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(5 * 1024 * 1024 * 1024), "5.0 GB");
        assert_eq!(format_bytes(u64::MAX), "16777216.0 TB");
    }

    #[test]
    fn reclaimable_is_pages_times_size_without_overflow() {
        assert_eq!(reclaimable_bytes(4096, 10), 40960);
        assert_eq!(reclaimable_bytes(4096, 0), 0);
        assert_eq!(reclaimable_bytes(u64::MAX, 2), u64::MAX);
    }

    #[test]
    fn the_refusal_message_names_both_sizes() {
        let m = describe_insufficient_space(&InsufficientSpace {
            required: 2048,
            available: 1024,
        });
        assert!(m.contains("2.0 KB") && m.contains("1.0 KB"));
    }
}
