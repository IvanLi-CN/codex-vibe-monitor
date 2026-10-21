use super::*;

#[derive(Debug)]
pub(crate) struct ProxyErrorResponse {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
    pub(crate) cvm_id: Option<String>,
    pub(crate) retry_after_secs: Option<u64>,
    pub(crate) code: Option<String>,
    pub(crate) blocked_binding: Option<BlockedBindingDiagnostic>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProxyErrorResponseEnvelope {
    pub(crate) status: StatusCode,
    pub(crate) body_text: String,
    pub(crate) retry_after: Option<String>,
    pub(crate) cvm_invoke_id: Option<String>,
}

impl ProxyErrorResponseEnvelope {
    pub(crate) fn into_response(self) -> Response {
        let mut response = Response::new(axum::body::Body::from(self.body_text));
        *response.status_mut() = self.status;
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        if let Some(cvm_invoke_id) = self.cvm_invoke_id
            && let Ok(header_value) = HeaderValue::from_str(&cvm_invoke_id)
        {
            response
                .headers_mut()
                .insert(HeaderName::from_static(CVM_INVOKE_ID_HEADER), header_value);
        }
        if let Some(retry_after) = self.retry_after
            && let Ok(header_value) = HeaderValue::from_str(&retry_after)
        {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, header_value);
        }
        response
    }
}

pub(crate) fn build_proxy_error_response_envelope(
    err: &ProxyErrorResponse,
    invoke_id: &str,
) -> ProxyErrorResponseEnvelope {
    let code = (err.status == StatusCode::GATEWAY_TIMEOUT
        && err
            .message
            .contains(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT))
    .then_some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT.to_string())
    .or_else(|| err.code.clone());
    let mut payload = json!({ "error": err.message });
    if let Some(cvm_id) = err.cvm_id.as_ref() {
        payload["cvmId"] = json!(cvm_id);
    }
    if let Some(code) = code.as_ref() {
        payload["code"] = json!(code);
    }
    if let Some(blocked_binding) = err.blocked_binding.as_ref() {
        payload["blockedBinding"] = json!(blocked_binding);
    }
    let body_text = if err.status == StatusCode::NOT_IMPLEMENTED
        && err.message == WEBSOCKET_PROXY_REMOVED_MESSAGE
        && code.as_deref() == Some(WEBSOCKET_PROXY_REMOVED_CODE)
        && err.cvm_id.is_none()
        && err.retry_after_secs.is_none()
        && err.blocked_binding.is_none()
    {
        WEBSOCKET_PROXY_REMOVED_BODY.to_string()
    } else {
        serde_json::to_string(&payload).expect("proxy error response payload should serialize")
    };
    ProxyErrorResponseEnvelope {
        status: err.status,
        body_text,
        retry_after: err.retry_after_secs.map(|value| value.to_string()),
        cvm_invoke_id: err.cvm_id.as_ref().map(|_| invoke_id.to_string()),
    }
}

pub(crate) const PROXY_POOL_ROUTE_KEY_MISSING_OR_INVALID_MESSAGE: &str =
    "pool route key missing or invalid";
pub(crate) fn build_proxy_error_response(err: ProxyErrorResponse, invoke_id: &str) -> Response {
    build_proxy_error_response_envelope(&err, invoke_id).into_response()
}

pub(crate) fn build_proxy_error_response_without_invoke_id(err: ProxyErrorResponse) -> Response {
    build_proxy_error_response_envelope(&err, "").into_response()
}

pub(crate) fn build_proxy_invoke_id_allocation_error_response(
    error: impl std::fmt::Display,
) -> Response {
    build_proxy_error_response_without_invoke_id(ProxyErrorResponse {
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: format!("failed to allocate proxy invoke id: {error}"),
        cvm_id: None,
        retry_after_secs: None,
        code: None,
        blocked_binding: None,
    })
}

pub(crate) fn build_blocked_binding_diagnostic(
    constraint_source: BlockedBindingConstraintSource,
    upstream_account_id: i64,
    upstream_account_label: Option<&str>,
    prompt_cache_key: Option<&str>,
) -> BlockedBindingDiagnostic {
    BlockedBindingDiagnostic {
        constraint_source,
        upstream_account_id,
        upstream_account_label: blocked_binding_account_label(
            upstream_account_label,
            upstream_account_id,
        ),
        prompt_cache_key: prompt_cache_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        recovery_action: BlockedBindingRecoveryAction::ClearAndResetAffinity,
    }
}

pub(crate) fn single_account_binding_constraint_source(
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    owner_auto_guard_active: bool,
) -> Option<(BlockedBindingConstraintSource, i64)> {
    let PromptCacheConversationBindingConstraint::UpstreamAccount(upstream_account_id) =
        binding_constraint?
    else {
        return None;
    };
    Some((
        if owner_auto_guard_active {
            BlockedBindingConstraintSource::EncryptedSessionOwner
        } else {
            BlockedBindingConstraintSource::UpstreamAccountBinding
        },
        *upstream_account_id,
    ))
}

