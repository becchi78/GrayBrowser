use crate::ports::release_source::ReleaseSource;

/// A release list with a canned response body, or one that cannot be fetched.
pub struct FakeReleaseSource {
    pub response: Result<String, String>,
}

impl FakeReleaseSource {
    pub fn with_body(body: &str) -> Self {
        Self {
            response: Ok(body.to_string()),
        }
    }

    pub fn failing(message: &str) -> Self {
        Self {
            response: Err(message.to_string()),
        }
    }
}

impl ReleaseSource for FakeReleaseSource {
    fn fetch_releases_json(&self) -> Result<String, String> {
        self.response.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_the_canned_answer() {
        assert_eq!(
            FakeReleaseSource::with_body("[]").fetch_releases_json(),
            Ok("[]".to_string())
        );
        assert!(FakeReleaseSource::failing("offline")
            .fetch_releases_json()
            .is_err());
    }
}
