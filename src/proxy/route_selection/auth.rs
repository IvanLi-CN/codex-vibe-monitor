use super::*;

pub(super) fn plain_proxy_error(
    status: StatusCode,
    message: impl Into<String>,
) -> ProxyErrorResponse {
    let message = message.into();
    ProxyErrorResponse {
        retry_after_secs: retry_after_secs_for_proxy_error(status, &message),
        status,
        message,
        cvm_id: None,
        code: None,
        blocked_binding: None,
    }
}

pub(crate) fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    let authorization = headers.get(header::AUTHORIZATION)?.to_str().ok()?.trim();
    let (scheme, token) = authorization.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let normalized = token.trim();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

pub(crate) fn pool_route_response_status_is_success(status: StatusCode) -> bool {
    status.is_success() || status.is_redirection()
}

pub(crate) async fn request_matches_pool_route(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<bool> {
    let Some(api_key) = extract_bearer_token(headers) else {
        return Ok(false);
    };
    pool_api_key_matches(state, &api_key).await
}
