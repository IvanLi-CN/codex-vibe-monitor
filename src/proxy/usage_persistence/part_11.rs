pub(crate) async fn insert_pool_upstream_terminal_attempt(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    final_error: &PoolUpstreamError,
    attempt_index: i64,
    distinct_account_index: i64,
    failure_kind: &'static str,
) -> Result<()> {
    let _ = (
        pool,
        trace,
        final_error,
        attempt_index,
        distinct_account_index,
        failure_kind,
    );
    // Terminal pool adjudications do not represent a real upstream dispatch and must not create
    // pseudo-attempt rows going forward. Historical rows remain readable via workflow
    // reconstruction, but new data should rely on invocation-level route + final failure.
    Ok(())
}

pub(crate) async fn insert_and_broadcast_pool_upstream_terminal_attempt(
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    final_error: &PoolUpstreamError,
    attempt_index: i64,
    distinct_account_index: i64,
    failure_kind: &'static str,
) -> Result<()> {
    let _ = (
        state,
        trace,
        final_error,
        attempt_index,
        distinct_account_index,
        failure_kind,
    );
    Ok(())
}

pub(crate) fn prompt_cache_upstream_account_rollup_key(
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&str>,
) -> String {
    let normalized_name = upstream_account_name
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (upstream_account_id, normalized_name) {
        (Some(account_id), Some(account_name)) => format!("id:{account_id}|name:{account_name}"),
        (Some(account_id), None) => format!("id:{account_id}"),
        (None, Some(account_name)) => format!("name:{account_name}"),
        (None, None) => "unknown".to_string(),
    }
}

pub(crate) async fn load_hourly_rollup_live_progress(
    pool: &Pool<Sqlite>,
    dataset: &str,
) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind(dataset)
    .fetch_optional(pool)
    .await?
    .unwrap_or(0))
}
pub(crate) async fn load_hourly_rollup_live_progress_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind(dataset)
    .fetch_optional(&mut *tx)
    .await?
    .unwrap_or(0))
}

pub(crate) async fn save_hourly_rollup_live_progress_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    cursor_id: i64,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_live_progress (dataset, cursor_id, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(dataset) DO UPDATE SET
            cursor_id = MAX(hourly_rollup_live_progress.cursor_id, excluded.cursor_id),
            updated_at = datetime('now')
        "#,
    )
    .bind(dataset)
    .bind(cursor_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn mark_hourly_rollup_archive_replayed_tx(
    tx: &mut SqliteConnection,
    target: &str,
    dataset: &str,
    file_path: &str,
) -> Result<()> {
    let result = sqlx::query(
        r#"
        INSERT INTO hourly_rollup_archive_replay (
            target,
            dataset,
            file_path,
            archive_sha256,
            replayed_at
        )
        SELECT ?1, ?2, ?3, batches.sha256, datetime('now')
        FROM archive_batches AS batches
        WHERE batches.dataset = ?2
          AND batches.status IN ('completed', 'materializing')
          AND batches.file_path = ?3
          AND batches.sha256 IS NOT NULL
          AND TRIM(batches.sha256) <> ''
        ON CONFLICT(target, dataset, file_path) DO UPDATE SET
            archive_sha256 = excluded.archive_sha256,
            replayed_at = excluded.replayed_at
        "#,
    )
    .bind(target)
    .bind(dataset)
    .bind(file_path)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() != 1 {
        bail!("archive manifest changed before replay marker could be persisted: {file_path}");
    }
    Ok(())
}

pub(crate) async fn hourly_rollup_archive_replayed_tx(
    tx: &mut SqliteConnection,
    target: &str,
    dataset: &str,
    file_path: &str,
) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
            SELECT 1
            FROM hourly_rollup_archive_replay AS replay
            INNER JOIN archive_batches AS batches
                ON batches.dataset = replay.dataset
               AND batches.file_path = replay.file_path
               AND batches.status = 'completed'
               AND batches.sha256 IS NOT NULL
               AND TRIM(batches.sha256) <> ''
               AND batches.sha256 = replay.archive_sha256
            WHERE replay.target = ?1
              AND replay.dataset = ?2
              AND replay.file_path = ?3
            LIMIT 1
            "#,
    )
    .bind(target)
    .bind(dataset)
    .bind(file_path)
    .fetch_optional(&mut *tx)
    .await?
    .is_some())
}

