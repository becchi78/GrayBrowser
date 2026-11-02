//! Clock the `.wb` import reads to timestamp its log and measure how long each
//! stage took. A port so the log text can be tested with fixed times; the
//! real implementation (in `src-tauri::adapters::wb_import_clock`) asks the OS.

use crate::wb_import_log::DateTimeParts;

/// One look at the clock: the wall time to print and a monotonic counter to
/// subtract. Both come from the same call so a printed time and a measured
/// span can never disagree about which instant they describe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockReading {
    /// Local time of day, for people reading the log.
    pub local: DateTimeParts,
    /// Milliseconds on a monotonic clock from an arbitrary origin. Durations
    /// are taken from this and not from `local`, so a clock adjustment or a
    /// daylight-saving change during a long import cannot skew them.
    pub elapsed_ms: u64,
}

pub trait WbImportClock: Send + Sync {
    fn now(&self) -> ClockReading;
}
