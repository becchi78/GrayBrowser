use tauri::State;

use crate::reachability::ReachabilityRegistry;

/// The watch folders known to be unreachable right now. The frontend reads
/// this once on mount (events emitted before it subscribed are lost) and then
/// follows `reachability:changed`.
///
/// Reads memory only -- it never probes -- so it cannot block on a dead share.
pub fn current_unreachable_folders(registry: &ReachabilityRegistry) -> Vec<String> {
    registry.unreachable_folders()
}

#[tauri::command]
pub fn get_unreachable_folders(registry: State<ReachabilityRegistry>) -> Vec<String> {
    current_unreachable_folders(&registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::NoopReachabilityNotifier;
    use gb_core::testing::fake_folder_probe::FakeFolderProbe;
    use std::sync::Arc;

    #[test]
    fn reports_only_folders_found_unreachable_and_never_probes() {
        let probe = Arc::new(FakeFolderProbe::with_unreachable(&["W:/video"]));
        let registry = ReachabilityRegistry::new(probe.clone(), Arc::new(NoopReachabilityNotifier));
        assert!(current_unreachable_folders(&registry).is_empty());
        assert!(probe.calls().is_empty());

        registry.is_reachable("W:/video");
        registry.is_reachable("C:/videos");

        assert_eq!(
            current_unreachable_folders(&registry),
            vec!["W:/video".to_string()]
        );
    }
}
