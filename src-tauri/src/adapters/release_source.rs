//! Real `ReleaseSource`: GitHub's "list releases" API for this repository.
//! The URL is a compile-time constant; nothing from the frontend reaches it.

use std::time::Duration;

use gb_core::ports::release_source::ReleaseSource;
use gb_core::version_pin::describe_fetch_failure;

/// `per_page=100` so the list is not cut at the API's default of 30; a project
/// with more releases than that would need paging, which is not done here.
const RELEASES_URL: &str =
    "https://api.github.com/repos/becchi78/GrayBrowser/releases?per_page=100";

/// Without a bound a stalled connection would leave the dialog on "loading".
const TIMEOUT: Duration = Duration::from_secs(20);

pub struct GithubReleaseSource;

impl ReleaseSource for GithubReleaseSource {
    /// Blocking: call from a blocking worker, not from the async runtime.
    fn fetch_releases_json(&self) -> Result<String, String> {
        // The updater installs the provider lazily, on its first check; a
        // client built before that would panic without one.
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            // Only fails when another thread installed one in the meantime.
            let _ = rustls::crypto::ring::default_provider().install_default();
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(TIMEOUT)
            // The API rejects requests without a User-Agent.
            .user_agent(concat!("GrayBrowser/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| format!("通信の準備に失敗しました。({e})"))?;
        let response = client
            .get(RELEASES_URL)
            .header("Accept", "application/vnd.github+json")
            .send()
            .map_err(|e| format!("リリース一覧を取得できませんでした。({e})"))?;
        let status = response.status();
        if !status.is_success() {
            let remaining = response
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|v| v.to_str().ok());
            return Err(describe_fetch_failure(status.as_u16(), remaining));
        }
        response
            .text()
            .map_err(|e| format!("リリース一覧を読み取れませんでした。({e})"))
    }
}