pub(crate) fn normalized_oauth_account_id(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

pub(crate) fn looks_like_uuid_shape(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    for (idx, byte) in bytes.iter().enumerate() {
        let is_hyphen = matches!(idx, 8 | 13 | 18 | 23);
        if is_hyphen {
            if *byte != b'-' {
                return false;
            }
        } else if !byte.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

pub(crate) fn oauth_account_id_shape(value: Option<&str>) -> &'static str {
    match normalized_oauth_account_id(value) {
        None => "empty",
        Some(value) if value.starts_with("org_") => "org",
        Some(value) if looks_like_uuid_shape(value) => "uuid",
        Some(_) => "other",
    }
}

pub(crate) fn oauth_account_header_attached_for_account(
    account: Option<&PoolResolvedAccount>,
) -> Option<bool> {
    let PoolResolvedAuth::Oauth {
        chatgpt_account_id, ..
    } = &account?.auth
    else {
        return None;
    };

    Some(normalized_oauth_account_id(chatgpt_account_id.as_deref()).is_some())
}

pub(crate) fn oauth_account_id_shape_for_account(
    account: Option<&PoolResolvedAccount>,
) -> Option<&'static str> {
    let PoolResolvedAuth::Oauth {
        chatgpt_account_id, ..
    } = &account?.auth
    else {
        return None;
    };

    Some(oauth_account_id_shape(chatgpt_account_id.as_deref()))
}

pub(crate) struct ProxyPayloadSummary<'a> {
    pub(crate) target: ProxyCaptureTarget,
    pub(crate) status: StatusCode,
    pub(crate) is_stream: bool,
    pub(crate) request_contains_encrypted_content: bool,
    pub(crate) response_contains_encrypted_content: bool,
    pub(crate) compaction_request_kind: Option<CompactionKind>,
    pub(crate) compaction_response_kind: Option<CompactionKind>,
    pub(crate) image_intent: Option<&'a str>,
    pub(crate) request_model: Option<&'a str>,
    pub(crate) requested_service_tier: Option<&'a str>,
    pub(crate) billing_service_tier: Option<&'a str>,
    pub(crate) reasoning_effort: Option<&'a str>,
    pub(crate) response_model: Option<&'a str>,
    pub(crate) usage_missing_reason: Option<&'a str>,
    pub(crate) request_parse_error: Option<&'a str>,
    pub(crate) request_compression_algorithm: Option<&'a str>,
    pub(crate) request_compression_mode: Option<&'a str>,
    pub(crate) request_compression_logical_body_bytes: Option<usize>,
    pub(crate) request_compression_transmitted_body_bytes: Option<usize>,
    pub(crate) request_compression_transmission_complete: Option<bool>,
    pub(crate) failure_kind: Option<&'a str>,
    pub(crate) requester_ip: Option<&'a str>,
    pub(crate) request_user_agent: Option<&'a str>,
    pub(crate) request_x_forwarded_for: Option<&'a str>,
    pub(crate) request_forwarded: Option<&'a str>,
    pub(crate) request_x_real_ip: Option<&'a str>,
    pub(crate) upstream_scope: &'a str,
    pub(crate) route_mode: &'a str,
    pub(crate) sticky_key: Option<&'a str>,
    pub(crate) prompt_cache_key: Option<&'a str>,
    pub(crate) prompt_cache_key_attribution_source: Option<&'a str>,
    pub(crate) client_fingerprint: Option<&'a str>,
    pub(crate) client_header_fingerprints: Option<&'a BTreeMap<String, String>>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<&'a str>,
    pub(crate) upstream_account_kind: Option<&'a str>,
    pub(crate) upstream_base_url_host: Option<&'a str>,
    pub(crate) oauth_account_header_attached: Option<bool>,
    pub(crate) oauth_account_id_shape: Option<&'a str>,
    pub(crate) oauth_forwarded_header_count: Option<usize>,
    pub(crate) oauth_forwarded_header_names: Option<&'a [String]>,
    pub(crate) oauth_fingerprint_version: Option<&'a str>,
    pub(crate) oauth_forwarded_header_fingerprints: Option<&'a BTreeMap<String, String>>,
    pub(crate) oauth_prompt_cache_header_forwarded: Option<bool>,
    pub(crate) oauth_request_body_prefix_fingerprint: Option<&'a str>,
    pub(crate) oauth_request_body_prefix_bytes: Option<usize>,
    pub(crate) oauth_request_body_snapshot_kind: Option<&'a str>,
    pub(crate) oauth_responses_body_mode: Option<&'a str>,
    pub(crate) oauth_responses_rewrite: Option<&'a oauth_bridge::OauthResponsesRewriteSummary>,
    pub(crate) service_tier: Option<&'a str>,
    pub(crate) stream_terminal_event: Option<&'a str>,
    pub(crate) upstream_error_code: Option<&'a str>,
    pub(crate) upstream_error_message: Option<&'a str>,
    pub(crate) downstream_status_code: Option<StatusCode>,
    pub(crate) downstream_error_message: Option<&'a str>,
    pub(crate) upstream_request_id: Option<&'a str>,
    pub(crate) response_content_encoding: Option<&'a str>,
    pub(crate) stream_failure_origin: Option<&'a str>,
    pub(crate) upstream_read_error_kind: Option<&'a str>,
    pub(crate) content_encoding_chain: Option<&'a str>,
    pub(crate) forwarded_chunk_count: Option<usize>,
    pub(crate) forwarded_bytes: Option<usize>,
    pub(crate) usage_observed: Option<bool>,
    pub(crate) downstream_close_phase: Option<&'a str>,
    pub(crate) downstream_write_error_kind: Option<&'a str>,
    pub(crate) last_upstream_chunk_gap_ms: Option<u64>,
    pub(crate) upstream_approx_upload_bytes: Option<usize>,
    pub(crate) upstream_approx_download_bytes: Option<usize>,
    pub(crate) proxy_display_name: Option<&'a str>,
    pub(crate) proxy_weight_delta: Option<f64>,
    pub(crate) pool_attempt_count: Option<usize>,
    pub(crate) pool_distinct_account_count: Option<usize>,
    pub(crate) pool_attempt_terminal_reason: Option<&'a str>,
    pub(crate) blocked_binding: Option<&'a BlockedBindingDiagnostic>,
}

