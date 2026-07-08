// Thin backend test-suite entry for reviewability; behavior is preserved via include! slices.

use super::*;

mod slices;

#[cfg(test)]
async fn resolve_pool_account_for_request(
    state: &crate::AppState,
    sticky_key: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &std::collections::HashSet<String>,
) -> anyhow::Result<crate::upstream_accounts::PoolAccountResolution> {
    crate::upstream_accounts::resolve_pool_account_for_request(
        state,
        sticky_key,
        None,
        excluded_ids,
        excluded_upstream_route_keys,
    )
    .await
}

#[cfg(test)]
async fn resolve_pool_account_for_request_with_wait(
    state: &crate::AppState,
    sticky_key: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &std::collections::HashSet<String>,
    required_upstream_route_key: Option<&str>,
    wait_for_no_available: bool,
    wait_deadline: &mut Option<std::time::Instant>,
    total_timeout_deadline: Option<std::time::Instant>,
) -> anyhow::Result<crate::proxy::PoolAccountResolutionWithWait> {
    crate::proxy::resolve_pool_account_for_request_with_wait(
        state,
        sticky_key,
        None,
        excluded_ids,
        excluded_upstream_route_keys,
        required_upstream_route_key,
        wait_for_no_available,
        wait_deadline,
        total_timeout_deadline,
    )
    .await
}
