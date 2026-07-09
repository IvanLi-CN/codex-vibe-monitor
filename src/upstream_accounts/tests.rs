use super::super::*;
use super::*;
use chrono::Timelike;

mod archive_file_io;
mod lightweight;
mod stateful_sqlite;
mod support;

pub(crate) use archive_file_io::*;
pub(crate) use lightweight::*;
pub(crate) use stateful_sqlite::*;
pub(crate) use support::*;

async fn resolve_pool_account_for_request(
    state: &AppState,
    sticky_key: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &std::collections::HashSet<String>,
) -> Result<PoolAccountResolution> {
    super::resolve_pool_account_for_request(
        state,
        sticky_key,
        None,
        excluded_ids,
        excluded_upstream_route_keys,
    )
    .await
}

async fn resolve_pool_account_for_request_with_binding_constraint(
    state: &AppState,
    sticky_key: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &std::collections::HashSet<String>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
) -> Result<PoolAccountResolution> {
    super::resolve_pool_account_for_request_with_binding_constraint(
        state,
        sticky_key,
        None,
        excluded_ids,
        excluded_upstream_route_keys,
        binding_constraint,
    )
    .await
}

async fn resolve_pool_account_for_request_with_binding_constraint_and_model(
    state: &AppState,
    sticky_key: Option<&str>,
    requested_model: Option<&str>,
    excluded_ids: &[i64],
    excluded_upstream_route_keys: &std::collections::HashSet<String>,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
) -> Result<PoolAccountResolution> {
    super::resolve_pool_account_for_request_with_route_requirement(
        state,
        sticky_key,
        requested_model,
        excluded_ids,
        excluded_upstream_route_keys,
        None,
        binding_constraint,
    )
    .await
}