pub(crate) fn build_proxy_payload_summary(summary: ProxyPayloadSummary<'_>) -> String {
    let mut payload = json!({
        "endpoint": summary.target.endpoint(),
        "statusCode": summary.status.as_u16(),
        "isStream": summary.is_stream,
        "requestContainsEncryptedContent": summary.request_contains_encrypted_content,
        "responseContainsEncryptedContent": summary.response_contains_encrypted_content,
        "compactionRequestKind": summary.compaction_request_kind.map(CompactionKind::as_payload_str),
        "compactionResponseKind": summary.compaction_response_kind.map(CompactionKind::as_payload_str),
        "imageIntent": summary.image_intent,
        "requestModel": summary.request_model,
        "requestedServiceTier": summary.requested_service_tier,
        "billingServiceTier": summary.billing_service_tier,
        "reasoningEffort": summary.reasoning_effort,
        "responseModel": summary.response_model,
        "usageMissingReason": summary.usage_missing_reason,
        "requestParseError": summary.request_parse_error,
        "requestCompressionAlgorithm": summary.request_compression_algorithm,
        "requestCompressionMode": summary.request_compression_mode,
        "requestCompressionLogicalBodyBytes": summary.request_compression_logical_body_bytes,
        "requestCompressionTransmittedBodyBytes": summary.request_compression_transmitted_body_bytes,
        "requestCompressionTransmissionComplete": summary.request_compression_transmission_complete,
        "failureKind": summary.failure_kind,
        "requesterIp": summary.requester_ip,
        "requestUserAgent": summary.request_user_agent,
        "requestXForwardedFor": summary.request_x_forwarded_for,
        "requestForwarded": summary.request_forwarded,
        "requestXRealIp": summary.request_x_real_ip,
        "upstreamScope": summary.upstream_scope,
        "routeMode": summary.route_mode,
        "stickyKey": summary.sticky_key,
        "promptCacheKey": summary.prompt_cache_key,
        "promptCacheKeyAttributionSource": summary.prompt_cache_key_attribution_source,
        "clientFingerprint": summary.client_fingerprint,
        "clientHeaderFingerprints": summary.client_header_fingerprints,
        "upstreamAccountId": summary.upstream_account_id,
        "upstreamAccountName": summary.upstream_account_name,
        "upstreamAccountKind": summary.upstream_account_kind,
        "upstreamBaseUrlHost": summary.upstream_base_url_host,
        "oauthAccountHeaderAttached": summary.oauth_account_header_attached,
        "oauthAccountIdShape": summary.oauth_account_id_shape,
        "oauthForwardedHeaderCount": summary.oauth_forwarded_header_count,
        "oauthForwardedHeaderNames": summary.oauth_forwarded_header_names,
        "oauthFingerprintVersion": summary.oauth_fingerprint_version,
        "oauthForwardedHeaderFingerprints": summary.oauth_forwarded_header_fingerprints,
        "oauthPromptCacheHeaderForwarded": summary.oauth_prompt_cache_header_forwarded,
        "oauthRequestBodyPrefixFingerprint": summary.oauth_request_body_prefix_fingerprint,
        "oauthRequestBodyPrefixBytes": summary.oauth_request_body_prefix_bytes,
        "oauthRequestBodySnapshotKind": summary.oauth_request_body_snapshot_kind,
        "oauthResponsesBodyMode": summary.oauth_responses_body_mode,
        "oauthResponsesRewrite": summary.oauth_responses_rewrite,
    });
    extend_proxy_payload_summary_diagnostics(&mut payload, &summary);
    serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string())
}

