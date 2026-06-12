// Thin backend test-suite entry for reviewability; behavior is preserved via include! slices.

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

include!("slices/time_and_proxy_basics.rs");
include!("slices/forward_proxy_algo_and_config.rs");
include!("slices/maintenance_and_raw_payload.rs");
include!("slices/upstream_account_group_rules.rs");
include!("slices/broadcast_runtime_and_harness.rs");
include!("slices/proxy_retry_headers_and_model_settings.rs");
include!("slices/timeseries_parallel_and_quota.rs");
include!("slices/invocation_failure_recovery_a.rs");
include!("slices/invocation_failure_recovery_b.rs");
include!("slices/pool_failover_window_a.rs");
include!("slices/pool_failover_window_b.rs");
include!("slices/pool_failover_window_c.rs");
include!("slices/pool_failover_window_d.rs");
include!("slices/pool_failover_window_e.rs");
include!("slices/pool_failover_window_f.rs");
include!("slices/pool_failover_window_g.rs");
include!("slices/pool_failover_window_h.rs");
include!("slices/pool_failover_window_i.rs");
include!("slices/pool_failover_window_j.rs");
include!("slices/pool_failover_window_k.rs");
include!("slices/compact_prompt_cache_attribution.rs");
