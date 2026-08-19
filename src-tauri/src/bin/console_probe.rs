//! Test-only helper binary for the `hidden_command_suppresses_the_child_console`
//! regression test in `src/adapters/ffmpeg.rs`.
//!
//! Probing "does the process I spawned end up with its own console window?"
//! from *outside* that process (via `EnumWindows`/`AttachConsole` from the
//! test harness) turned out to be unreliable in practice -- see that test's
//! doc comment for the two dead ends this ran into. This binary instead lets
//! the spawned child self-report: it just calls `GetConsoleWindow()` on
//! itself (a plain per-process API query, no window-station/EnumWindows
//! involvement) and prints whether it got a console at all. The test harness
//! spawns this via `hidden_command()` and reads the answer back over a
//! regular stdout pipe.
//!
//! Gated behind the `testing` Cargo feature (see `[[bin]]` entry in
//! `Cargo.toml`) so it's never built as part of a release/`tauri build`.

fn main() {
    #[cfg(windows)]
    {
        // SAFETY: `GetConsoleWindow` takes no arguments and is safe to call
        // unconditionally; it just reports this process's own console
        // window handle (or null if it has none).
        let hwnd = unsafe { windows_sys::Win32::System::Console::GetConsoleWindow() };
        println!(
            "{}",
            if hwnd.is_null() {
                "no-console"
            } else {
                "has-console"
            }
        );
    }
    #[cfg(not(windows))]
    {
        println!("not-windows");
    }
}
