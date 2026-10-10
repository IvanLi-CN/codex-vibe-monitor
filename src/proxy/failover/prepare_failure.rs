use super::*;

pub(super) async fn record_pool_request_prepare_failure_attempt(
    state: &AppState,
    trace_context: Option<&PoolUpstreamAttemptTraceContext>,
    account: &PoolResolvedAccount,
    requested_model: Option<&str>,
    model_mapping_pattern: Option<&str>,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    status: StatusCode,
    message: &str,
) {
    let Some(trace) = trace_context else {
        return;
    };
    let mut attempt_trace = trace.clone();
    if attempt_trace.request_model.is_none() {
        attempt_trace.request_model = requested_model.map(ToOwned::to_owned);
    }
    attempt_trace.upstream_base_url_host = account
        .upstream_base_url
        .host_str()
        .and_then(normalize_upstream_base_url_host_value);
    let group_name_snapshot = normalize_pool_attempt_group_name(account.group_name.clone());
    let upstream_route_key = account.upstream_route_key();
    let started_at = format_naive_precise(Utc::now().with_timezone(&Shanghai).naive_local());
    let mut pending = begin_pool_upstream_request_attempt_with_scope_and_routing_source_and_audit(
        &state.pool,
        &attempt_trace,
        group_name_snapshot.as_deref(),
        None,
        Some(account.routing_source),
        account.routing_selection_audit.as_ref(),
        account.account_id,
        &upstream_route_key,
        attempt_index,
        distinct_account_index,
        same_account_retry_index,
        &started_at,
    )
    .await;
    if let Err(err) = annotate_pool_upstream_request_attempt_model_mapping(
        &mut pending,
        None,
        model_mapping_pattern,
    ) {
        warn!(
            invoke_id = %pending.invoke_id,
            error = %err,
            "failed to persist pre-send pool model mapping metadata"
        );
    }
    let finished_at = shanghai_now_string();
    if let Err(err) = finalize_pool_upstream_request_attempt(
        &state.pool,
        &pending,
        &finished_at,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        Some(status),
        None,
        Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM),
        Some(message),
        None,
        Some(0.0),
        None,
        None,
        None,
        None,
        None,
    )
    .await
    {
        warn!(
            invoke_id = %pending.invoke_id,
            error = %err,
            "failed to persist pre-send pool attempt"
        );
    }
    if let Err(err) = broadcast_pool_upstream_attempts_snapshot(state, &pending.invoke_id).await {
        warn!(
            invoke_id = %pending.invoke_id,
            error = %err,
            "failed to broadcast pre-send pool attempt snapshot"
        );
    }
}
