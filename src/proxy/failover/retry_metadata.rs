use super::*;

pub(crate) type PoolSelectedForwardProxy = (
    ForwardProxyRouteScope,
    SelectedForwardProxy,
    reqwest::Client,
);

pub(crate) fn no_candidate_next_eligible_delay(
    audit: &PoolRoutingNoCandidateAudit,
) -> Option<Duration> {
    const MIN_STALE_NEXT_ELIGIBLE_RESELECT_DELAY: Duration = Duration::from_millis(25);

    audit
        .next_eligible_at
        .as_deref()
        .and_then(parse_to_utc_datetime)
        .map(|eligible_at| {
            (eligible_at - Utc::now())
                .to_std()
                .unwrap_or(Duration::ZERO)
                .max(MIN_STALE_NEXT_ELIGIBLE_RESELECT_DELAY)
        })
}

pub(crate) fn parse_retry_after_delay(value: &HeaderValue) -> Option<Duration> {
    let text = value.to_str().ok()?.trim();
    if text.is_empty() {
        return None;
    }

    if let Ok(seconds) = text.parse::<u64>() {
        return Some(Duration::from_secs(seconds).min(Duration::from_secs(
            MAX_PROXY_UPSTREAM_429_RETRY_AFTER_DELAY_SECS,
        )));
    }

    let retry_at = httpdate::parse_http_date(text).ok()?;
    let delay = retry_at.duration_since(std::time::SystemTime::now()).ok()?;
    Some(delay.min(Duration::from_secs(
        MAX_PROXY_UPSTREAM_429_RETRY_AFTER_DELAY_SECS,
    )))
}

pub(crate) async fn canonical_pool_attempt_proxy_binding_key(
    state: &AppState,
    selected_proxy_key: &str,
) -> Option<String> {
    let manager = state.forward_proxy.lock().await;
    manager.canonicalize_bound_proxy_key(selected_proxy_key, None)
}

pub(crate) fn normalize_pool_attempt_group_name(group_name: Option<String>) -> Option<String> {
    group_name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn take_same_account_only_terminal_error(
    last_error: &mut Option<PoolUpstreamError>,
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    let mut final_error = last_error
        .take()
        .expect("same-account-only retry should retain a terminal error");
    final_error.attempt_summary = pool_attempt_summary(
        attempt_count,
        distinct_account_count,
        Some(final_error.failure_kind.to_string()),
    );
    final_error
}

pub(crate) fn pool_retry_delay_with_total_timeout(
    retry_delay: Duration,
    same_account_only_retry_active: bool,
    responses_total_timeout: Option<Duration>,
    attempt_total_timeout_started_at: Option<Instant>,
) -> Duration {
    if !same_account_only_retry_active {
        return retry_delay;
    }
    pool_timeout_budget_with_total_limit(
        retry_delay,
        responses_total_timeout,
        attempt_total_timeout_started_at,
    )
    .unwrap_or(Duration::ZERO)
}