pub(crate) async fn build_single_account_binding_blocked_error(
    state: &AppState,
    binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    owner_auto_guard_active: bool,
    account: Option<PoolResolvedAccount>,
    prompt_cache_key: Option<&str>,
    message: Option<String>,
    attempt_count: usize,
    distinct_account_count: usize,
) -> Option<PoolUpstreamError> {
    let prompt_cache_key = prompt_cache_key
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let explicit_constraint =
        single_account_binding_constraint_source(binding_constraint, owner_auto_guard_active);
    let mut owner_row = None;
    let (constraint_source, upstream_account_id) =
        if let Some(explicit_constraint) = explicit_constraint {
            explicit_constraint
        } else {
            if owner_auto_guard_active && let Some(key) = prompt_cache_key {
                owner_row = load_prompt_cache_encrypted_session_owner_row(&state.pool, key)
                    .await
                    .ok()
                    .flatten();
            }
            owner_row.as_ref().map(|row| {
                (
                    BlockedBindingConstraintSource::EncryptedSessionOwner,
                    row.owner_upstream_account_id,
                )
            })?
        };
    let upstream_account_label = account
        .as_ref()
        .map(|value| value.display_name.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            owner_row
                .as_ref()
                .and_then(|row| row.owner_upstream_account_name.clone())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        });
    let blocked_binding = build_blocked_binding_diagnostic(
        constraint_source,
        upstream_account_id,
        upstream_account_label.as_deref(),
        prompt_cache_key,
    );
    let default_message = match constraint_source {
        BlockedBindingConstraintSource::UpstreamAccountBinding => format!(
            "prompt cache conversation is bound to upstream account {} but that account is currently unavailable",
            blocked_binding.upstream_account_label
        ),
        BlockedBindingConstraintSource::EncryptedSessionOwner => format!(
            "encrypted session owner routing is constrained to upstream account {} but that account is currently unavailable",
            blocked_binding.upstream_account_label
        ),
    };
    Some(build_pool_assigned_binding_blocked_error(
        account,
        message.unwrap_or(default_message),
        PROXY_FAILURE_POOL_ASSIGNED_ACCOUNT_BLOCKED,
        Some(blocked_binding),
        attempt_count,
        distinct_account_count,
    ))
}

pub(crate) fn proxy_error_response_from_pool_upstream_error(
    err: PoolUpstreamError,
    cvm_id: Option<String>,
) -> ProxyErrorResponse {
    ProxyErrorResponse {
        retry_after_secs: retry_after_secs_for_proxy_error(err.status, &err.message),
        status: err.status,
        message: err.message,
        cvm_id,
        code: Some(err.failure_kind.to_string()),
        blocked_binding: err.blocked_binding,
    }
}

pub(crate) struct ForwardProxyUpstreamResponse {
    pub(crate) selected_proxy: SelectedForwardProxy,
    pub(crate) response: ProxyUpstreamResponseBody,
    pub(crate) transport_bytes_live_counted: bool,
    pub(crate) connect_latency_ms: f64,
    /// `Instant` captured right before sending the upstream request for the final attempt.
    /// Used to record end-to-end latency once streaming finishes.
    pub(crate) attempt_started_at: Instant,
    pub(crate) attempt_recorded: bool,
    pub(crate) attempt_update: Option<ForwardProxyAttemptUpdate>,
    pub(crate) http_approx: ForwardProxyHttpApproxObservation,
}

#[derive(Debug)]
pub(crate) struct ForwardProxyUpstreamError {
    pub(crate) selected_proxy: SelectedForwardProxy,
    pub(crate) status: StatusCode,
    pub(crate) message: String,
    pub(crate) failure_kind: &'static str,
    pub(crate) attempt_failure_kind: &'static str,
    pub(crate) connect_latency_ms: f64,
    pub(crate) http_approx: ForwardProxyHttpApproxObservation,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ForwardProxyHttpApproxObservation {
    pub(crate) approx_upload_bytes: usize,
    pub(crate) approx_download_bytes_before_response_body: usize,
    pub(crate) final_response_header_bytes_approx: usize,
    pub(crate) request_compression: Option<RequestCompressionObservation>,
    pub(crate) request_transmission_complete: bool,
}

pub(crate) enum ProxyUpstreamResponseBody {
    Reqwest(reqwest::Response),
    Axum(Response),
}

impl fmt::Debug for ProxyUpstreamResponseBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reqwest(response) => f
                .debug_struct("ProxyUpstreamResponseBody::Reqwest")
                .field("status", &response.status())
                .finish(),
            Self::Axum(response) => f
                .debug_struct("ProxyUpstreamResponseBody::Axum")
                .field("status", &response.status())
                .finish(),
        }
    }
}

impl ProxyUpstreamResponseBody {
    pub(crate) fn status(&self) -> StatusCode {
        match self {
            Self::Reqwest(response) => response.status(),
            Self::Axum(response) => response.status(),
        }
    }

    pub(crate) fn headers(&self) -> &HeaderMap {
        match self {
            Self::Reqwest(response) => response.headers(),
            Self::Axum(response) => response.headers(),
        }
    }

    pub(crate) fn content_length(&self) -> Option<u64> {
        self.headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
    }

    pub(crate) async fn into_bytes(self) -> Result<Bytes, String> {
        match self {
            Self::Reqwest(response) => response.bytes().await.map_err(|err| err.to_string()),
            Self::Axum(response) => axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .map_err(|err| err.to_string()),
        }
    }

