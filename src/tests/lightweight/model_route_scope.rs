use super::*;

#[test]
fn model_route_scope_requires_model_specific_error_evidence() {
    assert!(is_explicit_model_failure_for_model(
        StatusCode::TOO_MANY_REQUESTS,
        Some("model rate limit exceeded"),
        Some("gpt-5.5"),
    ));
    assert!(is_explicit_model_failure_for_model(
        StatusCode::TOO_MANY_REQUESTS,
        Some("rate limit reached for gpt-5.5"),
        Some("gpt-5.5"),
    ));
    assert!(!is_explicit_model_failure_for_model(
        StatusCode::TOO_MANY_REQUESTS,
        Some("rate limit reached for this account"),
        Some("gpt-5.5"),
    ));
    assert!(!is_explicit_model_failure_for_model(
        StatusCode::TOO_MANY_REQUESTS,
        Some("organization model quota exceeded"),
        Some("gpt-5.5"),
    ));
}

#[test]
fn quota_failure_kinds_are_available_to_model_route_evaluation() {
    assert!(route_failure_kind_is_quota_exhausted(Some(
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
    )));
    assert!(!route_failure_kind_is_quota_exhausted(Some(
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
    )));
}
