// Backend test-suite entry grouped by resource profile; behavior is preserved via real modules.

pub(crate) use super::*;

mod archive_file_io;
mod lightweight;
mod stateful_sqlite;
mod support;

pub(crate) use lightweight::*;
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
pub(crate) struct PoolAccountWaitOptions<'a> {
    pub(crate) sticky_key: Option<&'a str>,
    pub(crate) requested_model: Option<&'a str>,
    pub(crate) excluded_ids: &'a [i64],
    pub(crate) excluded_upstream_route_keys: &'a std::collections::HashSet<String>,
    pub(crate) required_upstream_route_key: Option<&'a str>,
    pub(crate) wait_for_no_available: bool,
    pub(crate) wait_deadline: &'a mut Option<std::time::Instant>,
    pub(crate) total_timeout_deadline: Option<std::time::Instant>,
}

#[cfg(test)]
pub(crate) async fn resolve_pool_account_for_request_with_wait(
    state: &crate::AppState,
    options: PoolAccountWaitOptions<'_>,
) -> anyhow::Result<crate::proxy::PoolAccountResolutionWithWait> {
    crate::proxy::resolve_pool_account_for_request_with_wait(
        state,
        options.sticky_key,
        options.requested_model,
        options.excluded_ids,
        options.excluded_upstream_route_keys,
        options.required_upstream_route_key,
        options.wait_for_no_available,
        options.wait_deadline,
        options.total_timeout_deadline,
    )
    .await
}