    async fn into_first_chunk(self) -> Result<(Self, Option<Bytes>, Option<Instant>), String> {
        match self {
            Self::Reqwest(mut response) => {
                let first_chunk = response.chunk().await.map_err(|err| err.to_string())?;
                let received_at = first_chunk.as_ref().map(|_| Instant::now());
                Ok((Self::Reqwest(response), first_chunk, received_at))
            }
            Self::Axum(response) => {
                let (parts, body) = response.into_parts();
                let mut stream = body.into_data_stream();
                let first_chunk = match stream.next().await {
                    Some(Ok(chunk)) => Some(chunk),
                    Some(Err(err)) => return Err(err.to_string()),
                    None => None,
                };
                let received_at = first_chunk.as_ref().map(|_| Instant::now());
                let response = Response::from_parts(parts, Body::from_stream(stream));
                Ok((Self::Axum(response), first_chunk, received_at))
            }
        }
    }

    pub(crate) fn into_bytes_stream(
        self,
    ) -> Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, io::Error>> + Send>> {
        match self {
            Self::Reqwest(response) => Box::pin(response.bytes_stream().map_err(io::Error::other)),
            Self::Axum(response) => Box::pin(
                response
                    .into_body()
                    .into_data_stream()
                    .map_err(|err| io::Error::other(err.to_string())),
            ),
        }
    }
}

pub(crate) const ASYNC_STREAMING_RAW_WRITER_QUEUE_CAPACITY: usize = 8;

pub(crate) fn pool_upstream_timeout_message(total_timeout: Duration, phase: &str) -> String {
    format!(
        "request timed out after {}ms while {phase}",
        total_timeout.as_millis()
    )
}

pub(crate) fn proxy_request_send_timeout_message(
    capture_target: Option<ProxyCaptureTarget>,
    total_timeout: Duration,
) -> String {
    match capture_target {
        Some(ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact) => {
            pool_upstream_timeout_message(total_timeout, "waiting for first upstream chunk")
        }
        _ => format!(
            "{PROXY_UPSTREAM_HANDSHAKE_TIMEOUT} after {}ms",
            total_timeout.as_millis()
        ),
    }
}

pub(crate) async fn read_pool_upstream_bytes_with_timeout(
    response: ProxyUpstreamResponseBody,
    total_timeout: Duration,
    started: Instant,
    phase: &str,
) -> Result<Bytes, String> {
    let Some(timeout_budget) = remaining_timeout_budget(total_timeout, started.elapsed()) else {
        return Err(pool_upstream_timeout_message(total_timeout, phase));
    };

    match timeout(timeout_budget, response.into_bytes()).await {
        Ok(result) => result,
        Err(_) => Err(pool_upstream_timeout_message(total_timeout, phase)),
    }
}

pub(crate) async fn read_pool_upstream_first_chunk_with_timeout(
    response: ProxyUpstreamResponseBody,
    total_timeout: Duration,
    started: Instant,
) -> Result<(ProxyUpstreamResponseBody, Option<Bytes>, Option<Instant>), String> {
    let Some(timeout_budget) = remaining_timeout_budget(total_timeout, started.elapsed()) else {
        return Err(pool_upstream_timeout_message(
            total_timeout,
            "waiting for first upstream chunk",
        ));
    };

    match timeout(timeout_budget, response.into_first_chunk()).await {
        Ok(result) => result,
        Err(_) => Err(pool_upstream_timeout_message(
            total_timeout,
            "waiting for first upstream chunk",
        )),
    }
}

#[derive(Debug)]
pub(crate) struct PoolUpstreamResponse {
    pub(crate) account: PoolResolvedAccount,
    pub(crate) response: ProxyUpstreamResponseBody,
    pub(crate) transport_bytes_live_counted: bool,
    pub(crate) stream_timeout: Option<Duration>,
    pub(crate) oauth_responses_debug: Option<oauth_bridge::OauthResponsesDebugInfo>,
    pub(crate) connect_latency_ms: f64,
    pub(crate) attempt_started_at_utc: DateTime<Utc>,
    pub(crate) first_byte_latency_ms: f64,
    pub(crate) first_chunk: Option<Bytes>,
    pub(crate) first_chunk_received_at: Option<Instant>,
    pub(crate) first_stream_chunk_received_at: Option<Instant>,
    pub(crate) pending_attempt_record: Option<PendingPoolAttemptRecord>,
    pub(crate) deferred_early_phase_cleanup_guard: Option<PoolEarlyPhaseOrphanCleanupGuard>,
    pub(crate) live_attempt_activity_lease: Option<PoolLiveAttemptActivityLease>,
    pub(crate) attempt_summary: PoolAttemptSummary,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) request_body_for_capture: Option<Bytes>,
    pub(crate) codex_imagegen_rewrite: Option<Value>,
    pub(crate) reservation_guard: Option<PoolRoutingReservationDropGuard>,
}

