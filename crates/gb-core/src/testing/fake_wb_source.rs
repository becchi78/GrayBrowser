use crate::ports::wb_source::{WbMovieRow, WbSourceAdapter, WbSourceError};

pub struct FakeWbSourceAdapter {
    pub movies: Result<Vec<WbMovieRow>, WbSourceError>,
    /// Returned for every key, whatever was asked.
    pub profile_values: Result<Vec<String>, WbSourceError>,
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
            profile_values: Err(not_configured()),
        }
    }
}

impl WbSourceAdapter for FakeWbSourceAdapter {
    fn read_movies(&self) -> Result<Vec<WbMovieRow>, WbSourceError> {
        self.movies.clone()
    }

    fn read_profile_value(&self, _key: &str) -> Result<Vec<String>, WbSourceError> {
        self.profile_values.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_the_canned_movies() {
        let fake = FakeWbSourceAdapter {
            movies: Ok(vec![]),
            ..Default::default()
        };
        assert_eq!(fake.read_movies().unwrap(), vec![]);
    }

    #[test]
    fn default_is_a_safe_failure_for_every_method() {
        let fake = FakeWbSourceAdapter::default();
        assert!(fake.read_movies().is_err());
        assert!(fake.read_profile_value("thum_size").is_err());
    }
}
