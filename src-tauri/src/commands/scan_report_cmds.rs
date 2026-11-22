//! The scan report shown in developer mode. The scan keeps the numbers of its
//! most recent run in memory; this command adds what is only known later (the
//! worker tallies, the files the database holds as failed) and turns it into
//! text. The report lists file paths, so the command repeats the
//! developer-mode check rather than relying on the menu hiding it.

use gb_core::diagnostics::MaskRule;
use gb_core::scan_report::{format_report, FailedFile, ScanReport, WorkStats};
use tauri::State;

use super::developer_cmds::ensure_developer_mode;
use super::diagnostics_cmds::mask_rules_from_env;
use crate::db::{queries, Db};
use crate::scan::last_report::{LastScanReport, METADATA_WORK, THUMBNAIL_WORK};

/// The report as text, or `None` when no scan has run since the app started.
/// The developer-mode check comes first, so a refused caller cannot even learn
/// whether a report exists.
fn scan_report_text(
    db: &Db,
    stored: Option<ScanReport>,
    thumbnails: WorkStats,
    metadata: WorkStats,
    rules: &[MaskRule],
) -> Result<Option<String>, String> {
    {
        let conn = db.read_pool.get().map_err(|e| e.to_string())?;
        ensure_developer_mode(&conn)?;
    }
    let Some(mut report) = stored else {
        return Ok(None);
    };
    report.thumbnails = thumbnails;
    report.metadata = metadata;
    report.skipped = queries::list_skipped_files(&db.read_pool)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|row| FailedFile {
            reason: match row.detected_char {
                Some(c) => format!("{} (character {c})", row.reason),
                None => row.reason,
            },
            path: row.file_path,
        })
        .collect();
    report.thumbnail_failures =
        queries::list_videos_with_exhausted_thumbnail_attempts(&db.read_pool)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|r| r.file_path)
            .collect();
    report.metadata_failures = queries::list_videos_with_exhausted_metadata_attempts(&db.read_pool)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|r| r.file_path)
        .collect();
    Ok(Some(format_report(&report, rules)))
}

#[tauri::command(async)]
pub fn get_scan_report(
    db: State<'_, Db>,
    last: State<'_, LastScanReport>,
) -> Result<Option<String>, String> {
    scan_report_text(
        &db,
        last.get(),
        THUMBNAIL_WORK.snapshot(),
        METADATA_WORK.snapshot(),
        &mask_rules_from_env(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::test_support::{init_temp_db, insert_test_video};
    use gb_core::scan_pipeline::NewSkippedFile;
    use std::time::Duration;

    fn enable(db: &Db) {
        queries::set_developer_mode(&db.writer.lock().unwrap(), true).unwrap();
    }

    fn stored() -> Option<ScanReport> {
        Some(ScanReport {
            folders: 1,
            ..ScanReport::default()
        })
    }

    fn text(db: &Db, stored: Option<ScanReport>) -> Result<Option<String>, String> {
        scan_report_text(db, stored, WorkStats::default(), WorkStats::default(), &[])
    }

    #[test]
    fn refused_unless_developer_mode_is_on() {
        let (_d, db) = init_temp_db();
        assert_eq!(
            text(&db, stored()).unwrap_err(),
            super::super::developer_cmds::DEVELOPER_MODE_REQUIRED
        );
        enable(&db);
        assert!(text(&db, stored()).is_ok());
    }

    #[test]
    fn the_refusal_comes_before_the_missing_report_answer() {
        let (_d, db) = init_temp_db();
        assert!(text(&db, None).is_err());
    }

    #[test]
    fn no_report_before_the_first_scan() {
        let (_d, db) = init_temp_db();
        enable(&db);
        assert_eq!(text(&db, None), Ok(None));
    }

    #[test]
    fn the_text_adds_worker_tallies_and_the_failures_in_the_database() {
        let (_d, db) = init_temp_db();
        enable(&db);
        {
            let conn = db.writer.lock().unwrap();
            queries::upsert_skipped_file(
                &conn,
                &NewSkippedFile {
                    file_path: "C:/a/x\u{2460}.mp4".into(),
                    file_name: "x\u{2460}.mp4".into(),
                    reason: "machine_dependent_char",
                    detected_char: '\u{2460}',
                },
            )
            .unwrap();
        }
        insert_test_video(&db, "v1", "C:/a/broken.mp4");
        {
            let conn = db.writer.lock().unwrap();
            conn.execute(
                "UPDATE videos SET thumbnail_attempts = ?1 WHERE id = 'v1'",
                [i64::from(gb_core::retry::MAX_GENERATION_ATTEMPTS)],
            )
            .unwrap();
        }
        let thumbs = WorkStats {
            count: 3,
            failures: 1,
            total: Duration::from_millis(30),
        };

        let text = scan_report_text(&db, stored(), thumbs, WorkStats::default(), &[])
            .unwrap()
            .unwrap();

        assert!(text.contains("Thumbnails: 3 processed, 1 failed"));
        assert!(text.contains("machine_dependent_char (character \u{2460})"));
        assert!(text.contains("Thumbnail generation failures: 1\n  C:/a/broken.mp4"));
    }
}