#[derive(Debug)]
pub(crate) struct PoolUpstreamError {
    pub(crate) account: Option<PoolResolvedAccount>,
    pub(crate) status: StatusCode,
    pub(crate) message: String,
    pub(crate) canonical_error_message: Option<String>,
    pub(crate) failure_kind: &'static str,
    pub(crate) blocked_binding: Option<BlockedBindingDiagnostic>,
    pub(crate) connect_latency_ms: f64,
    pub(crate) upstream_error_code: Option<String>,
    pub(crate) upstream_error_message: Option<String>,
    pub(crate) downstream_error_message: Option<String>,
    pub(crate) upstream_request_id: Option<String>,
    pub(crate) proxy_binding_key_snapshot: Option<String>,
    pub(crate) oauth_responses_debug: Option<oauth_bridge::OauthResponsesDebugInfo>,
    pub(crate) attempt_summary: PoolAttemptSummary,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) request_body_for_capture: Option<Bytes>,
    pub(crate) codex_imagegen_rewrite: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PoolAttemptSummary {
    pub(crate) pool_attempt_count: usize,
    pub(crate) pool_distinct_account_count: usize,
    pub(crate) pool_attempt_terminal_reason: Option<String>,
    pub(crate) pool_routing_no_candidate_audit: Option<PoolRoutingNoCandidateAudit>,
}

pub(crate) fn pool_attempt_summary(
    pool_attempt_count: usize,
    pool_distinct_account_count: usize,
    pool_attempt_terminal_reason: Option<String>,
) -> PoolAttemptSummary {
    PoolAttemptSummary {
        pool_attempt_count,
        pool_distinct_account_count,
        pool_attempt_terminal_reason,
        pool_routing_no_candidate_audit: None,
    }
}

pub(crate) fn pool_upstream_error_is_rate_limited(err: &PoolUpstreamError) -> bool {
    err.status == StatusCode::TOO_MANY_REQUESTS
        || matches!(
            err.failure_kind,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429
                | FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
                | PROXY_FAILURE_POOL_ALL_ACCOUNTS_RATE_LIMITED
        )
}

pub(crate) fn build_pool_rate_limited_error(
    attempt_count: usize,
    distinct_account_count: usize,
    failure_kind: &'static str,
) -> PoolUpstreamError {
    PoolUpstreamError {
        codex_imagegen_rewrite: None,
        account: None,
        status: StatusCode::TOO_MANY_REQUESTS,
        message: POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
        canonical_error_message: None,
        failure_kind,
        blocked_binding: None,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        downstream_error_message: None,
        upstream_request_id: None,
        proxy_binding_key_snapshot: None,
        oauth_responses_debug: None,
        attempt_summary: pool_attempt_summary(
            attempt_count,
            distinct_account_count,
            Some(failure_kind.to_string()),
        ),
        requested_service_tier: None,
        request_body_for_capture: None,
    }
}

pub(crate) fn build_pool_no_available_account_error(
    attempt_count: usize,
    distinct_account_count: usize,
    _retry_after_secs: u64,
    no_candidate_audit: Option<PoolRoutingNoCandidateAudit>,
) -> PoolUpstreamError {
    PoolUpstreamError {
        codex_imagegen_rewrite: None,
        account: None,
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: POOL_NO_AVAILABLE_ACCOUNT_MESSAGE.to_string(),
        canonical_error_message: None,
        failure_kind: PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
        blocked_binding: None,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        downstream_error_message: None,
        upstream_request_id: None,
        proxy_binding_key_snapshot: None,
        oauth_responses_debug: None,
        attempt_summary: PoolAttemptSummary {
            pool_routing_no_candidate_audit: no_candidate_audit,
            ..pool_attempt_summary(
                attempt_count,
                distinct_account_count,
                Some(PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT.to_string()),
            )
        },
        requested_service_tier: None,
        request_body_for_capture: None,
    }
}

pub(crate) const PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE: &str =
    "encrypted_session_owner_unavailable";
pub(crate) const ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE: &str = "encrypted session owner unavailable; automatic routing cannot move this encrypted conversation";

pub(crate) fn build_encrypted_session_owner_unavailable_error(
    account: Option<PoolResolvedAccount>,
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    PoolUpstreamError {
        codex_imagegen_rewrite: None,
        account,
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE.to_string(),
        canonical_error_message: None,
        failure_kind: PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE,
        blocked_binding: None,
        connect_latency_ms: 0.0,
        upstream_error_code: Some(PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE.to_string()),
        upstream_error_message: Some(ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE.to_string()),
        downstream_error_message: None,
        upstream_request_id: None,
        proxy_binding_key_snapshot: None,
        oauth_responses_debug: None,
        attempt_summary: pool_attempt_summary(
            attempt_count,
            distinct_account_count,
            Some(PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE.to_string()),
        ),
        requested_service_tier: None,
        request_body_for_capture: None,
    }
}

pub(crate) fn build_pool_assigned_account_blocked_error(
    account: PoolResolvedAccount,
    message: String,
    failure_kind: &'static str,
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    build_pool_assigned_binding_blocked_error(
        Some(account),
        message,
        failure_kind,
        None,
        attempt_count,
        distinct_account_count,
    )
}

pub(crate) fn build_pool_assigned_binding_blocked_error(
    account: Option<PoolResolvedAccount>,
    message: String,
    failure_kind: &'static str,
    blocked_binding: Option<BlockedBindingDiagnostic>,
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    PoolUpstreamError {
        codex_imagegen_rewrite: None,
        account,
        status: StatusCode::SERVICE_UNAVAILABLE,
        message,
        canonical_error_message: None,
        failure_kind,
        blocked_binding,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        downstream_error_message: None,
        upstream_request_id: None,
        proxy_binding_key_snapshot: None,
        oauth_responses_debug: None,
        attempt_summary: pool_attempt_summary(
            attempt_count,
            distinct_account_count,
            Some(failure_kind.to_string()),
        ),
        requested_service_tier: None,
        request_body_for_capture: None,
    }
}