fn extend_proxy_payload_summary_diagnostics(
    payload: &mut Value,
    summary: &ProxyPayloadSummary<'_>,
) {
    let Some(object) = payload.as_object_mut() else {
        return;
    };
    let Value::Object(additions) = json!({
        "serviceTier": summary.service_tier,
        "streamTerminalEvent": summary.stream_terminal_event,
        "upstreamErrorCode": summary.upstream_error_code,
        "upstreamErrorMessage": summary.upstream_error_message,
        "downstreamStatusCode": summary.downstream_status_code.map(|value| value.as_u16()),
        "downstreamErrorMessage": summary.downstream_error_message,
        "upstreamRequestId": summary.upstream_request_id,
        "responseContentEncoding": summary.response_content_encoding,
        "streamFailureOrigin": summary.stream_failure_origin,
        "upstreamReadErrorKind": summary.upstream_read_error_kind,
        "contentEncodingChain": summary.content_encoding_chain,
        "forwardedChunkCount": summary.forwarded_chunk_count,
        "forwardedBytes": summary.forwarded_bytes,
        "usageObserved": summary.usage_observed,
        "downstreamClosePhase": summary.downstream_close_phase,
        "downstreamWriteErrorKind": summary.downstream_write_error_kind,
        "lastUpstreamChunkGapMs": summary.last_upstream_chunk_gap_ms,
        "upstreamApproxUploadBytes": summary.upstream_approx_upload_bytes,
        "upstreamApproxDownloadBytes": summary.upstream_approx_download_bytes,
        "proxyDisplayName": summary.proxy_display_name,
        "proxyWeightDelta": summary.proxy_weight_delta,
        "poolAttemptCount": summary.pool_attempt_count,
        "poolDistinctAccountCount": summary.pool_distinct_account_count,
        "poolAttemptTerminalReason": summary.pool_attempt_terminal_reason,
        "blockedBinding": summary.blocked_binding,
    }) else {
        return;
    };
    object.extend(additions);
}

