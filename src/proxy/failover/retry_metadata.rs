use super::*;

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
