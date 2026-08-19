use std::sync::Mutex;

use crate::ports::wb_source::{WbMovieRow, WbSourceAdapter, WbSourceError, WbTextCell};

#[derive(Debug, Clone, PartialEq)]
pub enum FakeCall {
    ReadMovies,
    ReadAllTextCells,
}

pub struct FakeWbSourceAdapter {
    pub movies: Result<Vec<WbMovieRow>, WbSourceError>,
    pub text_cells: Result<Vec<WbTextCell>, WbSourceError>,
    pub calls: Mutex<Vec<FakeCall>>,
}

impl Default for FakeWbSourceAdapter {
    fn default() -> Self {
        let not_configured = || {
            WbSourceError::Query(
                "FakeWbSourceAdapter: no canned value configured for this test".into(),
            )
        };
        Self {
            movies: Err(not_configured()),
            text_cells: Err(not_configured()),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl WbSourceAdapter for FakeWbSourceAdapter {
    fn read_movies(&self) -> Result<Vec<WbMovieRow>, WbSourceError> {
        self.calls.lock().unwrap().push(FakeCall::ReadMovies);
        self.movies.clone()
    }

    fn read_all_text_cells(&self) -> Result<Vec<WbTextCell>, WbSourceError> {
        self.calls.lock().unwrap().push(FakeCall::ReadAllTextCells);
        self.text_cells.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_the_canned_movies_and_records_the_call() {
        let fake = FakeWbSourceAdapter {
            movies: Ok(vec![]),
            ..Default::default()
        };
        assert_eq!(fake.read_movies().unwrap(), vec![]);
        assert_eq!(
            fake.calls.lock().unwrap().as_slice(),
            [FakeCall::ReadMovies]
        );
    }

    #[test]
    fn default_is_a_safe_failure_for_every_method() {
        let fake = FakeWbSourceAdapter::default();
        assert!(fake.read_movies().is_err());
        assert!(fake.read_all_text_cells().is_err());
    }
}