pub(crate) fn retry_after_secs_for_proxy_error(status: StatusCode, message: &str) -> Option<u64> {
    if status != StatusCode::SERVICE_UNAVAILABLE {
        return None;
    }
    if message == POOL_NO_AVAILABLE_ACCOUNT_MESSAGE
        || message == ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE
    {
        return Some(DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS);
    }
    None
}

pub(crate) fn build_pool_degraded_only_error(
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    PoolUpstreamError {
        codex_imagegen_rewrite: None,
        account: None,
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
        canonical_error_message: None,
        failure_kind: PROXY_FAILURE_POOL_ALL_ACCOUNTS_DEGRADED,
        blocked_binding: None,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        downstream_error_message: None,
        upstream_request_id: None,
        proxy_binding_key_snapshot: None,
        oauth_responses_debug: None,
        attempt_summary: pool_attempt_summary(
            attempt_count,
            distinct_account_count,
            Some(PROXY_FAILURE_POOL_ALL_ACCOUNTS_DEGRADED.to_string()),
        ),
        requested_service_tier: None,
        request_body_for_capture: None,
    }
}

pub(crate) struct CompactSupportObservation {
    pub(crate) status: &'static str,
    pub(crate) reason: Option<String>,
}

pub(crate) fn proxy_forward_response_status_is_success(
    status: StatusCode,
    stream_error: bool,
) -> bool {
    !stream_error && status != StatusCode::TOO_MANY_REQUESTS && !status.is_server_error()
}

pub(crate) fn proxy_capture_response_status_is_success(
    status: StatusCode,
    stream_error: bool,
    logical_stream_failure: bool,
) -> bool {
    !logical_stream_failure && proxy_forward_response_status_is_success(status, stream_error)
}

pub(crate) fn proxy_capture_is_pure_downstream_close(
    status: StatusCode,
    stream_error: bool,
    logical_stream_failure: bool,
    downstream_closed: bool,
) -> bool {
    downstream_closed && status.is_success() && !stream_error && !logical_stream_failure
}

pub(crate) fn proxy_capture_invocation_failure_kind(
    status: StatusCode,
    stream_error: bool,
    logical_stream_failure: bool,
    pure_downstream_closed: bool,
) -> Option<&'static str> {
    if stream_error {
        Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
    } else if logical_stream_failure {
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    } else if pure_downstream_closed {
        Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED)
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    } else if status.is_server_error() {
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
    } else {
        None
    }
}

pub(crate) fn proxy_capture_invocation_status(
    status: StatusCode,
    has_error_message: bool,
    _pure_downstream_closed: bool,
) -> String {
    if status.is_success() && !has_error_message {
        "success".to_string()
    } else {
        format!("http_{}", status.as_u16())
    }
}

pub(crate) fn pool_capture_attempt_status(
    status: StatusCode,
    stream_error: bool,
    logical_stream_failure: bool,
    pure_downstream_closed: bool,
) -> &'static str {
    if pure_downstream_closed {
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    } else if stream_error {
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    } else if !status.is_success() || logical_stream_failure {
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
    } else {
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    }
}

pub(crate) fn proxy_forward_response_failure_kind(
    status: StatusCode,
    stream_error: bool,
) -> Option<&'static str> {
    if stream_error {
        Some(FORWARD_PROXY_FAILURE_STREAM_ERROR)
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    } else if status.is_server_error() {
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
    } else {
        None
    }
}

