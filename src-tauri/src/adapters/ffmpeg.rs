//! Real `FfmpegAdapter`: spawns ffmpeg/ffprobe as PATH binaries via argument
//! arrays -- never a shell string.

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use gb_core::ports::ffmpeg::{FfmpegAdapter, FfmpegAvailability, FfmpegError, VideoMetadata};

use crate::adapters::long_path;

/// `CREATE_NO_WINDOW` (winbase.h): suppresses the console window Windows
/// otherwise auto-allocates for a console subprocess (ffmpeg.exe/ffprobe.exe)
/// spawned from a GUI parent (this app builds with
/// `#![windows_subsystem = "windows"]`). Without this, every ffmpeg/ffprobe
/// invocation flashes a new console window -- at the volume this adapter is
/// called during a large folder scan, that destabilizes the whole desktop.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Builds a `Command` for `program` with `CREATE_NO_WINDOW` set. **All**
/// ffmpeg/ffprobe invocations in this file must go through this helper --
/// see the `no_bare_command_new_for_ffmpeg_or_ffprobe` test below, which
/// guards against a future call site bypassing it.
fn hidden_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Stateless; safe to construct on demand wherever it's needed.
pub struct RealFfmpegAdapter;

impl FfmpegAdapter for RealFfmpegAdapter {
    fn check_available(&self) -> Result<FfmpegAvailability, FfmpegError> {
        Ok(FfmpegAvailability {
            ffmpeg_version: version_of("ffmpeg"),
            ffprobe_version: version_of("ffprobe"),
        })
    }

    fn probe_duration(&self, video_path: &Path) -> Result<Option<f64>, FfmpegError> {
        let long_video_path = long_path::to_long_path(video_path);
        let path_str = path_to_str(&long_video_path)?;
        let output = hidden_command("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "json",
                path_str,
            ])
            .output()
            .map_err(|e| FfmpegError::Spawn(e.to_string()))?;

        if !output.status.success() {
            return Err(FfmpegError::NonZeroExit {
                status: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }

        let json: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| FfmpegError::ParseError(e.to_string()))?;
        let duration = json
            .get("format")
            .and_then(|f| f.get("duration"))
            .and_then(|d| d.as_str())
            .and_then(|s| s.parse::<f64>().ok());
        Ok(duration)
    }

