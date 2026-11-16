//! A manual-only performance
//! benchmark for `queries::list_videos_filtered` -- the query behind
//! search/sort/tag-filter -- run against a dataset shaped like a real user's
//! library rather than uniform `SYNTH_####` synthetic rows (which
//! didn't vary file name length/script or leave `kana`/`roma` mostly NULL the
//! way real scanned-then-partially-.wb-imported data does).
//!
//! This measures the DB layer only (`std::time::Instant` around a direct
//! `list_videos_filtered` call, no HTTP/Tauri IPC in the loop) at 10,000 /
//! 50,000 / 100,000 rows, for these query-pattern combinations:
//! unfiltered browse, a short (broadly-matching) search term, a
//! long/specific (narrowly-matching) search term, a search term ANDed with a
//! tag filter, and each `SortField`.
//!
//! **Must be run `--release`** -- a debug build's numbers are not
//! representative of the shipped app and are not what this benchmark exists
//! to produce:
//!
//! ```text
//! cargo test --release --test search_performance_bench -- --ignored --nocapture
//! ```
//!
//! `#[ignore]`d on every test so `cargo test --all-features --workspace`
//! (debug, no `--ignored`) never runs this and never pays its build time.

mod common;

use common::init_temp_db;
use gb_core::sort::{SortDirection, SortField};
use graybrowser_lib::db::{self, queries};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Deterministic PRNG (no `rand` dependency: this is a `src-tauri`-only test
// binary and adding a new crate to `Cargo.toml` for one benchmark file would
// be an aggregate-file change for no real benefit -- xorshift64* is more than
// enough quality for shaping synthetic data). Fixed seed per dataset build so
// a benchmark run is reproducible.
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        // xorshift64* requires a non-zero seed.
        Rng(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn next_range(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// Uniform float in `[0, 1)`.
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn chance(&mut self, p: f64) -> bool {
        self.next_f64() < p
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.next_range(items.len())]
    }
}

// ---------------------------------------------------------------------------
// Character/word pools -- real, assigned Unicode ranges (not gaiji/PUA), a
// mix of ASCII words and the decorations/brackets/episode markers real
// video-file names accumulate.
// ---------------------------------------------------------------------------

struct Pools {
    hiragana: Vec<char>,
    katakana: Vec<char>,
    kanji: Vec<char>,
    ascii_words: Vec<&'static str>,
    decorations: Vec<&'static str>,
    extensions: Vec<&'static str>,
    drives: Vec<&'static str>,
    folder_names: Vec<&'static str>,
    romaji_syllables: Vec<&'static str>,
    tag_pool: Vec<String>,
}

/// Deliberately embedded, at a controlled probability, into a minority of
/// generated file names so query pattern (b) (short search term) has a
/// guaranteed, non-trivial hit count instead of gambling on a random
/// substring the generator happens to have produced somewhere.
const SHORT_TOKEN: &str = "BD";
const SHORT_TOKEN_PROBABILITY: f64 = 0.03;

/// Same idea as `SHORT_TOKEN`, but a longer/specific token at a much lower
/// probability, so query pattern (c) (long/specific term) reliably narrows
/// down to a small result set rather than a broad one.
const LONG_TOKEN: &str = "DirectorsCutSpecialEdition2024";
const LONG_TOKEN_PROBABILITY: f64 = 0.003;

/// Fraction of rows that additionally get `popular_tag_ids[0]`/`[1]`
/// (independent of the normal random 0..=5 tag assignment below), so query
/// pattern (d) (search term AND tag filter) has a guaranteed non-zero
/// intersection at every dataset size instead of relying on two independent
/// low-probability events to coincide by chance.
const POPULAR_TAG_0_PROBABILITY: f64 = 0.25;
const POPULAR_TAG_1_PROBABILITY: f64 = 0.15;

fn build_pools() -> Pools {
    let hiragana: Vec<char> = (0x3041u32..=0x3096).filter_map(char::from_u32).collect();
    // ぁ..ゖ plus katakana middle dot(・)/prolonged sound mark(ー), a
    // contiguous, fully-assigned run of the real Katakana Unicode block.
    let katakana: Vec<char> = (0x30A1u32..=0x30FC).filter_map(char::from_u32).collect();
    // A sub-range of CJK Unified Ideographs (fully assigned, real characters
    // -- not a gaiji/PUA concern, which is a separate filename-validation
    // path this benchmark doesn't exercise).
    let kanji: Vec<char> = (0x4E00u32..=0x5FFF).filter_map(char::from_u32).collect();

    let ascii_words = vec![
        "Video",
        "Movie",
        "Clip",
        "Recording",
        "Show",
        "Episode",
        "Special",
        "Trailer",
        "Highlights",
        "Backup",
        "Archive",
        "Live",
        "Concert",
        "Session",
        "Interview",
        "Demo",
        "Sample",
        "Final",
        "Draft",
        "Master",
        "Raw",
        "Edit",
        "Cut",
        "Scene",
        "Take",
        "Part",
        "Vol",
        "Disc",
        "Track",
        "Mix",
        "Tour",
        "Report",
        "Rehearsal",
        "Broadcast",
        "Rerun",
    ];

    let decorations = vec![
        "[1080p]",
        "[720p]",
        "[4K]",
        "(DVD)",
        "第01話",
        "第02話",
        "第12話",
        "-final-",
        "_v2",
        "(2024)",
        "(2023)",
        "【新作】",
        "【廃盤】",
        "x264",
        "HEVC",
        "FLAC",
        "5.1ch",
        "字幕版",
        "吹替版",
        "劇場版",
        "(uncut)",
        "[HDR]",
        "総集編",
    ];

    let extensions = vec![".mp4", ".mkv", ".avi", ".wmv", ".mov", ".ts", ".m2ts"];
    let drives = vec!["C:", "D:", "T:", "U:"];

    let folder_names = vec![
        "Videos",
        "Movies",
        "Anime",
        "TVShows",
        "Downloads",
        "Archive",
        "Backup",
        "External",
        "Recorded",
        "Series",
        "Documentary",
        "Concert",
        "動画",
        "アニメ",
        "録画",
        "ダウンロード",
        "映画",
        "ドラマ",
        "バラエティ",
        "2023",
        "2024",
        "Work",
        "Personal",
        "Family",
        "Old",
        "New",
        "Sorted",
        "Unsorted",
    ];

    let romaji_syllables = vec![
        "ka", "ki", "ku", "ke", "ko", "sa", "shi", "su", "se", "so", "ta", "chi", "tsu", "te",
        "to", "na", "ni", "nu", "ne", "no", "ha", "hi", "fu", "he", "ho", "ma", "mi", "mu", "me",
        "mo", "ya", "yu", "yo", "ra", "ri", "ru", "re", "ro", "wa", "n", "ga", "gi", "gu", "ge",
        "go",
    ];

    let tag_pool_raw = [
        "Action",
        "Comedy",
        "Drama",
        "Romance",
        "Horror",
        "SciFi",
        "Fantasy",
        "Documentary",
        "Anime",
        "Idol",
        "Music",
        "Concert",
        "LiveShow",
        "Sports",
        "News",
        "Variety",
        "Cooking",
        "Travel",
        "Nature",
        "History",
        "War",
        "Mystery",
        "Thriller",
        "Adventure",
        "Family",
        "Kids",
        "Education",
        "Tutorial",
        "Gaming",
        "ESports",
        "Vlog",
        "Interview",
        "Review",
        "Unboxing",
        "Tech",
        "Science",
        "Space",
        "Ocean",
        "Wildlife",
        "Cars",
        "Motorsport",
        "Fitness",
        "Yoga",
        "Dance",
        "Theater",
        "Opera",
        "Classical",
        "Jazz",
        "Rock",
        "Pop",
        "HipHop",
        "Indie",
        "Karaoke",
        "Wedding",
        "Birthday",
        "Festival",
        "Fireworks",
        "Parade",
        "Ceremony",
        "Graduation",
        "Recital",
        "BehindTheScenes",
        "Bloopers",
        "TeaserRaw",
        "RawFootage",
        "Uncategorized",
        "地上波",
        "BS放送",
        "CS放送",
        "深夜アニメ",
        "劇場版",
        "OVA",
        "スペシャル",
        "字幕",
        "吹替",
        "邦画",
        "洋画",
    ];
    // First two entries (`Action`, `Comedy`) double as the "popular tags"
    // deliberately over-assigned below, for query pattern (d).
    let tag_pool: Vec<String> = tag_pool_raw.iter().map(|s| s.to_string()).collect();

    Pools {
        hiragana,
        katakana,
        kanji,
        ascii_words,
        decorations,
        extensions,
        drives,
        folder_names,
        romaji_syllables,
        tag_pool,
    }
}

/// Builds one file name: length distributed 15..120 characters (short/
/// medium/long buckets), content mixing ASCII words, hiragana, katakana,
/// kanji and decoration tokens, then (at a controlled low probability) an
/// embedded `SHORT_TOKEN`/`LONG_TOKEN` anchor for the search benchmark's
/// guaranteed-hit query patterns.
fn gen_filename(rng: &mut Rng, pools: &Pools) -> String {
    let bucket = rng.next_f64();
    let target_len = if bucket < 0.2 {
        15 + rng.next_range(10) // ~15-24 chars: short, ascii-ish names
    } else if bucket < 0.7 {
        30 + rng.next_range(40) // ~30-69 chars: medium
    } else {
        70 + rng.next_range(50) // ~70-119 chars: long, decorated titles
    };

    let mut body = String::new();
    if rng.chance(0.4) {
        body.push_str(rng.pick(&pools.decorations));
        body.push(' ');
    }
    while body.chars().count() < target_len {
        let piece = rng.next_f64();
        if piece < 0.30 {
            body.push_str(rng.pick(&pools.ascii_words));
        } else if piece < 0.50 {
            body.push(*rng.pick(&pools.hiragana));
        } else if piece < 0.70 {
            body.push(*rng.pick(&pools.katakana));
        } else if piece < 0.90 {
            body.push(*rng.pick(&pools.kanji));
        } else {
            body.push_str(rng.pick(&pools.decorations));
        }
        if rng.chance(0.15) {
            body.push(*rng.pick(&['-', '_', ' ', '.']));
        }
    }
    if rng.chance(0.3) {
        body.push(' ');
        body.push_str(rng.pick(&pools.decorations));
    }

    if rng.chance(SHORT_TOKEN_PROBABILITY) {
        body.push('_');
        body.push_str(SHORT_TOKEN);
    }
    if rng.chance(LONG_TOKEN_PROBABILITY) {
        body.push('_');
        body.push_str(LONG_TOKEN);
    }

    let ext = rng.pick(&pools.extensions);
    format!("{body}{ext}")
}

/// Builds a plausible multi-drive, multi-level path for `file_name` (real
/// `.wb` libraries spread across several drive
/// letters / external HDDs, not one flat folder).
fn gen_path(rng: &mut Rng, pools: &Pools, file_name: &str) -> String {
    let drive = rng.pick(&pools.drives);
    let depth = 1 + rng.next_range(3); // 1..=3 folder levels
    let mut path = (*drive).to_string();
    for _ in 0..depth {
        path.push('\\');
        path.push_str(rng.pick(&pools.folder_names));
    }
    path.push('\\');
    path.push_str(file_name);
    path
}

fn gen_kana(rng: &mut Rng, pools: &Pools) -> String {
    let len = 5 + rng.next_range(8);
    (0..len).map(|_| *rng.pick(&pools.katakana)).collect()
}

fn gen_roma(rng: &mut Rng, pools: &Pools) -> String {
    let syllable_count = 3 + rng.next_range(4);
    (0..syllable_count)
        .map(|_| *rng.pick(&pools.romaji_syllables))
        .collect()
}

/// `2026-MM-DD HH:MM:SS`-shaped (matches `sort_index_usage.rs`'s existing
/// convention) but spread across a full year rather than one hour, so
/// `ORDER BY created_at` has real variance to sort across 10k-100k rows.
fn gen_created_at(rng: &mut Rng) -> String {
    let month = 1 + rng.next_range(12);
    let day = 1 + rng.next_range(28); // avoid per-month day-count edge cases
    let hour = rng.next_range(24);
    let minute = rng.next_range(60);
    let second = rng.next_range(60);
    format!("2026-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

/// Fraction of rows whose thumbnail is already generated, chosen to resemble a
/// library that has been running a while (most, not all, videos have one).
const THUMBNAIL_READY_FRACTION: f64 = 0.85;

struct Dataset {
    _dir: tempfile::TempDir,
    db: db::Db,
    /// DB ids of `tag_pool[0]` ("Action") / `tag_pool[1]` ("Comedy") -- the
    /// two tags over-assigned per `POPULAR_TAG_*_PROBABILITY`, used for query
    /// pattern (d)'s tag-AND filter.
    popular_tag_ids: [i64; 2],
}

fn build_dataset(row_count: usize, seed: u64) -> Dataset {
    let (dir, db) = init_temp_db();

    let pools = build_pools();
    let mut rng = Rng::new(seed);

    let popular_tag_ids;
    {
        let mut conn = db.writer.lock().unwrap();
        let tx = conn.transaction().unwrap();

        let mut tag_ids: Vec<i64> = Vec::with_capacity(pools.tag_pool.len());
        {
            let mut insert_tag = tx.prepare("INSERT INTO tags (name) VALUES (?1)").unwrap();
            for name in &pools.tag_pool {
                insert_tag.execute(params![name]).unwrap();
                tag_ids.push(tx.last_insert_rowid());
            }
        }
        popular_tag_ids = [tag_ids[0], tag_ids[1]];

        {
            let mut insert_video = tx
                .prepare(
                    "INSERT INTO videos
                        (id, file_path, file_name, file_size, quick_hash, status, rating,
                         mtime, created_at, kana, roma, thumbnail_ready)
                     VALUES (?1,?2,?3,?4,?5,'online',?6,?7,?8,?9,?10,?11)",
                )
                .unwrap();
            let mut insert_video_tag = tx
                .prepare("INSERT INTO video_tags (video_id, tag_id) VALUES (?1, ?2)")
                .unwrap();

            for _ in 0..row_count {
                let id = uuid::Uuid::new_v4().to_string();
                let file_name = gen_filename(&mut rng, &pools);
                let file_path = gen_path(&mut rng, &pools, &file_name);
                let file_size = 50_000_000i64 + (rng.next_u64() % 8_000_000_000) as i64;
                let quick_hash = format!("{:016x}", rng.next_u64());
                let rating = rng.next_range(6) as i64;
                let mtime: Option<i64> = if rng.chance(0.10) {
                    None
                } else {
                    Some(1_700_000_000 + rng.next_range(60_000_000) as i64)
                };
                let created_at = gen_created_at(&mut rng);
                let kana: Option<String> = if rng.chance(0.25) {
                    Some(gen_kana(&mut rng, &pools))
                } else {
                    None
                };
                let roma: Option<String> = if kana.is_some() {
                    Some(gen_roma(&mut rng, &pools))
                } else {
                    None
                };
                let thumbnail_ready = rng.chance(THUMBNAIL_READY_FRACTION);

                insert_video
                    .execute(params![
                        id,
                        file_path,
                        file_name,
                        file_size,
                        quick_hash,
                        rating,
                        mtime,
                        created_at,
                        kana,
                        roma,
                        thumbnail_ready
                    ])
                    .unwrap();

                // Independent 0..=5 random tags, plus a probability boost for
                // the two "popular" tags so query (d)'s AND-filter always has
                // a realistic, non-zero hit count to actually measure.
                let mut assigned = std::collections::HashSet::new();
                let count = rng.next_range(6); // 0..=5
                for _ in 0..count {
                    assigned.insert(rng.next_range(pools.tag_pool.len()));
                }
                if rng.chance(POPULAR_TAG_0_PROBABILITY) {
                    assigned.insert(0);
                }
                if rng.chance(POPULAR_TAG_1_PROBABILITY) {
                    assigned.insert(1);
                }
                for tag_idx in assigned {
                    insert_video_tag
                        .execute(params![id, tag_ids[tag_idx]])
                        .unwrap();
                }
            }
        }

        tx.commit().unwrap();
    }

    Dataset {
        _dir: dir,
        db,
        popular_tag_ids,
    }
}

/// Times `run_once` (which returns how many rows it produced) `runs` times,
/// returning `(median_ms, last_row_count)`. A median over several runs
/// (rather than a single measurement) smooths out first-call/cold-cache noise
/// without needing a dedicated separate warmup call.
fn median_of_runs(runs: usize, mut run_once: impl FnMut() -> usize) -> (f64, usize) {
    let mut samples_ms = Vec::with_capacity(runs);
    let mut last_count = 0;
    for _ in 0..runs {
        let start = Instant::now();
        last_count = run_once();
        samples_ms.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (samples_ms[samples_ms.len() / 2], last_count)
}

/// Runs `list_videos_filtered` `runs` times, returning `(median_ms,
/// last_result_row_count)`.
fn measure_median(
    pool: &r2d2::Pool<SqliteConnectionManager>,
    label: &str,
    search_terms: &[String],
    sort_field: SortField,
    sort_direction: SortDirection,
    tag_ids: &[i64],
    runs: usize,
) -> (f64, usize) {
    median_of_runs(runs, || {
        queries::list_videos_filtered(
            pool,
            search_terms,
            sort_field,
            sort_direction,
            tag_ids,
            None,
            &[],
        )
        .unwrap_or_else(|e| panic!("{label} query failed: {e}"))
        .len()
    })
}

const RUNS_PER_QUERY: usize = 5;

/// One row of the printed results table.
struct BenchResult {
    row_count: usize,
    pattern: &'static str,
    median_ms: f64,
    hit_count: usize,
}

/// (label, search_terms, sort_field, sort_direction, tag_ids) for one
/// `list_videos_filtered` call under test.
type QueryPattern<'a> = (
    &'static str,
    &'a [String],
    SortField,
    SortDirection,
    &'a [i64],
);

fn run_all_patterns_for(row_count: usize, seed: u64, results: &mut Vec<BenchResult>) {
    let dataset = build_dataset(row_count, seed);
    let pool = &dataset.db.read_pool;

    let no_terms: Vec<String> = Vec::new();
    let short_term = vec![SHORT_TOKEN.to_string()];
    let long_term = vec![LONG_TOKEN.to_string()];
    let no_tags: Vec<i64> = Vec::new();
    let popular_tags = dataset.popular_tag_ids.to_vec();

    let patterns: [QueryPattern; 11] = [
        (
            "(a) no term, sort=CreatedAt desc (browse)",
            &no_terms,
            SortField::CreatedAt,
            SortDirection::Desc,
            &no_tags,
        ),
        (
            "(b) short term \"BD\"",
            &short_term,
            SortField::CreatedAt,
            SortDirection::Desc,
            &no_tags,
        ),
        (
            "(c) long/specific term",
            &long_term,
            SortField::CreatedAt,
            SortDirection::Desc,
            &no_tags,
        ),
        (
            "(d) short term + 2 tags AND",
            &short_term,
            SortField::CreatedAt,
            SortDirection::Desc,
            &popular_tags,
        ),
        (
            "(e) sort=FileName asc",
            &no_terms,
            SortField::FileName,
            SortDirection::Asc,
            &no_tags,
        ),
        (
            "(e) sort=Rating desc",
            &no_terms,
            SortField::Rating,
            SortDirection::Desc,
            &no_tags,
        ),
        (
            "(e) sort=UpdatedDate desc [no index, reference]",
            &no_terms,
            SortField::UpdatedDate,
            SortDirection::Desc,
            &no_tags,
        ),
        (
            "(e) sort=FileSize asc",
            &no_terms,
            SortField::FileSize,
            SortDirection::Asc,
            &no_tags,
        ),
        (
            "(e) sort=FileSize desc",
            &no_terms,
            SortField::FileSize,
            SortDirection::Desc,
            &no_tags,
        ),
        (
            "(e) sort=Folder asc",
            &no_terms,
            SortField::Folder,
            SortDirection::Asc,
            &no_tags,
        ),
        (
            "(e) sort=Folder desc",
            &no_terms,
            SortField::Folder,
            SortDirection::Desc,
            &no_tags,
        ),
    ];

    for (label, terms, field, dir, tags) in patterns {
        let (median_ms, hit_count) =
            measure_median(pool, label, terms, field, dir, tags, RUNS_PER_QUERY);
        results.push(BenchResult {
            row_count,
            pattern: label,
            median_ms,
            hit_count,
        });
    }
}

fn print_results_table(results: &[BenchResult]) {
    println!();
    println!(
        "{:>10}  {:<48}  {:>12}  {:>10}",
        "rows", "pattern", "median(ms)", "hits"
    );
    println!("{}", "-".repeat(10 + 2 + 48 + 2 + 12 + 2 + 10));
    for r in results {
        println!(
            "{:>10}  {:<48}  {:>12.3}  {:>10}",
            r.row_count, r.pattern, r.median_ms, r.hit_count
        );
    }
    println!();
}

#[test]
#[ignore = "manual-only performance benchmark; run with --release --ignored --nocapture"]
fn search_performance_bench_10k_50k_100k() {
    let mut results = Vec::new();
    // Distinct seeds so the three dataset sizes aren't literally prefixes of
    // one another (each is an independently-shaped random library).
    run_all_patterns_for(10_000, 0xA1B2_C3D4_E5F6_0001, &mut results);
    run_all_patterns_for(50_000, 0xA1B2_C3D4_E5F6_0002, &mut results);
    run_all_patterns_for(100_000, 0xA1B2_C3D4_E5F6_0003, &mut results);

    print_results_table(&results);
}

// ---------------------------------------------------------------------------
// Folder-sidebar filter performance
// benchmark. Same dataset/methodology as the benchmark above, but
// exercising `list_videos_filtered`'s `folder_path` argument (a
// boundary-safe `file_path LIKE 'prefix%' ESCAPE '\'`, built by
// `gb_core::paths::folder_like_prefix`) instead of a file-name search term.
//
// `gen_path` (this file's synthetic-path generator) always places one of
// `Pools::folder_names` directly under one of `Pools::drives` as the first
// path segment, so `C:\Videos` is a real, naturally-occurring top-level
// folder in every generated dataset here -- not a contrived filter that
// happens to match nothing.
// ---------------------------------------------------------------------------

const FOLDER_FILTER_TARGET: &str = r"C:\Videos";

/// Prints `EXPLAIN QUERY PLAN` for the exact `file_path LIKE ... ESCAPE '\'`
/// query `list_videos_filtered` runs for a folder filter, so the benchmark
/// output records whether `idx_videos_path` (migration 0001) is actually
/// being used for this query shape, without requiring a human to reproduce
/// the query by hand afterward.
fn print_folder_filter_query_plan(pool: &r2d2::Pool<SqliteConnectionManager>, folder_path: &str) {
    let conn = pool.get().expect("failed to get a pooled connection");
    let pattern = format!("{}%", gb_core::paths::folder_like_prefix(folder_path));
    let mut stmt = conn
        .prepare("EXPLAIN QUERY PLAN SELECT id FROM videos WHERE file_path LIKE ?1 ESCAPE '\\'")
        .unwrap();
    let plan_rows: Vec<String> = stmt
        .query_map(params![pattern], |r| {
            // EXPLAIN QUERY PLAN's columns are (id, parent, notused, detail);
            // `detail` (index 3) is the human-readable line ("SCAN videos
            // USING INDEX idx_videos_path (file_path>? AND file_path<?)" or
            // "SCAN videos" for a full table scan).
            r.get::<_, String>(3)
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    println!("EXPLAIN QUERY PLAN for folder_path={folder_path:?}:");
    for line in &plan_rows {
        println!("  {line}");
    }
}

fn run_folder_filter_bench_for(row_count: usize, seed: u64, results: &mut Vec<BenchResult>) {
    let dataset = build_dataset(row_count, seed);
    let pool = &dataset.db.read_pool;

    print_folder_filter_query_plan(pool, FOLDER_FILTER_TARGET);

    let no_terms: Vec<String> = Vec::new();
    let no_tags: Vec<i64> = Vec::new();
    let label = "folder_path filter (C:\\Videos, sort=CreatedAt desc)";

    let (median_ms, hit_count) = median_of_runs(RUNS_PER_QUERY, || {
        queries::list_videos_filtered(
            pool,
            &no_terms,
            SortField::CreatedAt,
            SortDirection::Desc,
            &no_tags,
            Some(FOLDER_FILTER_TARGET),
            &[],
        )
        .unwrap_or_else(|e| panic!("{label} query failed: {e}"))
        .len()
    });

    results.push(BenchResult {
        row_count,
        pattern: label,
        median_ms,
        hit_count,
    });
}

#[test]
#[ignore = "manual-only performance benchmark; run with --release --ignored --nocapture"]
fn folder_filter_performance_bench_10k_50k_100k() {
    let mut results = Vec::new();
    // Distinct seeds from the other benchmarks in this file, so this
    // benchmark's datasets are independently shaped rather than accidental
    // duplicates.
    run_folder_filter_bench_for(10_000, 0xF01D_E001, &mut results);
    run_folder_filter_bench_for(50_000, 0xF01D_E002, &mut results);
    run_folder_filter_bench_for(100_000, 0xF01D_E003, &mut results);

    print_results_table(&results);
}