pub(crate) fn proxy_capture_response_failure_kind(
    status: StatusCode,
    stream_error: bool,
    logical_stream_failure: bool,
) -> Option<&'static str> {
    if logical_stream_failure {
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    } else {
        proxy_forward_response_failure_kind(status, stream_error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpstreamAccountFailureDisposition {
    HardUnavailable,
    RateLimited,
    Retryable,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct UpstreamAccountHttpFailureClassification {
    pub(crate) disposition: UpstreamAccountFailureDisposition,
    pub(crate) failure_kind: &'static str,
    pub(crate) reason_code: &'static str,
    pub(crate) next_account_status: Option<&'static str>,
}

pub(crate) fn upstream_error_indicates_quota_exhausted(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    [
        "insufficient_quota",
        "quota exhausted",
        "quota_exhausted",
        "the usage limit has been reached",
        "usage limit has been reached",
        "usage limit reached",
        "billing",
        "payment required",
        "subscription required",
        "weekly cap",
        "weekly limit",
        "plan limit",
        "plan quota",
        "check your plan",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

pub(crate) fn upstream_error_code_is_server_overloaded(code: Option<&str>) -> bool {
    code.is_some_and(|value| value.eq_ignore_ascii_case(UPSTREAM_ERROR_CODE_SERVER_IS_OVERLOADED))
}

fn upstream_error_message_indicates_concurrency_limit(message: Option<&str>) -> bool {
    let normalized = message.unwrap_or_default().to_ascii_lowercase();
    let has_concurrency_term =
        normalized.contains("concurrency") || normalized.contains("concurrent");
    let has_limit_term = normalized.contains("limit") || normalized.contains("request");
    let has_exhaustion_term = normalized.contains("exceed")
        || normalized.contains("too many")
        || normalized.contains("maximum")
        || normalized.contains("reached")
        || normalized.contains("hit");
    has_concurrency_term && has_limit_term && has_exhaustion_term
}

pub(crate) fn upstream_error_is_retryable_responses_overload(
    code: Option<&str>,
    message: Option<&str>,
) -> bool {
    if upstream_error_code_is_server_overloaded(code) {
        return true;
    }

    code.is_some_and(|value| value.eq_ignore_ascii_case(UPSTREAM_ERROR_CODE_RATE_LIMIT_EXCEEDED))
        && upstream_error_message_indicates_concurrency_limit(message)
}

pub(crate) fn route_http_failure_is_retryable_responses_overload(
    status: StatusCode,
    error_message: &str,
) -> bool {
    if status != StatusCode::OK {
        return false;
    }

    let prefix = format!("[{}] ", PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED);
    let Some(details) = error_message.strip_prefix(prefix.as_str()) else {
        return false;
    };
    let (code, message) = details
        .split_once(": ")
        .map_or((details, None), |(code, message)| (code, Some(message)));
    upstream_error_is_retryable_responses_overload(Some(code.trim()), message.map(str::trim))
}

pub(crate) fn response_info_is_retryable_responses_overload(
    status: StatusCode,
    response_info: &ResponseCaptureInfo,
) -> bool {
    status == StatusCode::OK
        && response_info.stream_terminal_event.is_some()
        && upstream_error_is_retryable_responses_overload(
            response_info.upstream_error_code.as_deref(),
            response_info.upstream_error_message.as_deref(),
        )
}

pub(crate) fn extract_unsupported_model_from_route_error(
    status: StatusCode,
    error_message: &str,
) -> Option<String> {
    static UNSUPPORTED_MODEL_CONTEXT_REGEX: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r#"(?xi)
            unsupported[_\s]+model\s*[:=]\s*['"`]?([a-z0-9][a-z0-9._-]{0,127})['"`]?
            |
            model(?:\s+id)?\s+['"`]?([a-z0-9][a-z0-9._-]{0,127})['"`]?\s+is\s+not\s+supported\b
            |
            model\s+is\s+not\s+supported\s*[:=]\s*['"`]?([a-z0-9][a-z0-9._-]{0,127})['"`]?
            "#,
        )
        .expect("valid unsupported model context regex")
    });
    if status != StatusCode::BAD_REQUEST {
        return None;
    }
    let normalized = error_message.to_ascii_lowercase();
    if !(normalized.contains("unsupported_model")
        || normalized.contains("unsupported model")
        || normalized.contains("model is not supported")
        || normalized.contains("is not supported")
        || normalized.contains("unsupported model"))
    {
        return None;
    }
    if normalized.contains("for model")
        && !normalized.contains("model is not supported")
        && !normalized.contains("unsupported model")
    {
        return None;
    }
    UNSUPPORTED_MODEL_CONTEXT_REGEX
        .captures_iter(error_message)
        .filter_map(|captures| (1..=3).find_map(|index| captures.get(index)))
        .map(|value| value.as_str().trim().to_string())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .any(|byte| byte.is_ascii_digit() || matches!(byte, b'-' | b'.'))
        })
        .last()
}

pub(crate) fn classify_pool_account_http_failure(
    account_kind: &str,
    status: StatusCode,
    error_message: &str,
) -> UpstreamAccountHttpFailureClassification {
    if status == StatusCode::TOO_MANY_REQUESTS
        && upstream_error_indicates_quota_exhausted(error_message)
    {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::RateLimited,
            failure_kind: FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            reason_code: "upstream_http_429_quota_exhausted",
            next_account_status: None,
        };
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::RateLimited,
            failure_kind: FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
            reason_code: "upstream_http_429_rate_limit",
            next_account_status: None,
        };
    }
    if status == StatusCode::PAYMENT_REQUIRED {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::HardUnavailable,
            failure_kind: PROXY_FAILURE_UPSTREAM_HTTP_402,
            reason_code: "upstream_http_402",
            next_account_status: Some("error"),
        };
    }
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::Retryable,
            failure_kind: PROXY_FAILURE_UPSTREAM_HTTP_413,
            reason_code: "upstream_http_413",
            next_account_status: None,
        };
    }
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        let next_account_status = if account_kind == "oauth_codex"
            && is_explicit_reauth_error_message(error_message)
            && !is_scope_permission_error_message(error_message)
            && !is_bridge_error_message(error_message)
        {
            Some("needs_reauth")
        } else {
            Some("error")
        };
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::HardUnavailable,
            failure_kind: PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
            reason_code: if status == StatusCode::UNAUTHORIZED {
                "upstream_http_401"
            } else {
                "upstream_http_403"
            },
            next_account_status,
        };
    }
    if status.is_server_error() {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::Retryable,
            failure_kind: FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX,
            reason_code: "upstream_http_5xx",
            next_account_status: None,
        };
    }
    UpstreamAccountHttpFailureClassification {
        disposition: UpstreamAccountFailureDisposition::Retryable,
        failure_kind: PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
        reason_code: "sync_error",
        next_account_status: None,
    }
}

