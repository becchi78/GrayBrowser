use std::collections::VecDeque;
use std::sync::Mutex;

use crate::ports::log_source::{LogChunk, LogReadError, LogSource};

/// Replays scripted results in order and records the arguments of each call.
/// When the script runs out it keeps answering with an empty chunk at the
/// last offset it returned, like a log that stopped growing.
#[derive(Default)]
pub struct FakeLogSource {
    script: Mutex<VecDeque<Result<LogChunk, LogReadError>>>,
    last_offset: Mutex<u64>,
    pub calls: Mutex<Vec<(Option<u64>, u64)>>,
}

impl FakeLogSource {
    pub fn with_script(script: Vec<Result<LogChunk, LogReadError>>) -> Self {
        Self {
            script: Mutex::new(script.into()),
            ..Self::default()
        }
    }
}

impl LogSource for FakeLogSource {
    fn read_from(&self, offset: Option<u64>, max_bytes: u64) -> Result<LogChunk, LogReadError> {
        self.calls.lock().unwrap().push((offset, max_bytes));
        let next = self.script.lock().unwrap().pop_front();
        match next {
            Some(Ok(chunk)) => {
                *self.last_offset.lock().unwrap() = chunk.next_offset;
                Ok(chunk)
            }
            Some(Err(e)) => Err(e),
            None => Ok(LogChunk {
                text: String::new(),
                next_offset: *self.last_offset.lock().unwrap(),
                reset: false,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replays_the_script_then_idles_and_records_calls() {
        let fake = FakeLogSource::with_script(vec![
            Ok(LogChunk {
                text: "a\n".into(),
                next_offset: 2,
                reset: false,
            }),
            Err(LogReadError::NotFound),
        ]);
        assert_eq!(fake.read_from(None, 10).unwrap().text, "a\n");
        assert_eq!(fake.read_from(Some(2), 10), Err(LogReadError::NotFound));
        let idle = fake.read_from(Some(2), 10).unwrap();
        assert_eq!((idle.text.as_str(), idle.next_offset), ("", 2));
        assert_eq!(
            *fake.calls.lock().unwrap(),
            vec![(None, 10), (Some(2), 10), (Some(2), 10)]
        );
    }
}
