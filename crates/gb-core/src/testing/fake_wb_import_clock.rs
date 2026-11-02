use std::sync::Mutex;

use crate::ports::wb_import_clock::{ClockReading, WbImportClock};
use crate::wb_import_log::DateTimeParts;

/// Returns the given readings one per call, in order; once they run out the
/// last one repeats. Lets a test pin the exact time printed at each point of
/// the import without sleeping.
pub struct FakeWbImportClock {
    readings: Vec<ClockReading>,
    next: Mutex<usize>,
}

impl FakeWbImportClock {
    /// Panics on an empty list: a clock with nothing to say has no sensible
    /// behavior to fake.
    pub fn new(readings: Vec<ClockReading>) -> Self {
        assert!(!readings.is_empty(), "FakeWbImportClock needs a reading");
        Self {
            readings,
            next: Mutex::new(0),
        }
    }

    /// A reading at `2026-11-02 12:00:00` plus `offset_secs`, on a monotonic
    /// counter that has also advanced `offset_secs`.
    pub fn reading_at(offset_secs: u32) -> ClockReading {
        ClockReading {
            local: DateTimeParts {
                year: 2026,
                month: 11,
                day: 2,
                hour: 12 + offset_secs / 3_600,
                minute: offset_secs % 3_600 / 60,
                second: offset_secs % 60,
            },
            elapsed_ms: u64::from(offset_secs) * 1_000,
        }
    }

    /// Readings at each of `offsets_secs` (see [`Self::reading_at`]).
    pub fn at_offsets(offsets_secs: &[u32]) -> Self {
        Self::new(offsets_secs.iter().map(|o| Self::reading_at(*o)).collect())
    }

    /// How many times the clock has been read.
    pub fn calls(&self) -> usize {
        *self.next.lock().unwrap()
    }
}

impl Default for FakeWbImportClock {
    /// Time stands still at `2026-11-02 12:00:00`.
    fn default() -> Self {
        Self::at_offsets(&[0])
    }
}

impl WbImportClock for FakeWbImportClock {
    fn now(&self) -> ClockReading {
        let mut next = self.next.lock().unwrap();
        let reading = self.readings[(*next).min(self.readings.len() - 1)];
        *next += 1;
        reading
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readings_come_in_order_then_the_last_repeats() {
        let clock = FakeWbImportClock::at_offsets(&[0, 65]);
        assert_eq!(clock.now().elapsed_ms, 0);
        let second = clock.now();
        assert_eq!(second.elapsed_ms, 65_000);
        assert_eq!((second.local.minute, second.local.second), (1, 5));
        assert_eq!(clock.now(), second);
        assert_eq!(clock.calls(), 3);
    }
}