pub(crate) fn compact_support_negative_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    let has_compact_signal = normalized.contains("compact")
        || normalized.contains("responses/compact")
        || normalized.contains("gpt-5.4-openai-compact");
    if normalized.contains("no available channel for model") && has_compact_signal {
        return true;
    }
    has_compact_signal
        && [
            "unsupported model",
            "unsupported endpoint",
            "unsupported path",
            "unsupported route",
            "not support",
            "does not support",
            "is not supported",
            "unknown model",
            "model not found",
            "no channel",
        ]
        .iter()
        .any(|needle| normalized.contains(needle))
}

pub(crate) fn classify_compact_support_observation(
    original_uri: &Uri,
    status: Option<StatusCode>,
    message: Option<&str>,
) -> Option<CompactSupportObservation> {
    if original_uri.path() != "/v1/responses/compact" {
        return None;
    }
    match status {
        Some(code) if code.is_success() => Some(CompactSupportObservation {
            status: COMPACT_SUPPORT_STATUS_SUPPORTED,
            reason: Some("compact request succeeded".to_string()),
        }),
        _ => {
            let message = message
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string());
            if message
                .as_deref()
                .is_some_and(compact_support_negative_signal)
            {
                Some(CompactSupportObservation {
                    status: COMPACT_SUPPORT_STATUS_UNSUPPORTED,
                    reason: message,
                })
            } else {
                Some(CompactSupportObservation {
                    status: COMPACT_SUPPORT_STATUS_UNKNOWN,
                    reason: message,
                })
            }
        }
    }
}

fn capability_support_failure_signal(normalized: &str) -> bool {
    [
        "unsupported endpoint",
        "unsupported path",
        "unsupported route",
        "unsupported tool",
        "unsupported model",
        "does not support",
        "is not supported",
        "not support",
        "unknown model",
        "model not found",
        "no available channel for model",
        "no channel",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

pub(crate) fn response_endpoint_capability_negative_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    capability_support_failure_signal(&normalized)
        && ["/v1/responses", "responses/compact"]
            .iter()
            .any(|needle| normalized.contains(needle))
}

pub(crate) fn chat_completions_capability_negative_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    capability_support_failure_signal(&normalized)
        && ["/v1/chat/completions", "chat/completions"]
            .iter()
            .any(|needle| normalized.contains(needle))
}

pub(crate) fn standalone_search_capability_negative_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    capability_support_failure_signal(&normalized)
        && ["/v1/alpha/search", "alpha/search", "standalone search"]
            .iter()
            .any(|needle| normalized.contains(needle))
}

pub(crate) fn classify_standalone_search_capability_observation(
    status: StatusCode,
    message: Option<&str>,
) -> CapabilitySupport {
    if status.is_success() {
        return CapabilitySupport::Supported;
    }
    if matches!(
        status,
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    ) {
        return CapabilitySupport::Unsupported;
    }
    if status == StatusCode::BAD_REQUEST
        && message
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some_and(standalone_search_capability_negative_signal)
    {
        CapabilitySupport::Unsupported
    } else {
        CapabilitySupport::Unknown
    }
}

pub(crate) fn response_image_tool_capability_negative_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    if is_responses_lite_top_level_image_tool_shape_error(&normalized) {
        return false;
    }
    if !capability_support_failure_signal(&normalized) {
        return false;
    }
    normalized.contains("image_generation")
        || normalized.contains("image generation")
        || normalized.contains("gpt-image-")
}

pub(crate) fn is_responses_lite_top_level_image_tool_shape_error(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("responses lite")
        && normalized.contains("top-level tool type")
        && normalized.contains("image_generation")
}

pub(crate) fn image_endpoint_capability_negative_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    if !capability_support_failure_signal(&normalized) {
        return false;
    }
    normalized.contains("gpt-image-")
        || normalized.contains("/v1/images/")
        || normalized.contains("images/generations")
        || normalized.contains("images/edits")
}

pub(crate) fn classify_response_endpoint_capability_observation(
    status: StatusCode,
    message: Option<&str>,
) -> CapabilitySupport {
    if status.is_success() {
        return CapabilitySupport::Supported;
    }
    let normalized_message = message
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if matches!(
        status,
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    ) && normalized_message
        .as_deref()
        .is_some_and(response_endpoint_capability_negative_signal)
    {
        CapabilitySupport::Unsupported
    } else {
        CapabilitySupport::Unknown
    }
}

pub(crate) fn classify_chat_completions_capability_observation(
    status: StatusCode,
    message: Option<&str>,
) -> CapabilitySupport {
    if status.is_success() {
        return CapabilitySupport::Supported;
    }
    let normalized_message = message
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if matches!(
        status,
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    ) && normalized_message
        .as_deref()
        .is_some_and(chat_completions_capability_negative_signal)
    {
        CapabilitySupport::Unsupported
    } else {
        CapabilitySupport::Unknown
    }
}

pub(crate) fn classify_response_image_tool_capability_observation(
    status: StatusCode,
    message: Option<&str>,
) -> CapabilitySupport {
    if status.is_success() {
        return CapabilitySupport::Supported;
    }
    let normalized_message = message
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if status == StatusCode::BAD_REQUEST
        && normalized_message
            .as_deref()
            .is_some_and(response_image_tool_capability_negative_signal)
    {
        CapabilitySupport::Unsupported
    } else {
        CapabilitySupport::Unknown
    }
}

