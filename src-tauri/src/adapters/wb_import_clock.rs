//! Real [`WbImportClock`]: local wall time from the OS, durations from a
//! monotonic [`Instant`].

use std::time::Instant;

use gb_core::ports::wb_import_clock::{ClockReading, WbImportClock};

use super::wb_import_log::local_now;

pub struct RealWbImportClock {
    origin: Instant,
}

impl RealWbImportClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for RealWbImportClock {
    fn default() -> Self {
        Self::new()
    }
}

impl WbImportClock for RealWbImportClock {
    fn now(&self) -> ClockReading {
        ClockReading {
            local: local_now(),
            elapsed_ms: u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX),
        }
    }
}