    fn extract_thumbnail(
        &self,
        video_path: &Path,
        output_tmp_path: &Path,
        seek_secs: f64,
        width_px: u32,
        quality: u8,
    ) -> Result<(), FfmpegError> {
        let long_input = long_path::to_long_path(video_path);
        let long_output = long_path::to_long_path(output_tmp_path);
        let input_str = path_to_str(&long_input)?;
        let output_str = path_to_str(&long_output)?;

        // Extracts one frame at the ~10% position (already computed by the
        // caller as seek_secs), scaled to about width_px wide, low-quality-
        // leaning WebP output via -quality/-lossless 0.
        let output = hidden_command("ffmpeg")
            .args([
                "-ss",
                &seek_secs.to_string(),
                "-i",
                input_str,
                "-frames:v",
                "1",
                "-vf",
                &format!("scale={width_px}:-1"),
                "-quality",
                &quality.to_string(),
                "-lossless",
                "0",
                // Output path is `[id].webp.tmp` (this app's atomic
                // write scheme), so ffmpeg can't infer the muxer from the
                // extension (it sees ".tmp", not ".webp") -- force it explicitly.
                // Confirmed via manual repro: without this, ffmpeg refuses to
                // open the output ("Unable to choose an output format").
                "-f",
                "webp",
                "-y",
                output_str,
            ])
            .output()
            .map_err(|e| FfmpegError::Spawn(e.to_string()))?;

        if !output.status.success() {
            return Err(FfmpegError::NonZeroExit {
                status: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(())
    }

    fn probe_metadata(&self, video_path: &Path) -> Result<VideoMetadata, FfmpegError> {
        let long_video_path = long_path::to_long_path(video_path);
        let path_str = path_to_str(&long_video_path)?;
        let output = hidden_command("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type,codec_name,width,height,r_frame_rate,bit_rate:format=bit_rate",
                "-of",
                "json",
                path_str,
            ])
            .output()
            .map_err(|e| FfmpegError::Spawn(e.to_string()))?;

        if !output.status.success() {
            return Err(FfmpegError::NonZeroExit {
                status: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }

        let json: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| FfmpegError::ParseError(e.to_string()))?;

        let streams = json.get("streams").and_then(|s| s.as_array());
        let video_stream = streams.and_then(|arr| {
            arr.iter()
                .find(|s| s.get("codec_type").and_then(|c| c.as_str()) == Some("video"))
        });
        let audio_stream = streams.and_then(|arr| {
            arr.iter()
                .find(|s| s.get("codec_type").and_then(|c| c.as_str()) == Some("audio"))
        });

        let width = video_stream
            .and_then(|s| s.get("width"))
            .and_then(|w| w.as_u64())
            .map(|w| w as u32);
        let height = video_stream
            .and_then(|s| s.get("height"))
            .and_then(|h| h.as_u64())
            .map(|h| h as u32);
        let video_codec = video_stream
            .and_then(|s| s.get("codec_name"))
            .and_then(|c| c.as_str())
            .map(String::from);
        let audio_codec = audio_stream
            .and_then(|s| s.get("codec_name"))
            .and_then(|c| c.as_str())
            .map(String::from);
        let fps = video_stream
            .and_then(|s| s.get("r_frame_rate"))
            .and_then(|r| r.as_str())
            .and_then(parse_frame_rate);

        // Falls back to the container-level figure when the video stream
        // doesn't report its own bit_rate -- common for some containers.
        let stream_bitrate = video_stream
            .and_then(|s| s.get("bit_rate"))
            .and_then(|b| b.as_str())
            .and_then(|s| s.parse::<i64>().ok());
        let format_bitrate = json
            .get("format")
            .and_then(|f| f.get("bit_rate"))
            .and_then(|b| b.as_str())
            .and_then(|s| s.parse::<i64>().ok());

        Ok(VideoMetadata {
            width,
            height,
            video_codec,
            audio_codec,
            bitrate: stream_bitrate.or(format_bitrate),
            fps,
        })
    }

    fn convert_image_to_webp(
        &self,
        src_path: &Path,
        output_tmp_path: &Path,
        quality: u8,
    ) -> Result<(), FfmpegError> {
        let long_input = long_path::to_long_path(src_path);
        let long_output = long_path::to_long_path(output_tmp_path);
        let input_str = path_to_str(&long_input)?;
        let output_str = path_to_str(&long_output)?;

        // Input is always `.jpg` (legacy WhiteBrowser thumbnail naming),
        // so ffmpeg can infer the demuxer from the
        // extension -- no input `-f` needed. The output, like
        // `extract_thumbnail`'s, is a `[id].webp.tmp` path (atomic
        // write scheme), so its extension can't tell ffmpeg the muxer;
        // force it explicitly for the same reason documented there.
        let output = hidden_command("ffmpeg")
            .args([
                "-i",
                input_str,
                "-quality",
                &quality.to_string(),
                "-lossless",
                "0",
                "-f",
                "webp",
                "-y",
                output_str,
            ])
            .output()
            .map_err(|e| FfmpegError::Spawn(e.to_string()))?;

        if !output.status.success() {
            return Err(FfmpegError::NonZeroExit {
                status: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(())
    }
}

/// Parses ffprobe's `r_frame_rate` fraction string (e.g. `"30000/1001"`) into
/// a decimal fps value. `None` for malformed input or a zero denominator
/// (ffprobe reports `"0/0"` for streams with no meaningful frame rate).
fn parse_frame_rate(raw: &str) -> Option<f64> {
    let (num, den) = raw.split_once('/')?;
    let num: f64 = num.parse().ok()?;
    let den: f64 = den.parse().ok()?;
    if den == 0.0 {
        return None;
    }
    Some(num / den)
}

fn path_to_str(path: &Path) -> Result<&str, FfmpegError> {
    path.to_str()
        .ok_or_else(|| FfmpegError::Spawn(format!("path is not valid UTF-8: {}", path.display())))
}

/// Runs `<binary_name> -version` and returns the first line of stdout if the
/// binary was found and ran successfully; `None` otherwise (not found,
/// failed to spawn, or non-zero exit).
fn version_of(binary_name: &str) -> Option<String> {
    let output = hidden_command(binary_name).arg("-version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(|line| line.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frame_rate_handles_a_fractional_ntsc_rate() {
        assert_eq!(parse_frame_rate("30000/1001"), Some(30000.0 / 1001.0));
    }

    #[test]
    fn parse_frame_rate_handles_a_whole_number_rate() {
        assert_eq!(parse_frame_rate("25/1"), Some(25.0));
    }

    #[test]
    fn parse_frame_rate_rejects_a_zero_denominator() {
        assert_eq!(parse_frame_rate("0/0"), None);
    }

    #[test]
    fn parse_frame_rate_rejects_malformed_input() {
        assert_eq!(parse_frame_rate("not-a-fraction"), None);
        assert_eq!(parse_frame_rate(""), None);
        assert_eq!(parse_frame_rate("30"), None);
    }

    /// Structural regression guard: every ffmpeg/ffprobe spawn
    /// in this file must go through `hidden_command()` (which sets
    /// `CREATE_NO_WINDOW`), never a bare `Command::new("ffmpeg"/"ffprobe")`.
    /// The needle is assembled at runtime from two pieces rather than
    /// written as one literal, so this test doesn't trivially self-match its
    /// own source text when scanning the file.
    #[test]
    fn no_bare_command_new_for_ffmpeg_or_ffprobe() {
        let source = include_str!("ffmpeg.rs");
        let open = "Command::new(";
        for bin in ["ffmpeg", "ffprobe"] {
            let bare_call = format!("{open}\"{bin}\")");
            assert!(
                !source.contains(&bare_call),
                "found a direct Command::new(\"{bin}\") call in ffmpeg.rs that bypasses \
                 hidden_command() -- every ffmpeg/ffprobe spawn must set CREATE_NO_WINDOW"
            );
        }
    }

    /// Behavioral regression guard: confirms `CREATE_NO_WINDOW`
    /// actually suppresses the console Windows would otherwise create for a
    /// console-subsystem child process spawned from this (GUI subsystem)
    /// app. `std::process::Command` has no getter for `creation_flags`
    /// (`CommandExt` is setter-only), so this can't be asserted
    /// structurally -- instead it spawns `console_probe` (a tiny test-only
    /// helper binary, `src/bin/console_probe.rs`) via `hidden_command()` and
    /// reads back, over a plain stdout pipe, whether *that process itself*
    /// sees a console via its own `GetConsoleWindow()` call.
    ///
    /// This self-reporting design exists because two more direct approaches
    /// -- discovered via mutation testing, which
    /// caught both of them passing identically whether or not
    /// `CREATE_NO_WINDOW` was actually applied -- turned out to be
    /// unreliable in this environment:
    ///
    /// 1. Enumerating top-level windows and matching by the child's PID
    ///    doesn't work because a console window's owning process (as
    ///    `GetWindowThreadProcessId` reports it) is a separate `conhost.exe`,
    ///    never the console application itself.
    /// 2. Probing from the test harness with `AttachConsole(child_pid)`
    ///    consistently failed (`ERROR_INVALID_HANDLE`) in this sandboxed dev
    ///    environment even for children confirmed still running, for
    ///    reasons that weren't fully diagnosable (plausibly related to how
    ///    this environment's terminal/console host is set up) -- it may work
    ///    on a plain interactive desktop, but the self-reporting approach
    ///    avoids the question entirely.
    ///
    /// One condition to reproduce faithfully either way: Windows only
    /// auto-allocates a *new* console for a console-subsystem child when the
    /// *spawning* process itself has no console -- when the parent already
    /// has one (as `cargo test` does, attached to the terminal), a plain
    /// child just silently attaches to it regardless of `CREATE_NO_WINDOW`.
    /// The real app never has a console at all (it builds with
    /// `windows_subsystem = "windows"`), so this test detaches itself from
    /// its console with `FreeConsole()` before spawning, then reattaches
    /// afterward so it doesn't disrupt the rest of the test run.
    #[cfg(windows)]
    #[test]
    fn hidden_command_suppresses_the_child_console() {
        use windows_sys::Win32::System::Console::{
            AttachConsole, FreeConsole, ATTACH_PARENT_PROCESS,
        };

        // SAFETY: `FreeConsole` takes no arguments/pointers and is safe to
        // call unconditionally; it just detaches this process from whatever
        // console it currently has (if any).
        let had_console = unsafe { FreeConsole() } != 0;

        // Run the actual probe inside `catch_unwind` so a failed assertion
        // (or an unexpected panic) still lets us reattach the console
        // below, instead of leaving this test process permanently
        // console-less for every other test still running in this binary.
        let probe_result = std::panic::catch_unwind(|| {
            let probe_exe = console_probe_exe_path();
            let output = hidden_command(probe_exe.to_str().expect("probe exe path is UTF-8"))
                .output()
                .expect("failed to run console_probe (build it with the `testing` feature)");
            assert!(
                output.status.success(),
                "console_probe exited non-zero: {:?}",
                output.status
            );
            String::from_utf8(output.stdout)
                .expect("console_probe's stdout is UTF-8")
                .trim()
                .to_string()
        });

        if had_console {
            // SAFETY: `AttachConsole` takes no pointers; `ATTACH_PARENT_PROCESS`
            // re-finds and reattaches to this process's original console (the
            // one the test binary was launched from), undoing `FreeConsole()`
            // above.
            unsafe {
                AttachConsole(ATTACH_PARENT_PROCESS);
            }
        }

        let report = match probe_result {
            Ok(report) => report,
            Err(payload) => std::panic::resume_unwind(payload),
        };

        assert_eq!(
            report, "no-console",
            "hidden_command()'s child process reports it has its own console \
             (console_probe printed {report:?}) -- CREATE_NO_WINDOW does not \
             appear to be taking effect"
        );
    }

    /// Locates the `console_probe` helper binary built alongside this test
    /// binary. Cargo's `CARGO_BIN_EXE_<name>` env var is only populated for
    /// integration tests/benchmarks, not for a library's own inline
    /// `#[cfg(test)]` unit tests (which is what this is), so this instead
    /// derives the path from the currently-running test binary's own
    /// location: unit test binaries land in `target/<profile>/deps/`, and
    /// `[[bin]]` targets in the same package land one directory up, in
    /// `target/<profile>/`.
    #[cfg(windows)]
    fn console_probe_exe_path() -> std::path::PathBuf {
        let current_exe = std::env::current_exe().expect("could not determine current_exe()");
        let deps_dir = current_exe
            .parent()
            .expect("current_exe() has a parent directory");
        let profile_dir = deps_dir
            .parent()
            .expect("deps/ has a parent directory (the profile dir)");
        let candidate = profile_dir.join("console_probe.exe");
        assert!(
            candidate.is_file(),
            "expected console_probe.exe at {candidate:?} (built via the `testing` feature, \
             which `cargo test` enables automatically for this crate) -- got current_exe {current_exe:?}"
        );
        candidate
    }
}
