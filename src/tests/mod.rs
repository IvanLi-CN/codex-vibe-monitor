// Backend test-suite entry grouped by resource profile; behavior is preserved via real modules.
#![allow(unused_imports)]

use super::*;

mod archive_file_io;
mod lightweight;
mod stateful_sqlite;
mod support;

pub(crate) use lightweight::*;
pub(crate) use stateful_sqlite::parallel_work_stats_and_timeseries::part_01::{
    SeedInvocationArchiveBatchRow, assert_f64_close, seed_invocation_archive_batch,
    seed_invocation_archive_batch_with_details,
};
pub(crate) use stateful_sqlite::system_status_and_account_roster::part_02::test_state_with_openai_base_and_pool_no_available_wait;
pub(crate) use stateful_sqlite::system_status_and_account_roster::part_03::{
    insert_test_pool_oauth_account, seed_pool_routing_api_key,
};
pub(crate) use stateful_sqlite::*;
pub(crate) use support::*;

#[tokio::test]
async fn prepare_current_schema_template_for_stateful_profile() {
    let path = std::env::var_os(stateful_sqlite::STATEFUL_SCHEMA_TEMPLATE_PATH_ENV)
        .or_else(|| std::env::var_os(stateful_sqlite::ARCHIVE_SCHEMA_TEMPLATE_PATH_ENV));
    let Some(path) = path else {
        return;
    };
    let path = std::path::PathBuf::from(path);
    stateful_sqlite::write_stateful_schema_template(&path)
        .await
        .expect("prepare stateful profile schema template");
}

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
#[derive(Default)]
struct PoolAccountWaitOptions<'a> {
    required_upstream_route_key: Option<&'a str>,
    wait_for_no_available: bool,
    wait_deadline: Option<std::time::Instant>,
    total_timeout_deadline: Option<std::time::Instant>,
}

async fn resolve_pool_account_for_request_with_wait(
    state: &crate::AppState,
    sticky_key: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &std::collections::HashSet<String>,
    options: &mut PoolAccountWaitOptions<'_>,
) -> anyhow::Result<crate::proxy::PoolAccountResolutionWithWait> {
    crate::proxy::resolve_pool_account_for_request_with_wait(
        state,
        sticky_key,
        None,
        excluded_ids,
        excluded_upstream_route_keys,
        options.required_upstream_route_key,
        options.wait_for_no_available,
        &mut options.wait_deadline,
        options.total_timeout_deadline,
    )
    .await
}
