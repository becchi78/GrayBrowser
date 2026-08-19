//! Reads a real `.wb` (legacy WhiteBrowser SQLite DB) read-only, anonymizes
//! it deterministically, runs the leak/coverage check,
//! and -- only if that check passes -- writes a stratified sample of the
//! anonymized `movie` rows to a brand-new output file. Never opens the
//! output path for reading first, and refuses to run at all if the input
//! and output paths resolve to the same file.
//!
//! Usage: `wb-anonymize-tool <input .wb path> <output .wb path>`

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use gb_core::wb_anonymize::Anonymizer;
use gb_core::wb_import::count_clamped_scores;
use gb_core::wb_sampling::select_sample_rows;
use wb_anonymize_tool::{leak_check, reader, writer};

const SAMPLE_TARGET_COUNT: usize = 50;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: wb-anonymize-tool <input .wb path> <output .wb path>");
        return ExitCode::FAILURE;
    }
    let input_path = PathBuf::from(&args[1]);
    let output_path = PathBuf::from(&args[2]);

    if let Err(e) = check_paths_distinct(&input_path, &output_path) {
        eprintln!("refusing to run: {e}");
        return ExitCode::FAILURE;
    }

    match run(&input_path, &output_path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("failed: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Resolves both paths to their absolute, canonical form (following
/// symlinks where possible) and refuses to proceed if they match -- the one
/// hard guarantee that stands between this tool and overwriting real
/// personal data.
fn check_paths_distinct(input: &Path, output: &Path) -> io::Result<()> {
    let input_abs = resolve_absolute(input)?;
    let output_abs = resolve_absolute(output)?;
    if input_abs == output_abs {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "input and output paths resolve to the same file ({}) -- refusing to write to the real data file",
                input_abs.display()
            ),
        ));
    }
    Ok(())
}

fn resolve_absolute(path: &Path) -> io::Result<PathBuf> {
    if let Ok(canon) = fs::canonicalize(path) {
        return Ok(canon);
    }
    // Output path doesn't exist yet -- canonicalize its parent (which must
    // exist) and rejoin the file name so the comparison still works.
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let canon_parent = fs::canonicalize(&parent)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    Ok(canon_parent.join(file_name))
}

fn run(input_path: &Path, output_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let input_conn = reader::open_readonly(input_path)?;
    let mut movies = reader::read_movies(&input_conn)?;
    movies.sort_by_key(|r| r.movie_id);
    let real_cells = reader::read_all_text_cells(&input_conn)?;
    drop(input_conn); // input is never touched again

    // One shared Anonymizer for both the row-level transform and the
    // cell-level leak check, so a value seen in both places (e.g. a
    // `movie.hash` anonymized here and again while scanning
    // `read_all_text_cells`) always resolves to the identical cached dummy
    // value rather than being independently (and, for `Hash`, possibly
    // non-identically) re-derived.
    let mut anonymizer = Anonymizer::new();

    let anonymized_movies: Vec<_> = movies
        .iter()
        .map(|r| anonymizer.anonymize_movie_row(r))
        .collect();

    let report = leak_check::run(&real_cells, &mut anonymizer);
    eprintln!(
        "leak_check: cells_checked={} token_candidate_count={} token_excluded_short={} \
         token_kept_min_len={:?} token_kept_max_len={:?} negative_violations={} positive_violations={}",
        report.cells_checked,
        report.token_stats.candidate_count,
        report.token_stats.excluded_short,
        report.token_stats.kept_min_len,
        report.token_stats.kept_max_len,
        report.negative_violations,
        report.positive_violations,
    );
    if !report.passed() {
        return Err(Box::new(io::Error::other(
            "leak check failed -- refusing to write the fixture (see leak_check counts above)",
        )));
    }

    let clamped_scores = count_clamped_scores(&movies);
    let sample = select_sample_rows(&anonymized_movies, SAMPLE_TARGET_COUNT);

    let output_conn = writer::create_output(output_path)?;
    writer::write_movies(&output_conn, &sample)?;

    eprintln!(
        "input_movies={} clamped_scores={} sample_movies={} output={}",
        movies.len(),
        clamped_scores,
        sample.len(),
        output_path.display()
    );

    Ok(())
}