pub(crate) fn with_proxy_stream_terminal_diagnostics(
    payload: String,
    upstream_outcome: Option<&str>,
    post_terminal_upstream_read_error_kind: Option<&str>,
    post_terminal_upstream_read_error_message: Option<&str>,
    post_terminal_downstream_write_error_kind: Option<&str>,
    post_terminal_downstream_write_error_message: Option<&str>,
) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(&payload) else {
        return payload;
    };
    let Some(object) = value.as_object_mut() else {
        return payload;
    };
    for (key, entry) in [
        ("upstreamOutcome", upstream_outcome),
        (
            "postTerminalUpstreamReadErrorKind",
            post_terminal_upstream_read_error_kind,
        ),
        (
            "postTerminalUpstreamReadErrorMessage",
            post_terminal_upstream_read_error_message,
        ),
        (
            "postTerminalDownstreamWriteErrorKind",
            post_terminal_downstream_write_error_kind,
        ),
        (
            "postTerminalDownstreamWriteErrorMessage",
            post_terminal_downstream_write_error_message,
        ),
    ] {
        if let Some(entry) = entry {
            object.insert(key.to_string(), Value::String(entry.to_string()));
        }
    }
    serde_json::to_string(&value).unwrap_or(payload)
}

pub(crate) fn with_image_tool_rewrite_payload_summary(
    payload: String,
    image_tool_rewrite: Option<&Value>,
) -> String {
    let Some(image_tool_rewrite) = image_tool_rewrite else {
        return payload;
    };
    let Ok(mut value) = serde_json::from_str::<Value>(&payload) else {
        return payload;
    };
    let Some(object) = value.as_object_mut() else {
        return payload;
    };
    object.insert("imageToolRewrite".to_string(), image_tool_rewrite.clone());
    serde_json::to_string(&value).unwrap_or(payload)
}

pub(crate) fn with_codex_imagegen_rewrite_payload_summary(
    payload: String,
    codex_imagegen_rewrite: Option<&Value>,
) -> String {
    let Some(codex_imagegen_rewrite) = codex_imagegen_rewrite else {
        return payload;
    };
    let Ok(mut value) = serde_json::from_str::<Value>(&payload) else {
        return payload;
    };
    let Some(object) = value.as_object_mut() else {
        return payload;
    };
    object.insert(
        "codexImagegenRewrite".to_string(),
        codex_imagegen_rewrite.clone(),
    );
    serde_json::to_string(&value).unwrap_or(payload)
}

pub(crate) fn invocation_status_is_in_flight(status: Option<&str>) -> bool {
    matches!(
        status
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        INVOCATION_STATUS_RUNNING | INVOCATION_STATUS_PENDING
    )
}

pub(crate) fn invocation_status_is_recoverable_proxy_interrupted(
    status: Option<&str>,
    failure_kind: Option<&str>,
) -> bool {
    status
        .unwrap_or_default()
        .trim()
        .eq_ignore_ascii_case(INVOCATION_STATUS_INTERRUPTED)
        && failure_kind
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case(PROXY_FAILURE_INVOCATION_INTERRUPTED)
}