pub(crate) fn classify_image_endpoint_capability_observation(
    status: StatusCode,
    message: Option<&str>,
) -> CapabilitySupport {
    if status.is_success() {
        return CapabilitySupport::Supported;
    }
    let normalized_message = message
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if status == StatusCode::BAD_REQUEST
        && normalized_message
            .as_deref()
            .is_some_and(image_endpoint_capability_negative_signal)
    {
        CapabilitySupport::Unsupported
    } else {
        CapabilitySupport::Unknown
    }
}

pub(crate) fn fallback_proxy_429_retry_delay(retry_index: u32) -> Duration {
    let exponent = retry_index.saturating_sub(1).min(16);
    let multiplier = 1_u64 << exponent;
    Duration::from_millis(500_u64.saturating_mul(multiplier)).min(Duration::from_secs(5))
}

pub(crate) fn fallback_proxy_429_retry_delay_for_state(
    state: &AppState,
    retry_index: u32,
) -> Duration {
    #[cfg(not(test))]
    let _ = state;

    #[cfg(test)]
    if let Some(delay) = state.fallback_proxy_429_retry_delay_override {
        return delay;
    }

    fallback_proxy_429_retry_delay(retry_index)
}

pub(crate) fn pool_group_upstream_429_retry_delay(state: &AppState) -> Duration {
    if let Some(delay) = state.pool_group_429_retry_delay_override {
        return delay;
    }
    Duration::from_secs(rand::thread_rng().gen_range(
        MIN_POOL_GROUP_UPSTREAM_429_RETRY_DELAY_SECS..=MAX_POOL_GROUP_UPSTREAM_429_RETRY_DELAY_SECS,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_unsupported_model_from_route_error_supports_short_and_hyphenated_ids() {
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "model o3 is not supported by this account",
            )
            .as_deref(),
            Some("o3")
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "unsupported model: 'o4-mini'",
            )
            .as_deref(),
            Some("o4-mini")
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "model id `computer-use-preview` is not supported",
            )
            .as_deref(),
            Some("computer-use-preview")
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "unsupported_model: pool upstream responded with 400: unsupported model: gpt-5.5",
            )
            .as_deref(),
            Some("gpt-5.5")
        );
    }

    #[test]
    fn extract_unsupported_model_from_route_error_ignores_non_model_bad_requests() {
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "request body is not supported for this endpoint",
            ),
            None
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::TOO_MANY_REQUESTS,
                "unsupported model: gpt-5.5",
            ),
            None
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "model is not supported",
            ),
            None
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "response_format is not supported for model gpt-4o",
            ),
            None
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "unsupported_model: pool",
            ),
            None
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "unsupported_model: response_format is not supported for model gpt-4o",
            ),
            None
        );
    }

    #[test]
    fn classify_response_endpoint_capability_observation_is_conservative() {
        assert_eq!(
            classify_response_endpoint_capability_observation(StatusCode::OK, None),
            CapabilitySupport::Supported
        );
        assert_eq!(
            classify_response_endpoint_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("unsupported endpoint: /v1/responses is not supported by this account"),
            ),
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            classify_response_endpoint_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("unsupported tool: image_generation is not supported by this account"),
            ),
            CapabilitySupport::Unknown
        );
    }

    #[test]
    fn classify_standalone_search_capability_observation_uses_route_specific_failures() {
        assert_eq!(
            classify_standalone_search_capability_observation(StatusCode::OK, None),
            CapabilitySupport::Supported
        );
        assert_eq!(
            classify_standalone_search_capability_observation(StatusCode::NOT_FOUND, None),
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            classify_standalone_search_capability_observation(
                StatusCode::METHOD_NOT_ALLOWED,
                Some("method not allowed"),
            ),
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            classify_standalone_search_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("unsupported endpoint: /v1/alpha/search"),
            ),
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            classify_standalone_search_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("request body is invalid"),
            ),
            CapabilitySupport::Unknown
        );
        for status in [
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
        ] {
            assert_eq!(
                classify_standalone_search_capability_observation(
                    status,
                    Some("standalone search request failed"),
                ),
                CapabilitySupport::Unknown
            );
        }
    }

    #[test]
    fn classify_response_image_tool_capability_observation_learns_tool_failures_only() {
        assert_eq!(
            classify_response_image_tool_capability_observation(StatusCode::OK, None),
            CapabilitySupport::Supported
        );
        assert_eq!(
            classify_response_image_tool_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("unsupported tool: image_generation is not supported by this account"),
            ),
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            classify_response_image_tool_capability_observation(
                StatusCode::BAD_REQUEST,
                Some(
                    "Responses Lite rejected top-level tool type image_generation; use input.additional_tools",
                ),
            ),
            CapabilitySupport::Unknown
        );
        assert_eq!(
            classify_response_image_tool_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("request body is invalid"),
            ),
            CapabilitySupport::Unknown
        );
        assert_eq!(
            classify_response_image_tool_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("invalid image size: width must be divisible by 64"),
            ),
            CapabilitySupport::Unknown
        );
    }

    fn classify_image_endpoint_capability_observation_learns_direct_image_failures() {
        assert_eq!(
            classify_image_endpoint_capability_observation(StatusCode::OK, None),
            CapabilitySupport::Supported
        );
        assert_eq!(
            classify_image_endpoint_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("No available channel for model gpt-image-1 under group default"),
            ),
            CapabilitySupport::Unsupported
        );
    }
}
