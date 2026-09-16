pub(crate) async fn finalize_pool_upstream_request_attempt(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    finished_at: &str,
    status: &str,
    http_status: Option<StatusCode>,
    downstream_http_status: Option<StatusCode>,
    failure_kind: Option<&str>,
    error_message: Option<&str>,
    downstream_error_message: Option<&str>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
    upstream_request_id: Option<&str>,
    compact_support_status: Option<&str>,
    compact_support_reason: Option<&str>,
) -> Result<()> {
    let _write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    let terminal_phase = terminal_pool_upstream_request_attempt_phase(status);
    let compact_support_status =
        compact_support_status.or(pending.compact_support_status.as_deref());
    let compact_support_reason =
        compact_support_reason.or(pending.compact_support_reason.as_deref());
    let upstream_request_compression_algorithm =
        pending.upstream_request_compression_algorithm.as_deref();
    let upstream_request_compression_mode = pending.upstream_request_compression_mode.as_deref();
    let upstream_request_logical_body_bytes = pending.upstream_request_logical_body_bytes;
    let upstream_request_transmitted_body_bytes = pending.upstream_request_transmitted_body_bytes;
    let upstream_request_header_bytes_approx = pending.upstream_request_header_bytes_approx;
    let upstream_response_body_bytes = pending.upstream_response_body_bytes;
    let upstream_response_header_bytes_approx = pending.upstream_response_header_bytes_approx;
    let response_raw_capture_present = pending.response_raw_path.is_some()
        || pending.response_raw_size.is_some()
        || pending.response_raw_truncated
        || pending.response_raw_truncated_reason.is_some()
        || pending.response_content_encoding.is_some();
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: pending.invoke_id.clone(),
        occurred_at: pending.occurred_at.clone(),
        endpoint: pending.endpoint.clone(),
        sticky_key: pending.sticky_key.clone(),
        requester_ip: pending.requester_ip.clone(),
        upstream_base_url_host: pending.upstream_base_url_host.clone(),
        request_model: pending.request_model.clone(),
    };
    if let Some(attempt_id) = pending.attempt_id {
        let result = sqlx::query(
            r#"
            UPDATE pool_upstream_request_attempts
            SET
                finished_at = ?2,
                status = ?3,
                phase = ?4,
                http_status = ?5,
                downstream_http_status = ?6,
                failure_kind = ?7,
                error_message = ?8,
                downstream_error_message = ?9,
                connect_latency_ms = ?10,
                first_byte_latency_ms = ?11,
                stream_latency_ms = ?12,
                upstream_request_id = ?13,
                compact_support_status = ?14,
                compact_support_reason = ?15,
                upstream_request_compression_algorithm = COALESCE(?16, upstream_request_compression_algorithm),
                upstream_request_compression_mode = COALESCE(?17, upstream_request_compression_mode),
                upstream_request_logical_body_bytes = COALESCE(?18, upstream_request_logical_body_bytes),
                upstream_request_transmitted_body_bytes = COALESCE(?19, upstream_request_transmitted_body_bytes),
                upstream_request_header_bytes_approx = COALESCE(?20, upstream_request_header_bytes_approx),
                upstream_response_body_bytes = COALESCE(?21, upstream_response_body_bytes),
                upstream_response_header_bytes_approx = COALESCE(?22, upstream_response_header_bytes_approx),
                upstream_base_url_host = COALESCE(?23, upstream_base_url_host),
                response_raw_path = COALESCE(?24, response_raw_path),
                response_raw_codec = COALESCE(?25, response_raw_codec),
                response_raw_size = COALESCE(?26, response_raw_size),
                response_raw_truncated = COALESCE(?27, response_raw_truncated),
                response_raw_truncated_reason = COALESCE(?28, response_raw_truncated_reason),
                response_content_encoding = COALESCE(?29, response_content_encoding)
            WHERE id = ?1
            "#,
        )
        .bind(attempt_id)
        .bind(finished_at)
        .bind(status)
        .bind(terminal_phase)
        .bind(http_status.map(|value| i64::from(value.as_u16())))
        .bind(downstream_http_status.map(|value| i64::from(value.as_u16())))
        .bind(failure_kind)
        .bind(error_message)
        .bind(downstream_error_message)
        .bind(connect_latency_ms)
        .bind(first_byte_latency_ms)
        .bind(stream_latency_ms)
        .bind(upstream_request_id)
        .bind(compact_support_status)
        .bind(compact_support_reason)
        .bind(upstream_request_compression_algorithm)
        .bind(upstream_request_compression_mode)
        .bind(upstream_request_logical_body_bytes)
        .bind(upstream_request_transmitted_body_bytes)
        .bind(upstream_request_header_bytes_approx)
        .bind(upstream_response_body_bytes)
        .bind(upstream_response_header_bytes_approx)
        .bind(pending.upstream_base_url_host.as_deref())
        .bind(pending.response_raw_path.as_deref())
        .bind(pending.response_raw_codec.as_deref())
        .bind(pending.response_raw_size)
        .bind(response_raw_capture_present.then_some(if pending.response_raw_truncated {
            1_i64
        } else {
            0_i64
        }))
        .bind(pending.response_raw_truncated_reason.as_deref())
        .bind(pending.response_content_encoding.as_deref())
        .execute(pool)
        .await?;

        if result.rows_affected() > 0 {
            return Ok(());
        }
    }

    let inserted_attempt_id = insert_pool_upstream_request_attempt_with_scope(
        pool,
        &trace,
        pending.group_name_snapshot.as_deref(),
        pending.proxy_binding_key_snapshot.as_deref(),
        pending.routing_source.as_deref(),
        pending.routing_selection_audit_json.as_deref(),
        Some(pending.upstream_account_id),
        Some(pending.upstream_route_key.as_str()),
        pending.attempt_index,
        pending.distinct_account_index,
        pending.same_account_retry_index,
        Some(pending.started_at.as_str()),
        Some(finished_at),
        status,
        Some(terminal_phase),
        http_status,
        downstream_http_status,
        failure_kind,
        error_message,
        downstream_error_message,
        connect_latency_ms,
        first_byte_latency_ms,
        stream_latency_ms,
        upstream_request_id,
        upstream_request_compression_algorithm,
        upstream_request_compression_mode,
        upstream_request_logical_body_bytes,
        upstream_request_transmitted_body_bytes,
        upstream_request_header_bytes_approx,
        upstream_response_body_bytes,
        upstream_response_header_bytes_approx,
        compact_support_status,
        compact_support_reason,
    )
    .await?;
    sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET
            response_raw_path = ?2,
            response_raw_codec = COALESCE(?3, response_raw_codec),
            response_raw_size = ?4,
            response_raw_truncated = ?5,
            response_raw_truncated_reason = ?6,
            response_content_encoding = ?7
        WHERE id = ?1
        "#,
    )
    .bind(inserted_attempt_id)
    .bind(pending.response_raw_path.as_deref())
    .bind(pending.response_raw_codec.as_deref())
    .bind(pending.response_raw_size)
    .bind(if pending.response_raw_truncated {
        1_i64
    } else {
        0_i64
    })
    .bind(pending.response_raw_truncated_reason.as_deref())
    .bind(pending.response_content_encoding.as_deref())
    .execute(pool)
    .await?;
    Ok(())
}

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
    let ProxyPayloadSummary {
        target,
        status,
        is_stream,
        request_contains_encrypted_content,
        response_contains_encrypted_content,
        compaction_request_kind,
        compaction_response_kind,
        image_intent,
        request_model,
        requested_service_tier,
        billing_service_tier,
        reasoning_effort,
        response_model,
        usage_missing_reason,
        request_parse_error,
        request_compression_algorithm,
        request_compression_mode,
        request_compression_logical_body_bytes,
        request_compression_transmitted_body_bytes,
        request_compression_transmission_complete,
        failure_kind,
        requester_ip,
        request_user_agent,
        request_x_forwarded_for,
        request_forwarded,
        request_x_real_ip,
        upstream_scope,
        route_mode,
        sticky_key,
        prompt_cache_key,
        prompt_cache_key_attribution_source,
        client_fingerprint,
        client_header_fingerprints,
        upstream_account_id,
        upstream_account_name,
        upstream_account_kind,
        upstream_base_url_host,
        oauth_account_header_attached,
        oauth_account_id_shape,
        oauth_forwarded_header_count,
        oauth_forwarded_header_names,
        oauth_fingerprint_version,
        oauth_forwarded_header_fingerprints,
        oauth_prompt_cache_header_forwarded,
        oauth_request_body_prefix_fingerprint,
        oauth_request_body_prefix_bytes,
        oauth_request_body_snapshot_kind,
        oauth_responses_body_mode,
        oauth_responses_rewrite,
        service_tier,
        stream_terminal_event,
        upstream_error_code,
        upstream_error_message,
        downstream_status_code,
        downstream_error_message,
        upstream_request_id,
        response_content_encoding,
        stream_failure_origin,
        upstream_read_error_kind,
        content_encoding_chain,
        forwarded_chunk_count,
        forwarded_bytes,
        usage_observed,
        downstream_close_phase,
        downstream_write_error_kind,
        last_upstream_chunk_gap_ms,
        upstream_approx_upload_bytes,
        upstream_approx_download_bytes,
        proxy_display_name,
        proxy_weight_delta,
        pool_attempt_count,
        pool_distinct_account_count,
        pool_attempt_terminal_reason,
        blocked_binding,
    } = summary;
    let payload = json!({
        "endpoint": target.endpoint(),
        "statusCode": status.as_u16(),
        "isStream": is_stream,
        "requestContainsEncryptedContent": request_contains_encrypted_content,
        "responseContainsEncryptedContent": response_contains_encrypted_content,
        "compactionRequestKind": compaction_request_kind.map(CompactionKind::as_payload_str),
        "compactionResponseKind": compaction_response_kind.map(CompactionKind::as_payload_str),
        "imageIntent": image_intent,
        "requestModel": request_model,
        "requestedServiceTier": requested_service_tier,
        "billingServiceTier": billing_service_tier,
        "reasoningEffort": reasoning_effort,
        "responseModel": response_model,
        "usageMissingReason": usage_missing_reason,
        "requestParseError": request_parse_error,
        "requestCompressionAlgorithm": request_compression_algorithm,
        "requestCompressionMode": request_compression_mode,
        "requestCompressionLogicalBodyBytes": request_compression_logical_body_bytes,
        "requestCompressionTransmittedBodyBytes": request_compression_transmitted_body_bytes,
        "requestCompressionTransmissionComplete": request_compression_transmission_complete,
        "failureKind": failure_kind,
        "requesterIp": requester_ip,
        "requestUserAgent": request_user_agent,
        "requestXForwardedFor": request_x_forwarded_for,
        "requestForwarded": request_forwarded,
        "requestXRealIp": request_x_real_ip,
        "upstreamScope": upstream_scope,
        "routeMode": route_mode,
        "stickyKey": sticky_key,
        "promptCacheKey": prompt_cache_key,
        "promptCacheKeyAttributionSource": prompt_cache_key_attribution_source,
        "clientFingerprint": client_fingerprint,
        "clientHeaderFingerprints": client_header_fingerprints,
        "upstreamAccountId": upstream_account_id,
        "upstreamAccountName": upstream_account_name,
        "upstreamAccountKind": upstream_account_kind,
        "upstreamBaseUrlHost": upstream_base_url_host,
        "oauthAccountHeaderAttached": oauth_account_header_attached,
        "oauthAccountIdShape": oauth_account_id_shape,
        "oauthForwardedHeaderCount": oauth_forwarded_header_count,
        "oauthForwardedHeaderNames": oauth_forwarded_header_names,
        "oauthFingerprintVersion": oauth_fingerprint_version,
        "oauthForwardedHeaderFingerprints": oauth_forwarded_header_fingerprints,
        "oauthPromptCacheHeaderForwarded": oauth_prompt_cache_header_forwarded,
        "oauthRequestBodyPrefixFingerprint": oauth_request_body_prefix_fingerprint,
        "oauthRequestBodyPrefixBytes": oauth_request_body_prefix_bytes,
        "oauthRequestBodySnapshotKind": oauth_request_body_snapshot_kind,
        "oauthResponsesBodyMode": oauth_responses_body_mode,
        "oauthResponsesRewrite": oauth_responses_rewrite,
        "serviceTier": service_tier,
        "streamTerminalEvent": stream_terminal_event,
        "upstreamErrorCode": upstream_error_code,
        "upstreamErrorMessage": upstream_error_message,
        "downstreamStatusCode": downstream_status_code.map(|value| value.as_u16()),
        "downstreamErrorMessage": downstream_error_message,
        "upstreamRequestId": upstream_request_id,
        "responseContentEncoding": response_content_encoding,
        "streamFailureOrigin": stream_failure_origin,
        "upstreamReadErrorKind": upstream_read_error_kind,
        "contentEncodingChain": content_encoding_chain,
        "forwardedChunkCount": forwarded_chunk_count,
        "forwardedBytes": forwarded_bytes,
        "usageObserved": usage_observed,
        "downstreamClosePhase": downstream_close_phase,
        "downstreamWriteErrorKind": downstream_write_error_kind,
        "lastUpstreamChunkGapMs": last_upstream_chunk_gap_ms,
        "upstreamApproxUploadBytes": upstream_approx_upload_bytes,
        "upstreamApproxDownloadBytes": upstream_approx_download_bytes,
        "proxyDisplayName": proxy_display_name,
        "proxyWeightDelta": proxy_weight_delta,
        "poolAttemptCount": pool_attempt_count,
        "poolDistinctAccountCount": pool_distinct_account_count,
        "poolAttemptTerminalReason": pool_attempt_terminal_reason,
        "blockedBinding": blocked_binding,
    });
    serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string())
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