pub(crate) fn nullable_runtime_timing_value(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

#[derive(Debug, FromRow)]
pub(crate) struct PersistedInvocationIdentityRow {
    pub(crate) id: i64,
    pub(crate) status: Option<String>,
    pub(crate) failure_kind: Option<String>,
}

pub(crate) async fn load_persisted_invocation_identity_tx(
    tx: &mut SqliteConnection,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<Option<PersistedInvocationIdentityRow>> {
    sqlx::query_as::<_, PersistedInvocationIdentityRow>(
        r#"
        SELECT id, status, failure_kind
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_optional(&mut *tx)
    .await
    .map_err(Into::into)
}

pub(crate) struct ProxyInvocationUpdateRequest<'a> {
    pub(crate) id: i64,
    pub(crate) record: &'a ProxyCaptureRecord,
    pub(crate) raw_response: &'a str,
    pub(crate) resp_raw: &'a RawPayloadMeta,
    pub(crate) failure_kind: Option<&'a str>,
    pub(crate) failure_class: &'a str,
    pub(crate) is_actionable: bool,
    pub(crate) t_total_ms: Option<f64>,
    pub(crate) t_req_read_ms: Option<f64>,
    pub(crate) t_req_parse_ms: Option<f64>,
    pub(crate) t_upstream_connect_ms: Option<f64>,
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    pub(crate) first_token_ms: Option<f64>,
    pub(crate) t_upstream_stream_ms: Option<f64>,
    pub(crate) t_resp_parse_ms: Option<f64>,
    pub(crate) t_persist_ms: Option<f64>,
}

pub(crate) async fn update_existing_proxy_invocation_record_tx(
    tx: &mut SqliteConnection,
    request: ProxyInvocationUpdateRequest<'_>,
) -> Result<bool> {
    execute_proxy_invocation_update(tx, request).await
}

const PROXY_INVOCATION_UPDATE_QUERY: &str = r#"
        UPDATE codex_invocations
        SET
            source = ?2,
            model = ?3,
            input_tokens = ?4,
            output_tokens = ?5,
            cache_input_tokens = ?6,
            reasoning_tokens = ?7,
            total_tokens = ?8,
            cost = ?9,
            cost_input = ?10,
            cost_cache_write = ?11,
            cost_cache_read = ?12,
            cost_output = ?13,
            cost_reasoning = ?14,
            cost_estimated = ?15,
            price_version = ?16,
            status = ?17,
            error_message = ?18,
            failure_kind = ?19,
            failure_class = ?20,
            is_actionable = ?21,
            payload = ?22,
            raw_response = ?23,
            request_raw_path = ?24,
            request_raw_codec = ?25,
            request_raw_size = ?26,
            request_raw_truncated = ?27,
            request_raw_truncated_reason = ?28,
            response_raw_path = ?29,
            response_raw_codec = ?30,
            response_raw_size = ?31,
            response_raw_truncated = ?32,
            response_raw_truncated_reason = ?33,
            t_total_ms = ?34,
            t_req_read_ms = ?35,
            t_req_parse_ms = ?36,
            t_upstream_connect_ms = ?37,
            t_upstream_ttfb_ms = ?38,
            first_token_ms = ?39,
            t_upstream_stream_ms = ?40,
            t_resp_parse_ms = ?41,
            t_persist_ms = ?42
        WHERE id = ?1
          AND (
                LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
                OR (
                    LOWER(TRIM(COALESCE(status, ''))) = 'interrupted'
                    AND LOWER(TRIM(COALESCE(failure_kind, ''))) = 'proxy_interrupted'
                )
          )
        "#;

async fn execute_proxy_invocation_update(
    tx: &mut SqliteConnection,
    request: ProxyInvocationUpdateRequest<'_>,
) -> Result<bool> {
    let ProxyInvocationUpdateRequest {
        id,
        record,
        raw_response,
        resp_raw,
        failure_kind,
        failure_class,
        is_actionable,
        t_total_ms,
        t_req_read_ms,
        t_req_parse_ms,
        t_upstream_connect_ms,
        t_upstream_ttfb_ms,
        first_token_ms,
        t_upstream_stream_ms,
        t_resp_parse_ms,
        t_persist_ms,
    } = request;
    let result = sqlx::query(PROXY_INVOCATION_UPDATE_QUERY)
        .bind(id)
        .bind(SOURCE_PROXY)
        .bind(&record.model)
        .bind(record.usage.input_tokens)
        .bind(record.usage.output_tokens)
        .bind(record.usage.cache_input_tokens)
        .bind(record.usage.reasoning_tokens)
        .bind(record.usage.total_tokens)
        .bind(record.cost)
        .bind(record.cost_breakdown.map(|value| value.input))
        .bind(record.cost_breakdown.map(|value| value.cache_write))
        .bind(record.cost_breakdown.map(|value| value.cache_read))
        .bind(record.cost_breakdown.map(|value| value.output))
        .bind(record.cost_breakdown.map(|value| value.reasoning))
        .bind(record.cost_estimated as i64)
        .bind(record.price_version.as_deref())
        .bind(&record.status)
        .bind(record.error_message.as_deref())
        .bind(failure_kind)
        .bind(failure_class)
        .bind(is_actionable as i64)
        .bind(record.payload.as_deref())
        .bind(raw_response)
        .bind(record.req_raw.path.as_deref())
        .bind(raw_payload_meta_codec(&record.req_raw))
        .bind(record.req_raw.size_bytes)
        .bind(record.req_raw.truncated as i64)
        .bind(record.req_raw.truncated_reason.as_deref())
        .bind(resp_raw.path.as_deref())
        .bind(raw_payload_meta_codec(resp_raw))
        .bind(resp_raw.size_bytes)
        .bind(resp_raw.truncated as i64)
        .bind(resp_raw.truncated_reason.as_deref())
        .bind(t_total_ms)
        .bind(t_req_read_ms)
        .bind(t_req_parse_ms)
        .bind(t_upstream_connect_ms)
        .bind(t_upstream_ttfb_ms)
        .bind(first_token_ms)
        .bind(t_upstream_stream_ms)
        .bind(t_resp_parse_ms)
        .bind(t_persist_ms)
        .execute(&mut *tx)
        .await?;

    Ok(result.rows_affected() > 0)
}

struct RuntimeRecordPayload {
    value: Option<Value>,
    prompt_cache_key: Option<String>,
    sticky_key: Option<String>,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<String>,
    blocked_binding: Option<BlockedBindingDiagnostic>,
    blocked_binding_json: Option<String>,
    failure: FailureClassification,
}

impl RuntimeRecordPayload {
    fn text(&self, key: &str) -> Option<String> {
        self.value
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }

    fn i64(&self, key: &str) -> Option<i64> {
        self.value
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(crate::proxy::json_value_to_i64)
    }

    fn f64(&self, key: &str) -> Option<f64> {
        self.value
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(Value::as_f64)
    }
}

fn runtime_record_payload(record: &ProxyCaptureRecord) -> RuntimeRecordPayload {
    let payload = record
        .payload
        .as_deref()
        .and_then(|payload| serde_json::from_str::<Value>(payload).ok());
    let payload_text = |key: &str| {
        payload
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    };
    let prompt_cache_key = payload_text("promptCacheKey");
    let sticky_key = payload
        .as_ref()
        .and_then(|value| {
            value
                .get("stickyKey")
                .or_else(|| value.get("promptCacheKey"))
        })
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let upstream_account_id = payload
        .as_ref()
        .and_then(|value| value.get("upstreamAccountId"))
        .and_then(crate::proxy::json_value_to_i64);
    let upstream_account_name = payload_text("upstreamAccountName");
    let blocked_binding_json = payload
        .as_ref()
        .and_then(|value| value.get("blockedBinding"))
        .filter(|value| value.is_object())
        .and_then(|value| serde_json::to_string(value).ok());
    let blocked_binding = parse_blocked_binding_json(blocked_binding_json.as_deref());
    let failure = resolve_failure_classification(
        Some(record.status.as_str()),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        None,
        None,
    );
    RuntimeRecordPayload {
        value: payload,
        prompt_cache_key,
        sticky_key,
        upstream_account_id,
        upstream_account_name,
        blocked_binding,
        blocked_binding_json,
        failure,
    }
}

pub(crate) fn api_invocation_from_runtime_record(record: &ProxyCaptureRecord) -> ApiInvocation {
    build_api_invocation_from_runtime_record(record, runtime_record_payload(record))
}

fn build_api_invocation_from_runtime_record(
    record: &ProxyCaptureRecord,
    payload: RuntimeRecordPayload,
) -> ApiInvocation {
    let failure = payload.failure.clone();
    ApiInvocation {
        id: 0,
        invoke_id: record.invoke_id.clone(),
        occurred_at: record.occurred_at.clone(),
        source: SOURCE_PROXY.to_string(),
        proxy_display_name: payload.text("proxyDisplayName"),
        model: record.model.clone(),
        request_model: payload.text("requestModel"),
        response_model: payload.text("responseModel"),
        input_tokens: record.usage.input_tokens,
        output_tokens: record.usage.output_tokens,
        cache_input_tokens: record.usage.cache_input_tokens,
        reasoning_tokens: record.usage.reasoning_tokens,
        reasoning_effort: payload.text("reasoningEffort"),
        total_tokens: record.usage.total_tokens,
        cost: record.cost,
        cost_input: record.cost_breakdown.map(|value| value.input),
        cost_cache_write: record.cost_breakdown.map(|value| value.cache_write),
        cost_cache_read: record.cost_breakdown.map(|value| value.cache_read),
        cost_output: record.cost_breakdown.map(|value| value.output),
        cost_reasoning: record.cost_breakdown.map(|value| value.reasoning),
        cache_write_tokens: record.usage.input_tokens.map(|input| {
            input.saturating_sub(record.usage.cache_input_tokens.unwrap_or_default().max(0))
        }),
        status: Some(record.status.clone()),
        live_phase: None,
        error_message: record.error_message.clone(),
        downstream_status_code: payload.i64("downstreamStatusCode"),
        failure_kind: failure
            .failure_kind
            .clone()
            .or_else(|| record.failure_kind.clone()),
        blocked_binding: payload.blocked_binding.clone(),
        blocked_binding_json: payload.blocked_binding_json.clone(),
        stream_terminal_event: payload.text("streamTerminalEvent"),
        upstream_error_code: payload.text("upstreamErrorCode"),
        upstream_error_message: payload.text("upstreamErrorMessage"),
        downstream_error_message: payload.text("downstreamErrorMessage"),
        upstream_request_id: payload.text("upstreamRequestId"),
        failure_class: Some(failure.failure_class.as_str().to_string()),
        is_actionable: Some(failure.is_actionable),
        endpoint: payload.text("endpoint"),
        compaction_request_kind: payload.text("compactionRequestKind"),
        compaction_response_kind: payload.text("compactionResponseKind"),
        image_intent: payload.text("imageIntent"),
        requester_ip: payload.text("requesterIp"),
        prompt_cache_key: payload.prompt_cache_key.clone(),
        sticky_key: payload.sticky_key.clone(),
        route_mode: payload.text("routeMode"),
        upstream_account_id: payload.upstream_account_id,
        upstream_account_name: payload.upstream_account_name.clone(),
        response_content_encoding: payload.text("responseContentEncoding"),
        request_compression_algorithm: payload.text("requestCompressionAlgorithm"),
        transport: None,
        pool_attempt_count: payload.i64("poolAttemptCount"),
        pool_distinct_account_count: payload.i64("poolDistinctAccountCount"),
        pool_attempt_terminal_reason: payload.text("poolAttemptTerminalReason"),
        requested_service_tier: payload.text("requestedServiceTier"),
        service_tier: payload.text("serviceTier"),
        billing_service_tier: payload.text("billingServiceTier"),
        proxy_weight_delta: payload.f64("proxyWeightDelta"),
        cost_estimated: Some(record.cost_estimated as i64),
        price_version: record.price_version.clone(),
        cost_audit: None,
        request_raw_path: record.req_raw.path.clone(),
        request_raw_size: Some(record.req_raw.size_bytes),
        request_raw_truncated: Some(record.req_raw.truncated as i64),
        request_raw_truncated_reason: record.req_raw.truncated_reason.clone(),
        response_raw_path: record.resp_raw.path.clone(),
        response_raw_size: Some(record.resp_raw.size_bytes),
        response_raw_truncated: Some(record.resp_raw.truncated as i64),
        response_raw_truncated_reason: record.resp_raw.truncated_reason.clone(),
        detail_level: DETAIL_LEVEL_FULL.to_string(),
        detail_pruned_at: None,
        detail_prune_reason: None,
        t_total_ms: nullable_runtime_timing_value(record.timings.t_total_ms),
        t_req_read_ms: nullable_runtime_timing_value(record.timings.t_req_read_ms),
        t_req_parse_ms: nullable_runtime_timing_value(record.timings.t_req_parse_ms),
        t_upstream_connect_ms: nullable_runtime_timing_value(record.timings.t_upstream_connect_ms),
        t_upstream_ttfb_ms: nullable_runtime_timing_value(record.timings.t_upstream_ttfb_ms),
        first_token_ms: record
            .timings
            .first_token_ms
            .filter(|value| value.is_finite() && *value >= 0.0),
        t_upstream_stream_ms: nullable_runtime_timing_value(record.timings.t_upstream_stream_ms),
        t_resp_parse_ms: nullable_runtime_timing_value(record.timings.t_resp_parse_ms),
        t_persist_ms: nullable_runtime_timing_value(record.timings.t_persist_ms),
        created_at: format_utc_iso_millis(Utc::now()),
    }
}
