//! Source of the published release list. The trait is network-independent; the
//! real implementation (in `src-tauri::adapters::release_source`) asks GitHub.

pub trait ReleaseSource: Send + Sync {
    /// Body of the "list releases" response, unparsed: filtering the list is
    /// pure logic (`version_pin::parse_installable_releases`) and is tested
    /// against canned bodies. The error text is shown to the user.
    fn fetch_releases_json(&self) -> Result<String, String>;
}
