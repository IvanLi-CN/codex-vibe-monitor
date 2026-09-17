fn forwarded_request_compression_observation(
    headers: &HeaderMap,
) -> Option<(&'static str, &'static str)> {
    let encodings = parse_content_encodings(
        headers
            .get(header::CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok()),
    );
    if encodings.is_empty() || encodings.iter().all(|encoding| encoding == "identity") {
        return Some(("identity", "identity"));
    }

    match encodings.as_slice() {
        [encoding] => match encoding.as_str() {
            "gzip" | "x-gzip" => Some(("gzip", "passthrough")),
            "deflate" => Some(("deflate", "passthrough")),
            "zstd" => Some(("zstd", "passthrough")),
            _ => None,
        },
        _ => None,
    }
}

async fn latest_pool_attempt_request_compression_algorithm(
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
) -> Result<Option<Option<String>>, sqlx::Error> {
    sqlx::query_scalar::<_, Option<String>>(
        r#"
        SELECT NULLIF(TRIM(upstream_request_compression_algorithm), '')
        FROM pool_upstream_request_attempts
        WHERE invoke_id = ?1
          AND occurred_at = ?2
          AND LOWER(TRIM(COALESCE(status, ''))) <> 'budget_exhausted_final'
        ORDER BY attempt_index DESC, id DESC
        LIMIT 1
        "#,
    )
    .bind(&trace.invoke_id)
    .bind(&trace.occurred_at)
    .fetch_optional(&state.pool)
    .await
}

fn resolve_terminal_request_compression_algorithm(
    latest_attempt_algorithm: Option<Option<String>>,
    fallback_algorithm: Option<String>,
) -> Option<String> {
    latest_attempt_algorithm.unwrap_or(fallback_algorithm)
}

fn pool_terminal_request_compression_algorithm(
    headers: &HeaderMap,
    error: &PoolUpstreamError,
) -> Option<&'static str> {
    if error.attempt_summary.pool_attempt_count == 0 {
        return None;
    }

    let account = error.account.as_ref()?;
    resolved_pool_request_compression_algorithm(
        headers,
        account.auth.is_oauth(),
        account.request_compression_algorithm,
    )
}

fn resolved_pool_request_compression_algorithm(
    headers: &HeaderMap,
    is_oauth: bool,
    configured_algorithm: RequestCompressionAlgorithm,
) -> Option<&'static str> {
    if is_oauth || configured_algorithm == RequestCompressionAlgorithm::Follow {
        forwarded_request_compression_observation(headers).map(|(algorithm, _)| algorithm)
    } else {
        Some(configured_algorithm.as_str())
    }
}

fn spawn_pool_attempt_response_capture(
    state: Arc<AppState>,
    pending: PendingPoolAttemptRecord,
    response_body: Bytes,
    response_body_logging_enabled: bool,
    response_content_encoding: Option<String>,
) {
    let capture_key = pool_attempt_response_capture_key(&pending);
    tokio::spawn(async move {
        let raw_meta = spawn_raw_payload_file_write(
            state.as_ref(),
            &capture_key,
            "response",
            response_body,
            response_body_logging_enabled,
        )
        .finish()
        .await;
        let mut pending = pending;
        set_pending_pool_upstream_request_attempt_response_capture(
            &mut pending,
            &raw_meta,
            response_content_encoding.as_deref(),
        );
        match persist_pool_upstream_request_attempt_response_capture(&state.pool, &pending).await {
            Ok(()) => {
                if let Err(err) =
                    broadcast_pool_upstream_attempts_snapshot(state.as_ref(), &pending.invoke_id)
                        .await
                {
                    warn!(
                        invoke_id = %pending.invoke_id,
                        error = %err,
                        "failed to broadcast asynchronous pool attempt response capture"
                    );
                }
            }
            Err(err) => {
                warn!(
                    invoke_id = %pending.invoke_id,
                    error = %err,
                    "failed to persist asynchronous pool attempt response capture"
                );
            }
        }
    });
}

fn take_priority_handoff_terminal_error(
    account: &PoolResolvedAccount,
    last_error: &mut Option<PoolUpstreamError>,
    attempt_count: usize,
    distinct_account_count: usize,
) -> Option<PoolUpstreamError> {
    if !priority_handoff_is_single_attempt(account) {
        return None;
    }
    let mut error = last_error.take()?;
    error.attempt_summary = pool_attempt_summary(
        attempt_count,
        distinct_account_count,
        Some(error.failure_kind.to_string()),
    );
    Some(error)
}

fn priority_handoff_is_single_attempt(account: &PoolResolvedAccount) -> bool {
    account.routing_source == PoolRoutingSelectionSource::PriorityHandoff
        && account
            .priority_handoff_permit
            .as_ref()
            .is_some_and(|permit| permit.is_sticky_migration())
}
