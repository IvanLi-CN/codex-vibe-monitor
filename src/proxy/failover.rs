use super::*;

mod candidate_selection;
pub(crate) use candidate_selection::*;
mod retry_metadata;
pub(crate) use retry_metadata::*;
mod prepare_failure;
pub(super) use prepare_failure::record_pool_request_prepare_failure_attempt;
mod terminal_outcome;
pub(crate) use terminal_outcome::*;
mod attempt_execution;
pub(crate) use attempt_execution::*;
mod attempt_response;
pub(crate) use attempt_response::*;

pub(crate) async fn send_pool_request_with_failover(
    state: Arc<AppState>,
    proxy_request_id: u64,
    method: Method,
    original_uri: &Uri,
    headers: &HeaderMap,
    body: Option<PoolReplayBodySnapshot>,
    handshake_timeout: Duration,
    trace_context: Option<PoolUpstreamAttemptTraceContext>,
    runtime_snapshot_context: Option<PoolAttemptRuntimeSnapshotContext>,
    sticky_key: Option<&str>,
    preferred_account: Option<PoolResolvedAccount>,
    failover_progress: PoolFailoverProgress,
    same_account_attempts: u8,
) -> Result<PoolUpstreamResponse, PoolUpstreamError> {
    send_pool_request_with_failover_and_binding_constraint(
        state,
        proxy_request_id,
        method,
        original_uri,
        headers,
        body,
        handshake_timeout,
        trace_context,
        runtime_snapshot_context,
        sticky_key,
        None,
        None,
        None,
        preferred_account,
        failover_progress,
        same_account_attempts,
        true,
    )
    .await
}

pub(crate) fn send_pool_request_with_failover_and_binding_constraint<'a>(
    state: Arc<AppState>,
    proxy_request_id: u64,
    method: Method,
    original_uri: &'a Uri,
    headers: &'a HeaderMap,
    body: Option<PoolReplayBodySnapshot>,
    handshake_timeout: Duration,
    trace_context: Option<PoolUpstreamAttemptTraceContext>,
    runtime_snapshot_context: Option<PoolAttemptRuntimeSnapshotContext>,
    sticky_key: Option<&'a str>,
    sticky_event_prompt_cache_key: Option<&'a str>,
    binding_constraint: Option<PromptCacheConversationBindingConstraint>,
    conversation_override: Option<ConversationRoutingOverride>,
    preferred_account: Option<PoolResolvedAccount>,
    failover_progress: PoolFailoverProgress,
    same_account_attempts: u8,
    persist_terminal_invocation: bool,
) -> Pin<Box<dyn Future<Output = Result<PoolUpstreamResponse, PoolUpstreamError>> + Send + 'a>> {
    // Keep this large state machine off the request task stack. The imagegen audit
    // extends its state enough to overflow normal test and Axum worker stacks.
    Box::pin(async move {
        let capture_started = Instant::now();
        let state_for_terminal_capture = state.clone();
        let body_for_terminal_capture = body.clone();
        let trace_for_terminal_capture = trace_context.clone();
        let runtime_context_for_terminal_capture = runtime_snapshot_context.clone();
        let result = send_pool_request_with_failover_and_binding_constraint_inner(
            state,
            proxy_request_id,
            method,
            original_uri,
            headers,
            body,
            handshake_timeout,
            trace_context,
            runtime_snapshot_context,
            sticky_key,
            sticky_event_prompt_cache_key,
            binding_constraint,
            conversation_override,
            preferred_account,
            failover_progress,
            same_account_attempts,
        )
        .await;
        if persist_terminal_invocation && let Err(error) = &result {
            persist_pool_failover_terminal_invocation(
                state_for_terminal_capture,
                proxy_request_id,
                capture_started,
                original_uri,
                headers,
                trace_for_terminal_capture.as_ref(),
                runtime_context_for_terminal_capture.as_ref(),
                body_for_terminal_capture.unwrap_or(PoolReplayBodySnapshot::Empty),
                error,
            )
            .await;
        }
        result
    })
}

mod retry_orchestration;
pub(crate) use retry_orchestration::*;

#[cfg(test)]
mod request_compression_tests {
    use super::*;

    #[test]
    fn forwarded_oauth_request_compression_is_recorded_as_passthrough() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("zstd"));

        assert_eq!(
            forwarded_request_compression_observation(&headers),
            Some(("zstd", "passthrough"))
        );
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("x-gzip"));
        assert_eq!(
            forwarded_request_compression_observation(&headers),
            Some(("gzip", "passthrough"))
        );
        assert_eq!(
            resolved_pool_request_compression_algorithm(
                &headers,
                true,
                RequestCompressionAlgorithm::Identity,
            ),
            Some("gzip")
        );
    }

    #[test]
    fn terminal_pool_request_compression_uses_actual_forwarding_mode() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));

        assert_eq!(
            resolved_pool_request_compression_algorithm(
                &headers,
                false,
                RequestCompressionAlgorithm::Follow,
            ),
            Some("gzip")
        );
        assert_eq!(
            resolved_pool_request_compression_algorithm(
                &headers,
                false,
                RequestCompressionAlgorithm::Zstd,
            ),
            Some("zstd")
        );
    }

    #[test]
    fn terminal_pool_request_compression_keeps_unknown_attempt_metadata_unknown() {
        assert_eq!(
            resolve_terminal_request_compression_algorithm(Some(None), Some("zstd".to_string())),
            None
        );
        assert_eq!(
            resolve_terminal_request_compression_algorithm(
                Some(Some("gzip".to_string())),
                Some("zstd".to_string())
            ),
            Some("gzip".to_string())
        );
        assert_eq!(
            resolve_terminal_request_compression_algorithm(None, Some("zstd".to_string())),
            Some("zstd".to_string())
        );
    }

    #[test]
    fn priority_handoff_disables_upstream_413_retry_budget() {
        assert!(!pool_upstream_413_retry_allowed(
            StatusCode::PAYLOAD_TOO_LARGE,
            true,
            false,
        ));
        assert!(pool_upstream_413_retry_allowed(
            StatusCode::PAYLOAD_TOO_LARGE,
            false,
            false,
        ));
        assert!(!pool_upstream_413_retry_allowed(
            StatusCode::PAYLOAD_TOO_LARGE,
            false,
            true,
        ));
    }
}
