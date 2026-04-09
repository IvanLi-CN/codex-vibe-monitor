fn upstream_account_name_from_payload(payload: Option<&str>) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get("upstreamAccountName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn prompt_cache_key_from_payload(payload: Option<&str>) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get("promptCacheKey")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn sticky_key_from_payload(payload: Option<&str>) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get("stickyKey")
        .or_else(|| value.get("promptCacheKey"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn shanghai_now_string() -> String {
    format_naive(Utc::now().with_timezone(&Shanghai).naive_local())
}

fn terminal_pool_upstream_request_attempt_phase(status: &str) -> &'static str {
    if status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS {
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED
    } else {
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED
    }
}

async fn insert_pool_upstream_request_attempt(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    upstream_account_id: Option<i64>,
    upstream_route_key: Option<&str>,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    started_at: Option<&str>,
    finished_at: Option<&str>,
    status: &str,
    phase: Option<&str>,
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
) -> Result<i64> {
    let result = sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            invoke_id,
            occurred_at,
            endpoint,
            route_mode,
            sticky_key,
            upstream_account_id,
            upstream_route_key,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            requester_ip,
            started_at,
            finished_at,
            status,
            phase,
            http_status,
            downstream_http_status,
            failure_kind,
            error_message,
            downstream_error_message,
            connect_latency_ms,
            first_byte_latency_ms,
            stream_latency_ms,
            upstream_request_id,
            compact_support_status,
            compact_support_reason
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26
        )
        "#,
    )
    .bind(&trace.invoke_id)
    .bind(&trace.occurred_at)
    .bind(&trace.endpoint)
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(trace.sticky_key.as_deref())
    .bind(upstream_account_id)
    .bind(upstream_route_key)
    .bind(attempt_index)
    .bind(distinct_account_index)
    .bind(same_account_retry_index)
    .bind(trace.requester_ip.as_deref())
    .bind(started_at)
    .bind(finished_at)
    .bind(status)
    .bind(phase)
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
    .execute(pool)
    .await?;
    Ok(result.last_insert_rowid())
}

async fn begin_pool_upstream_request_attempt(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    upstream_account_id: i64,
    upstream_route_key: &str,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    started_at: &str,
) -> PendingPoolAttemptRecord {
    let attempt_id = match insert_pool_upstream_request_attempt(
        pool,
        trace,
        Some(upstream_account_id),
        Some(upstream_route_key),
        attempt_index,
        distinct_account_index,
        same_account_retry_index,
        Some(started_at),
        None,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING,
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_CONNECTING),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    {
        Ok(attempt_id) => Some(attempt_id),
        Err(err) => {
            warn!(
                invoke_id = trace.invoke_id,
                error = %err,
                "failed to persist pending pool attempt"
            );
            None
        }
    };

    PendingPoolAttemptRecord {
        attempt_id,
        invoke_id: trace.invoke_id.clone(),
        occurred_at: trace.occurred_at.clone(),
        endpoint: trace.endpoint.clone(),
        sticky_key: trace.sticky_key.clone(),
        requester_ip: trace.requester_ip.clone(),
        upstream_account_id,
        upstream_route_key: upstream_route_key.to_string(),
        attempt_index,
        distinct_account_index,
        same_account_retry_index,
        started_at: started_at.to_string(),
        connect_latency_ms: 0.0,
        first_byte_latency_ms: 0.0,
        compact_support_status: None,
        compact_support_reason: None,
    }
}

async fn update_pool_upstream_request_attempt_phase(
    pool: &Pool<Sqlite>,
    pending: &PendingPoolAttemptRecord,
    phase: &str,
) -> Result<bool> {
    let Some(attempt_id) = pending.attempt_id else {
        return Ok(false);
    };

    let result = sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET phase = ?2
        WHERE id = ?1
          AND COALESCE(phase, '') <> ?2
        "#,
    )
    .bind(attempt_id)
    .bind(phase)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

async fn advance_pool_upstream_request_attempt_phase(
    state: &AppState,
    pending: &PendingPoolAttemptRecord,
    phase: &str,
) -> Result<()> {
    if !update_pool_upstream_request_attempt_phase(&state.pool, pending, phase).await? {
        return Ok(());
    }

    broadcast_pool_upstream_attempts_snapshot(state, &pending.invoke_id).await
}

async fn recover_orphaned_pool_upstream_request_attempts(pool: &Pool<Sqlite>) -> Result<u64> {
    let finished_at = shanghai_now_string();
    let result = sqlx::query(
        r#"
        UPDATE pool_upstream_request_attempts
        SET
            finished_at = COALESCE(finished_at, ?1),
            status = ?2,
            phase = ?3,
            failure_kind = COALESCE(failure_kind, ?4),
            error_message = COALESCE(error_message, ?5)
        WHERE status = ?6
          AND finished_at IS NULL
        "#,
    )
    .bind(finished_at)
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE)
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED)
    .bind(PROXY_FAILURE_POOL_ATTEMPT_INTERRUPTED)
    .bind(POOL_ATTEMPT_INTERRUPTED_MESSAGE)
    .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

async fn recover_orphaned_proxy_invocations(pool: &Pool<Sqlite>) -> Result<u64> {
    let mut tx = pool.begin().await?;
    let rows = sqlx::query_as::<_, PersistedInvocationIdentityRow>(
        r#"
        SELECT id, status
        FROM codex_invocations
        WHERE source = ?1
          AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
        ORDER BY id ASC
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_all(tx.as_mut())
    .await?;

    if rows.is_empty() {
        tx.commit().await?;
        return Ok(0);
    }

    let affected = sqlx::query(
        r#"
        UPDATE codex_invocations
        SET status = ?1,
            error_message = ?2,
            failure_kind = ?3,
            failure_class = ?4,
            is_actionable = 1
        WHERE source = ?5
          AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
        "#,
    )
    .bind(INVOCATION_STATUS_INTERRUPTED)
    .bind(INVOCATION_INTERRUPTED_MESSAGE)
    .bind(PROXY_FAILURE_INVOCATION_INTERRUPTED)
    .bind(FAILURE_CLASS_SERVICE)
    .bind(SOURCE_PROXY)
    .execute(tx.as_mut())
    .await?
    .rows_affected();

    if affected > 0 {
        let updated_ids: Vec<i64> = rows.iter().map(|row| row.id).collect();
        recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &updated_ids).await?;
        if let Some(max_id) = updated_ids.iter().copied().max() {
            save_hourly_rollup_live_progress_tx(
                tx.as_mut(),
                HOURLY_ROLLUP_DATASET_INVOCATIONS,
                max_id,
            )
            .await?;
        }
    }

    tx.commit().await?;
    Ok(affected)
}

async fn broadcast_pool_upstream_attempts_snapshot(
    state: &AppState,
    invoke_id: &str,
) -> Result<()> {
    if state.broadcaster.receiver_count() == 0 {
        return Ok(());
    }

    let attempts = query_pool_attempt_records_from_live(&state.pool, invoke_id)
        .await
        .map_err(|err| anyhow!("failed to load live pool attempts for SSE broadcast: {err:?}"))?;
    state
        .broadcaster
        .send(BroadcastPayload::PoolAttempts {
            invoke_id: invoke_id.to_string(),
            attempts,
        })
        .map_err(|err| anyhow!("failed to broadcast pool attempts snapshot: {err}"))?;
    Ok(())
}

async fn broadcast_pool_attempt_started_runtime_snapshot(
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    runtime_snapshot: &PoolAttemptRuntimeSnapshotContext,
    account: &PoolResolvedAccount,
    attempt_count: usize,
    distinct_account_count: usize,
) {
    let running_record = build_running_proxy_capture_record(
        &trace.invoke_id,
        &trace.occurred_at,
        runtime_snapshot.capture_target,
        &runtime_snapshot.request_info,
        trace.requester_ip.as_deref(),
        trace.sticky_key.as_deref(),
        runtime_snapshot.prompt_cache_key.as_deref(),
        true,
        Some(account.account_id),
        Some(account.display_name.as_str()),
        payload_summary_upstream_account_kind(Some(account)),
        payload_summary_upstream_base_url_host(Some(account)),
        None,
        Some(attempt_count),
        Some(distinct_account_count),
        None,
        None,
        runtime_snapshot.t_req_read_ms,
        runtime_snapshot.t_req_parse_ms,
        0.0,
        0.0,
    );
    if let Err(err) =
        persist_and_broadcast_proxy_capture_runtime_snapshot(state, running_record).await
    {
        warn!(
            ?err,
            invoke_id = %trace.invoke_id,
            "failed to broadcast pool attempt start runtime snapshot"
        );
    }
    if let Err(err) = broadcast_pool_upstream_attempts_snapshot(state, &trace.invoke_id).await {
        warn!(
            invoke_id = %trace.invoke_id,
            error = %err,
            "failed to broadcast pool attempt start snapshot"
        );
    }
}

async fn finalize_pool_upstream_request_attempt(
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
    let terminal_phase = terminal_pool_upstream_request_attempt_phase(status);
    let compact_support_status =
        compact_support_status.or(pending.compact_support_status.as_deref());
    let compact_support_reason =
        compact_support_reason.or(pending.compact_support_reason.as_deref());
    let trace = PoolUpstreamAttemptTraceContext {
        invoke_id: pending.invoke_id.clone(),
        occurred_at: pending.occurred_at.clone(),
        endpoint: pending.endpoint.clone(),
        sticky_key: pending.sticky_key.clone(),
        requester_ip: pending.requester_ip.clone(),
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
                compact_support_reason = ?15
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
        .execute(pool)
        .await?;

        if result.rows_affected() > 0 {
            return Ok(());
        }
    }

    insert_pool_upstream_request_attempt(
        pool,
        &trace,
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
        compact_support_status,
        compact_support_reason,
    )
    .await
    .map(|_| ())
}

async fn insert_pool_upstream_terminal_attempt(
    pool: &Pool<Sqlite>,
    trace: &PoolUpstreamAttemptTraceContext,
    final_error: &PoolUpstreamError,
    attempt_index: i64,
    distinct_account_index: i64,
    failure_kind: &'static str,
) -> Result<()> {
    let finished_at = shanghai_now_string();
    let upstream_route_key = final_error
        .account
        .as_ref()
        .map(|account| account.upstream_route_key());
    let (http_status, downstream_http_status) = if final_error.downstream_error_message.is_some() {
        (None, Some(final_error.status))
    } else {
        (Some(final_error.status), None)
    };
    let canonical_error_message = final_error
        .canonical_error_message
        .as_deref()
        .unwrap_or(final_error.message.as_str());
    insert_pool_upstream_request_attempt(
        pool,
        trace,
        final_error
            .account
            .as_ref()
            .map(|account| account.account_id),
        upstream_route_key.as_deref(),
        attempt_index,
        distinct_account_index,
        0,
        Some(finished_at.as_str()),
        Some(finished_at.as_str()),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
        Some(POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED),
        http_status,
        downstream_http_status,
        Some(failure_kind),
        Some(canonical_error_message),
        final_error.downstream_error_message.as_deref(),
        None,
        None,
        None,
        final_error.upstream_request_id.as_deref(),
        None,
        None,
    )
    .await
    .map(|_| ())
}

async fn insert_and_broadcast_pool_upstream_terminal_attempt(
    state: &AppState,
    trace: &PoolUpstreamAttemptTraceContext,
    final_error: &PoolUpstreamError,
    attempt_index: i64,
    distinct_account_index: i64,
    failure_kind: &'static str,
) -> Result<()> {
    insert_pool_upstream_terminal_attempt(
        &state.pool,
        trace,
        final_error,
        attempt_index,
        distinct_account_index,
        failure_kind,
    )
    .await?;
    broadcast_pool_upstream_attempts_snapshot(state, &trace.invoke_id).await?;
    Ok(())
}

fn prompt_cache_upstream_account_rollup_key(
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

async fn load_hourly_rollup_live_progress(pool: &Pool<Sqlite>, dataset: &str) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress WHERE dataset = ?1",
    )
    .bind(dataset)
    .fetch_optional(pool)
    .await?
    .unwrap_or(0))
}

async fn load_hourly_rollup_live_progress_tx(
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

async fn save_hourly_rollup_live_progress_tx(
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

async fn mark_hourly_rollup_archive_replayed_tx(
    tx: &mut SqliteConnection,
    target: &str,
    dataset: &str,
    file_path: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO hourly_rollup_archive_replay (
            target,
            dataset,
            file_path,
            replayed_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        "#,
    )
    .bind(target)
    .bind(dataset)
    .bind(file_path)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn hourly_rollup_archive_replayed_tx(
    tx: &mut SqliteConnection,
    target: &str,
    dataset: &str,
    file_path: &str,
) -> Result<bool> {
    Ok(
        sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2 AND file_path = ?3 LIMIT 1",
        )
        .bind(target)
        .bind(dataset)
        .bind(file_path)
        .fetch_optional(&mut *tx)
        .await?
        .is_some(),
    )
}

fn normalized_oauth_account_id(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn looks_like_uuid_shape(value: &str) -> bool {
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

fn oauth_account_id_shape(value: Option<&str>) -> &'static str {
    match normalized_oauth_account_id(value) {
        None => "empty",
        Some(value) if value.starts_with("org_") => "org",
        Some(value) if looks_like_uuid_shape(value) => "uuid",
        Some(_) => "other",
    }
}

fn oauth_account_header_attached_for_account(
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

fn oauth_account_id_shape_for_account(
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

struct ProxyPayloadSummary<'a> {
    target: ProxyCaptureTarget,
    status: StatusCode,
    is_stream: bool,
    request_model: Option<&'a str>,
    requested_service_tier: Option<&'a str>,
    billing_service_tier: Option<&'a str>,
    reasoning_effort: Option<&'a str>,
    response_model: Option<&'a str>,
    usage_missing_reason: Option<&'a str>,
    request_parse_error: Option<&'a str>,
    failure_kind: Option<&'a str>,
    requester_ip: Option<&'a str>,
    upstream_scope: &'a str,
    route_mode: &'a str,
    sticky_key: Option<&'a str>,
    prompt_cache_key: Option<&'a str>,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&'a str>,
    upstream_account_kind: Option<&'a str>,
    upstream_base_url_host: Option<&'a str>,
    oauth_account_header_attached: Option<bool>,
    oauth_account_id_shape: Option<&'a str>,
    oauth_forwarded_header_count: Option<usize>,
    oauth_forwarded_header_names: Option<&'a [String]>,
    oauth_fingerprint_version: Option<&'a str>,
    oauth_forwarded_header_fingerprints: Option<&'a BTreeMap<String, String>>,
    oauth_prompt_cache_header_forwarded: Option<bool>,
    oauth_request_body_prefix_fingerprint: Option<&'a str>,
    oauth_request_body_prefix_bytes: Option<usize>,
    oauth_request_body_snapshot_kind: Option<&'a str>,
    oauth_responses_body_mode: Option<&'a str>,
    oauth_responses_rewrite: Option<&'a oauth_bridge::OauthResponsesRewriteSummary>,
    service_tier: Option<&'a str>,
    stream_terminal_event: Option<&'a str>,
    upstream_error_code: Option<&'a str>,
    upstream_error_message: Option<&'a str>,
    downstream_status_code: Option<StatusCode>,
    downstream_error_message: Option<&'a str>,
    upstream_request_id: Option<&'a str>,
    response_content_encoding: Option<&'a str>,
    proxy_display_name: Option<&'a str>,
    proxy_weight_delta: Option<f64>,
    pool_attempt_count: Option<usize>,
    pool_distinct_account_count: Option<usize>,
    pool_attempt_terminal_reason: Option<&'a str>,
}

fn build_proxy_payload_summary(summary: ProxyPayloadSummary<'_>) -> String {
    let ProxyPayloadSummary {
        target,
        status,
        is_stream,
        request_model,
        requested_service_tier,
        billing_service_tier,
        reasoning_effort,
        response_model,
        usage_missing_reason,
        request_parse_error,
        failure_kind,
        requester_ip,
        upstream_scope,
        route_mode,
        sticky_key,
        prompt_cache_key,
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
        proxy_display_name,
        proxy_weight_delta,
        pool_attempt_count,
        pool_distinct_account_count,
        pool_attempt_terminal_reason,
    } = summary;
    let payload = json!({
        "endpoint": target.endpoint(),
        "statusCode": status.as_u16(),
        "isStream": is_stream,
        "requestModel": request_model,
        "requestedServiceTier": requested_service_tier,
        "billingServiceTier": billing_service_tier,
        "reasoningEffort": reasoning_effort,
        "responseModel": response_model,
        "usageMissingReason": usage_missing_reason,
        "requestParseError": request_parse_error,
        "failureKind": failure_kind,
        "requesterIp": requester_ip,
        "upstreamScope": upstream_scope,
        "routeMode": route_mode,
        "stickyKey": sticky_key,
        "promptCacheKey": prompt_cache_key,
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
        "proxyDisplayName": proxy_display_name,
        "proxyWeightDelta": proxy_weight_delta,
        "poolAttemptCount": pool_attempt_count,
        "poolDistinctAccountCount": pool_distinct_account_count,
        "poolAttemptTerminalReason": pool_attempt_terminal_reason,
    });
    serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string())
}

fn invocation_status_is_in_flight(status: Option<&str>) -> bool {
    matches!(
        status
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        INVOCATION_STATUS_RUNNING | INVOCATION_STATUS_PENDING
    )
}

fn nullable_runtime_timing_value(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}

#[derive(Debug, FromRow)]
struct PersistedInvocationIdentityRow {
    id: i64,
    status: Option<String>,
}

async fn load_persisted_invocation_identity_tx(
    tx: &mut SqliteConnection,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<Option<PersistedInvocationIdentityRow>> {
    sqlx::query_as::<_, PersistedInvocationIdentityRow>(
        r#"
        SELECT id, status
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

async fn load_persisted_api_invocation_tx(
    tx: &mut SqliteConnection,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<ApiInvocation> {
    sqlx::query_as::<_, ApiInvocation>(
        r#"
        SELECT
            id,
            invoke_id,
            occurred_at,
            source,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.proxyDisplayName') END AS proxy_display_name,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.reasoningEffort') END AS reasoning_effort,
            total_tokens,
            cost,
            status,
            error_message,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.downstreamStatusCode') END AS downstream_status_code,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.endpoint') END AS endpoint,
            COALESCE(CASE WHEN json_valid(payload) THEN json_extract(payload, '$.failureKind') END, failure_kind) AS failure_kind,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.streamTerminalEvent') END AS stream_terminal_event,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamErrorCode') END AS upstream_error_code,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamErrorMessage') END AS upstream_error_message,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.downstreamErrorMessage') END AS downstream_error_message,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamRequestId') END AS upstream_request_id,
            failure_class,
            is_actionable,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.requesterIp') END AS requester_ip,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.promptCacheKey') END AS prompt_cache_key,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.routeMode') END AS route_mode,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamAccountId') END AS upstream_account_id,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamAccountName') END AS upstream_account_name,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.responseContentEncoding') END AS response_content_encoding,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.poolAttemptCount') IN ('integer', 'real')
                THEN json_extract(payload, '$.poolAttemptCount')
            END AS pool_attempt_count,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.poolDistinctAccountCount') IN ('integer', 'real')
                THEN json_extract(payload, '$.poolDistinctAccountCount')
            END AS pool_distinct_account_count,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.poolAttemptTerminalReason') = 'text'
                THEN json_extract(payload, '$.poolAttemptTerminalReason')
            END AS pool_attempt_terminal_reason,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text'
                THEN json_extract(payload, '$.requestedServiceTier')
              WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text'
                THEN json_extract(payload, '$.requested_service_tier') END AS requested_service_tier,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.serviceTier') = 'text'
                THEN json_extract(payload, '$.serviceTier')
              WHEN json_valid(payload) AND json_type(payload, '$.service_tier') = 'text'
                THEN json_extract(payload, '$.service_tier') END AS service_tier,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.billingServiceTier') = 'text'
                THEN json_extract(payload, '$.billingServiceTier')
              WHEN json_valid(payload) AND json_type(payload, '$.billing_service_tier') = 'text'
                THEN json_extract(payload, '$.billing_service_tier') END AS billing_service_tier,
            CASE WHEN json_valid(payload)
              AND json_type(payload, '$.proxyWeightDelta') IN ('integer', 'real')
              THEN json_extract(payload, '$.proxyWeightDelta') END AS proxy_weight_delta,
            cost_estimated,
            price_version,
            request_raw_path,
            request_raw_size,
            request_raw_truncated,
            request_raw_truncated_reason,
            response_raw_path,
            response_raw_size,
            response_raw_truncated,
            response_raw_truncated_reason,
            detail_level,
            detail_pruned_at,
            detail_prune_reason,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms,
            created_at
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&mut *tx)
    .await
    .map_err(Into::into)
}

async fn touch_invocation_upstream_account_last_activity_tx(
    tx: &mut SqliteConnection,
    occurred_at: &str,
    payload: Option<&str>,
) -> Result<()> {
    if let Some(upstream_account_id) = upstream_account_id_from_payload(payload) {
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET last_activity_at = CASE
                WHEN last_activity_at IS NULL OR last_activity_at < ?1 THEN ?1
                ELSE last_activity_at
            END
            WHERE id = ?2
            "#,
        )
        .bind(occurred_at)
        .bind(upstream_account_id)
        .execute(&mut *tx)
        .await?;
    }

    Ok(())
}

async fn persist_and_broadcast_proxy_capture_runtime_snapshot(
    state: &AppState,
    record: ProxyCaptureRecord,
) -> Result<()> {
    let persisted = persist_proxy_capture_runtime_record(&state.pool, record).await?;
    let Some(persisted_record) = persisted else {
        return Ok(());
    };

    if persisted_record
        .prompt_cache_key
        .as_deref()
        .is_some_and(|key| !key.trim().is_empty())
    {
        invalidate_prompt_cache_conversations_cache(&state.prompt_cache_conversation_cache).await;
    }

    if state.broadcaster.receiver_count() == 0 {
        return Ok(());
    }

    let invoke_id = persisted_record.invoke_id.clone();
    if let Err(err) = state.broadcaster.send(BroadcastPayload::Records {
        records: vec![persisted_record],
    }) {
        warn!(
            ?err,
            invoke_id = %invoke_id,
            "failed to broadcast runtime proxy capture snapshot"
        );
    }

    Ok(())
}

async fn persist_proxy_capture_runtime_record(
    pool: &Pool<Sqlite>,
    record: ProxyCaptureRecord,
) -> Result<Option<ApiInvocation>> {
    let failure = resolve_failure_classification(
        Some(record.status.as_str()),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        None,
        None,
    );
    let failure_kind = failure.failure_kind.clone();
    let t_req_read_ms = nullable_runtime_timing_value(record.timings.t_req_read_ms);
    let t_req_parse_ms = nullable_runtime_timing_value(record.timings.t_req_parse_ms);
    let t_upstream_connect_ms = nullable_runtime_timing_value(record.timings.t_upstream_connect_ms);
    let t_upstream_ttfb_ms = nullable_runtime_timing_value(record.timings.t_upstream_ttfb_ms);
    let mut tx = pool.begin().await?;
    let insert_result = sqlx::query(
        r#"
        INSERT OR IGNORE INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            total_tokens,
            cost,
            cost_estimated,
            price_version,
            status,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            raw_response,
            request_raw_path,
            request_raw_size,
            request_raw_truncated,
            request_raw_truncated_reason,
            response_raw_path,
            response_raw_size,
            response_raw_truncated,
            response_raw_truncated_reason,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19,
            ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35
        )
        "#,
    )
    .bind(&record.invoke_id)
    .bind(&record.occurred_at)
    .bind(SOURCE_PROXY)
    .bind(&record.model)
    .bind(record.usage.input_tokens)
    .bind(record.usage.output_tokens)
    .bind(record.usage.cache_input_tokens)
    .bind(record.usage.reasoning_tokens)
    .bind(record.usage.total_tokens)
    .bind(record.cost)
    .bind(record.cost_estimated as i64)
    .bind(record.price_version.as_deref())
    .bind(&record.status)
    .bind(record.error_message.as_deref())
    .bind(failure_kind.as_deref())
    .bind(failure.failure_class.as_str())
    .bind(failure.is_actionable as i64)
    .bind(record.payload.as_deref())
    .bind(&record.raw_response)
    .bind(record.req_raw.path.as_deref())
    .bind(record.req_raw.path.as_ref().map(|_| record.req_raw.size_bytes))
    .bind(record.req_raw.truncated as i64)
    .bind(record.req_raw.truncated_reason.as_deref())
    .bind(record.resp_raw.path.as_deref())
    .bind(record.resp_raw.path.as_ref().map(|_| record.resp_raw.size_bytes))
    .bind(record.resp_raw.truncated as i64)
    .bind(record.resp_raw.truncated_reason.as_deref())
    .bind(None::<f64>)
    .bind(t_req_read_ms)
    .bind(t_req_parse_ms)
    .bind(t_upstream_connect_ms)
    .bind(t_upstream_ttfb_ms)
    .bind(None::<f64>)
    .bind(None::<f64>)
    .bind(None::<f64>)
    .execute(tx.as_mut())
    .await?;

    let invocation_id = if insert_result.rows_affected() > 0 {
        let inserted_id = insert_result.last_insert_rowid();
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &[InvocationHourlySourceRecord {
                id: inserted_id,
                occurred_at: record.occurred_at.clone(),
                source: SOURCE_PROXY.to_string(),
                status: Some(record.status.clone()),
                detail_level: DETAIL_LEVEL_FULL.to_string(),
                total_tokens: record.usage.total_tokens,
                cost: record.cost,
                error_message: record.error_message.clone(),
                failure_kind: failure_kind.clone(),
                failure_class: Some(failure.failure_class.as_str().to_string()),
                is_actionable: Some(failure.is_actionable as i64),
                payload: record.payload.clone(),
                t_total_ms: None,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                t_upstream_stream_ms: None,
                t_resp_parse_ms: None,
                t_persist_ms: None,
            }],
            &INVOCATION_HOURLY_ROLLUP_TARGETS,
        )
        .await?;
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            inserted_id,
        )
        .await?;
        touch_invocation_upstream_account_last_activity_tx(
            tx.as_mut(),
            &record.occurred_at,
            record.payload.as_deref(),
        )
        .await?;
        inserted_id
    } else {
        let Some(existing) = load_persisted_invocation_identity_tx(
            tx.as_mut(),
            &record.invoke_id,
            &record.occurred_at,
        )
        .await?
        else {
            tx.commit().await?;
            return Ok(None);
        };
        if !invocation_status_is_in_flight(existing.status.as_deref()) {
            tx.commit().await?;
            return Ok(None);
        }

        let affected = sqlx::query(
            r#"
            UPDATE codex_invocations
            SET source = ?2,
                model = ?3,
                input_tokens = ?4,
                output_tokens = ?5,
                cache_input_tokens = ?6,
                reasoning_tokens = ?7,
                total_tokens = ?8,
                cost = ?9,
                cost_estimated = ?10,
                price_version = ?11,
                status = ?12,
                error_message = ?13,
                failure_kind = ?14,
                failure_class = ?15,
                is_actionable = ?16,
                payload = ?17,
                raw_response = ?18,
                request_raw_path = ?19,
                request_raw_size = ?20,
                request_raw_truncated = ?21,
                request_raw_truncated_reason = ?22,
                response_raw_path = ?23,
                response_raw_size = ?24,
                response_raw_truncated = ?25,
                response_raw_truncated_reason = ?26,
                t_total_ms = ?27,
                t_req_read_ms = ?28,
                t_req_parse_ms = ?29,
                t_upstream_connect_ms = ?30,
                t_upstream_ttfb_ms = ?31,
                t_upstream_stream_ms = ?32,
                t_resp_parse_ms = ?33,
                t_persist_ms = ?34
            WHERE id = ?1
              AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
            "#,
        )
        .bind(existing.id)
        .bind(SOURCE_PROXY)
        .bind(&record.model)
        .bind(record.usage.input_tokens)
        .bind(record.usage.output_tokens)
        .bind(record.usage.cache_input_tokens)
        .bind(record.usage.reasoning_tokens)
        .bind(record.usage.total_tokens)
        .bind(record.cost)
        .bind(record.cost_estimated as i64)
        .bind(record.price_version.as_deref())
        .bind(&record.status)
        .bind(record.error_message.as_deref())
        .bind(failure_kind.as_deref())
        .bind(failure.failure_class.as_str())
        .bind(failure.is_actionable as i64)
        .bind(record.payload.as_deref())
        .bind(&record.raw_response)
        .bind(record.req_raw.path.as_deref())
        .bind(record.req_raw.path.as_ref().map(|_| record.req_raw.size_bytes))
        .bind(record.req_raw.truncated as i64)
        .bind(record.req_raw.truncated_reason.as_deref())
        .bind(record.resp_raw.path.as_deref())
        .bind(record.resp_raw.path.as_ref().map(|_| record.resp_raw.size_bytes))
        .bind(record.resp_raw.truncated as i64)
        .bind(record.resp_raw.truncated_reason.as_deref())
        .bind(None::<f64>)
        .bind(t_req_read_ms)
        .bind(t_req_parse_ms)
        .bind(t_upstream_connect_ms)
        .bind(t_upstream_ttfb_ms)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .bind(None::<f64>)
        .execute(tx.as_mut())
        .await?
        .rows_affected();
        if affected == 0 {
            tx.commit().await?;
            return Ok(None);
        }
        recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &[existing.id]).await?;
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            existing.id,
        )
        .await?;
        touch_invocation_upstream_account_last_activity_tx(
            tx.as_mut(),
            &record.occurred_at,
            record.payload.as_deref(),
        )
        .await?;
        existing.id
    };

    let persisted = load_persisted_api_invocation_tx(tx.as_mut(), &record.invoke_id, &record.occurred_at)
        .await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        invocation_id,
    )
    .await?;
    tx.commit().await?;

    Ok(Some(persisted))
}

fn build_running_proxy_capture_record(
    invoke_id: &str,
    occurred_at: &str,
    target: ProxyCaptureTarget,
    request_info: &RequestCaptureInfo,
    requester_ip: Option<&str>,
    sticky_key: Option<&str>,
    prompt_cache_key: Option<&str>,
    pool_route_active: bool,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&str>,
    upstream_account_kind: Option<&str>,
    upstream_base_url_host: Option<&str>,
    proxy_display_name: Option<&str>,
    pool_attempt_count: Option<usize>,
    pool_distinct_account_count: Option<usize>,
    pool_attempt_terminal_reason: Option<&str>,
    response_content_encoding: Option<&str>,
    t_req_read_ms: f64,
    t_req_parse_ms: f64,
    t_upstream_connect_ms: f64,
    t_upstream_ttfb_ms: f64,
) -> ProxyCaptureRecord {
    ProxyCaptureRecord {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        model: request_info.model.clone(),
        usage: ParsedUsage::default(),
        cost: None,
        cost_estimated: false,
        price_version: None,
        status: "running".to_string(),
        error_message: None,
        failure_kind: None,
        payload: Some(build_proxy_payload_summary(ProxyPayloadSummary {
            target,
            status: StatusCode::OK,
            is_stream: request_info.is_stream,
            request_model: request_info.model.as_deref(),
            requested_service_tier: request_info.requested_service_tier.as_deref(),
            billing_service_tier: None,
            reasoning_effort: request_info.reasoning_effort.as_deref(),
            response_model: None,
            usage_missing_reason: None,
            request_parse_error: request_info.parse_error.as_deref(),
            failure_kind: None,
            requester_ip,
            upstream_scope: if pool_route_active {
                INVOCATION_UPSTREAM_SCOPE_INTERNAL
            } else {
                INVOCATION_UPSTREAM_SCOPE_EXTERNAL
            },
            route_mode: if pool_route_active {
                INVOCATION_ROUTE_MODE_POOL
            } else {
                INVOCATION_ROUTE_MODE_FORWARD_PROXY
            },
            sticky_key,
            prompt_cache_key,
            upstream_account_id,
            upstream_account_name,
            upstream_account_kind,
            upstream_base_url_host,
            oauth_account_header_attached: None,
            oauth_account_id_shape: None,
            oauth_forwarded_header_count: None,
            oauth_forwarded_header_names: None,
            oauth_fingerprint_version: None,
            oauth_forwarded_header_fingerprints: None,
            oauth_prompt_cache_header_forwarded: None,
            oauth_request_body_prefix_fingerprint: None,
            oauth_request_body_prefix_bytes: None,
            oauth_request_body_snapshot_kind: None,
            oauth_responses_body_mode: None,
            oauth_responses_rewrite: None,
            service_tier: None,
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            downstream_status_code: None,
            downstream_error_message: None,
            upstream_request_id: None,
            response_content_encoding,
            proxy_display_name,
            proxy_weight_delta: None,
            pool_attempt_count,
            pool_distinct_account_count,
            pool_attempt_terminal_reason,
        })),
        raw_response: "{}".to_string(),
        req_raw: RawPayloadMeta::default(),
        resp_raw: RawPayloadMeta::default(),
        timings: StageTimings {
            t_total_ms: 0.0,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms: 0.0,
            t_resp_parse_ms: 0.0,
            t_persist_ms: 0.0,
        },
    }
}

fn resolve_invocation_proxy_display_name(
    selected_proxy: Option<&SelectedForwardProxy>,
) -> Option<String> {
    selected_proxy.map(|proxy| proxy.display_name.clone())
}

fn summarize_response_content_encoding(content_encoding: Option<&str>) -> String {
    let encodings = parse_content_encodings(content_encoding);
    if encodings.is_empty() {
        "identity".to_string()
    } else {
        encodings.join(", ")
    }
}

#[derive(Default)]
struct RawResponsePreviewBuffer {
    bytes: Vec<u8>,
}

impl RawResponsePreviewBuffer {
    fn append(&mut self, chunk: &[u8]) {
        let remaining = RAW_RESPONSE_PREVIEW_LIMIT.saturating_sub(self.bytes.len());
        if remaining == 0 || chunk.is_empty() {
            return;
        }
        self.bytes
            .extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    fn into_preview(self) -> String {
        build_raw_response_preview(&self.bytes)
    }
}

struct BoundedResponseParseBuffer {
    bytes: Vec<u8>,
    limit: usize,
    exceeded_limit: bool,
}

impl BoundedResponseParseBuffer {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            exceeded_limit: false,
        }
    }

    fn append(&mut self, chunk: &[u8]) {
        if self.exceeded_limit || chunk.is_empty() {
            return;
        }

        let remaining = self.limit.saturating_sub(self.bytes.len());
        let take_len = remaining.min(chunk.len());
        if take_len > 0 {
            self.bytes.extend_from_slice(&chunk[..take_len]);
        }
        if take_len < chunk.len() {
            self.exceeded_limit = true;
        }
    }

    fn into_response_info(
        self,
        target: ProxyCaptureTarget,
        content_encoding: Option<&str>,
    ) -> ResponseCaptureInfo {
        let mut response_info =
            parse_target_response_payload(target, &self.bytes, false, content_encoding);
        if self.exceeded_limit {
            merge_response_capture_reason(
                &mut response_info,
                PROXY_USAGE_MISSING_NON_STREAM_PARSE_SKIPPED,
            );
        }
        response_info
    }
}

enum PendingRawPayloadWrite {
    Ready(RawPayloadMeta),
    Task(JoinHandle<RawPayloadMeta>),
}

impl PendingRawPayloadWrite {
    fn dropped(size_bytes: usize) -> Self {
        Self::Ready(RawPayloadMeta {
            path: None,
            size_bytes: size_bytes as i64,
            truncated: size_bytes > 0,
            truncated_reason: (size_bytes > 0)
                .then_some(RAW_PAYLOAD_TRUNCATED_REASON_ASYNC_BACKPRESSURE_DROPPED.to_string()),
        })
    }

    async fn finish(self) -> RawPayloadMeta {
        match self {
            Self::Ready(meta) => meta,
            Self::Task(handle) => match handle.await {
                Ok(meta) => meta,
                Err(err) => RawPayloadMeta {
                    path: None,
                    size_bytes: 0,
                    truncated: true,
                    truncated_reason: Some(format!("write_failed:{err}")),
                },
            },
        }
    }
}

fn spawn_raw_payload_file_write(
    state: &AppState,
    invoke_id: &str,
    kind: &'static str,
    bytes: Bytes,
) -> PendingRawPayloadWrite {
    if bytes.is_empty() {
        return PendingRawPayloadWrite::Ready(RawPayloadMeta::default());
    }

    let Ok(permit) = state.proxy_raw_async_semaphore.clone().try_acquire_owned() else {
        return PendingRawPayloadWrite::dropped(bytes.len());
    };

    let config = state.config.clone();
    let invoke_id = invoke_id.to_string();
    PendingRawPayloadWrite::Task(tokio::spawn(async move {
        let _permit = permit;
        store_raw_payload_file_async(&config, &invoke_id, kind, bytes).await
    }))
}

struct AsyncStreamingRawPayloadWriter {
    tx: Option<mpsc::Sender<Bytes>>,
    meta_rx: Option<oneshot::Receiver<RawPayloadMeta>>,
    observed_size_bytes: i64,
    local_truncated_reason: Option<String>,
    local_truncated: bool,
}

impl AsyncStreamingRawPayloadWriter {
    fn new(state: &AppState, invoke_id: &str, kind: &'static str) -> Self {
        let Ok(permit) = state.proxy_raw_async_semaphore.clone().try_acquire_owned() else {
            return Self {
                tx: None,
                meta_rx: None,
                observed_size_bytes: 0,
                local_truncated_reason: Some(
                    RAW_PAYLOAD_TRUNCATED_REASON_ASYNC_BACKPRESSURE_DROPPED.to_string(),
                ),
                local_truncated: true,
            };
        };

        let path = state
            .config
            .resolved_proxy_raw_dir()
            .join(format!("{invoke_id}-{kind}.bin"));
        let max_bytes = state.config.proxy_raw_max_bytes;
        let (tx, mut rx) = mpsc::channel::<Bytes>(ASYNC_STREAMING_RAW_WRITER_QUEUE_CAPACITY);
        let (meta_tx, meta_rx) = oneshot::channel();
        tokio::spawn(async move {
            let _permit = permit;
            let meta = write_streaming_raw_payload_to_file(path, max_bytes, &mut rx).await;
            let _ = meta_tx.send(meta);
        });

        Self {
            tx: Some(tx),
            meta_rx: Some(meta_rx),
            observed_size_bytes: 0,
            local_truncated_reason: None,
            local_truncated: false,
        }
    }

    fn mark_async_backpressure_dropped(&mut self) {
        self.local_truncated = true;
        self.local_truncated_reason
            .get_or_insert_with(|| RAW_PAYLOAD_TRUNCATED_REASON_ASYNC_BACKPRESSURE_DROPPED.to_string());
        self.tx = None;
    }

    fn mark_writer_closed(&mut self, message: String) {
        self.local_truncated = true;
        self.local_truncated_reason
            .get_or_insert_with(|| format!("write_failed:{message}"));
        self.tx = None;
    }

    async fn append(&mut self, bytes: Bytes) {
        if bytes.is_empty() {
            return;
        }
        self.observed_size_bytes = self.observed_size_bytes.saturating_add(bytes.len() as i64);
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
        match tx.try_send(bytes) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => self.mark_async_backpressure_dropped(),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.mark_writer_closed("async raw writer channel closed".to_string())
            }
        }
    }

    async fn finish(mut self) -> RawPayloadMeta {
        self.tx.take();
        let mut meta = match self.meta_rx.take() {
            Some(meta_rx) => match meta_rx.await {
                Ok(meta) => meta,
                Err(err) => RawPayloadMeta {
                    path: None,
                    size_bytes: self.observed_size_bytes,
                    truncated: true,
                    truncated_reason: Some(format!("write_failed:{err}")),
                },
            },
            None => RawPayloadMeta::default(),
        };
        meta.size_bytes = self.observed_size_bytes;
        if self.local_truncated {
            meta.truncated = true;
            if meta.truncated_reason.is_none() {
                meta.truncated_reason = self.local_truncated_reason;
            }
        }
        meta
    }
}

async fn write_streaming_raw_payload_to_file(
    path: PathBuf,
    max_bytes: Option<usize>,
    rx: &mut mpsc::Receiver<Bytes>,
) -> RawPayloadMeta {
    let mut meta = RawPayloadMeta::default();
    let Some(parent) = path.parent() else {
        meta.truncated = true;
        meta.truncated_reason = Some(format!(
            "write_failed:raw payload path has no parent: {}",
            path.display()
        ));
        return meta;
    };
    let mut file: Option<tokio::fs::File> = None;
    let mut written_bytes = 0usize;
    while let Some(bytes) = rx.recv().await {
        if bytes.is_empty() {
            continue;
        }
        meta.size_bytes = meta.size_bytes.saturating_add(bytes.len() as i64);

        let write_len = if let Some(limit) = max_bytes {
            let remaining = limit.saturating_sub(written_bytes);
            if remaining == 0 {
                meta.truncated = true;
                meta.truncated_reason
                    .get_or_insert_with(|| "max_bytes_exceeded".to_string());
                continue;
            }
            let write_len = remaining.min(bytes.len());
            if write_len < bytes.len() {
                meta.truncated = true;
                meta.truncated_reason
                    .get_or_insert_with(|| "max_bytes_exceeded".to_string());
            }
            write_len
        } else {
            bytes.len()
        };

        if write_len == 0 {
            continue;
        }

        if file.is_none() {
            if let Err(err) = tokio::fs::create_dir_all(parent).await {
                meta.truncated = true;
                meta.truncated_reason = Some(format!("write_failed:{err}"));
                return meta;
            }
            match tokio::fs::File::create(&path).await {
                Ok(created) => {
                    meta.path = Some(path.to_string_lossy().to_string());
                    file = Some(created);
                }
                Err(err) => {
                    meta.truncated = true;
                    meta.truncated_reason = Some(format!("write_failed:{err}"));
                    return meta;
                }
            }
        }

        if let Err(err) = file
            .as_mut()
            .expect("raw payload file should be opened before writing")
            .write_all(&bytes[..write_len])
            .await
        {
            meta.truncated = true;
            meta.truncated_reason = Some(format!("write_failed:{err}"));
            let _ = tokio::fs::remove_file(&path).await;
            meta.path = None;
            return meta;
        }
        written_bytes = written_bytes.saturating_add(write_len);
    }

    if let Some(file) = file.as_mut()
        && let Err(err) = file.flush().await
    {
        meta.truncated = true;
        meta.truncated_reason = Some(format!("write_failed:{err}"));
        let _ = tokio::fs::remove_file(&path).await;
        meta.path = None;
    }
    meta
}

fn build_raw_response_preview(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "{}".to_string();
    }
    let preview = if bytes.len() > RAW_RESPONSE_PREVIEW_LIMIT {
        &bytes[..RAW_RESPONSE_PREVIEW_LIMIT]
    } else {
        bytes
    };
    String::from_utf8_lossy(preview).to_string()
}

fn extract_error_message_from_response(bytes: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<Value>(bytes).ok()?;
    value
        .pointer("/error/message")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .or_else(|| {
            value
                .get("message")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
}

fn summarize_plaintext_upstream_error(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if text.is_empty() {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    if text.starts_with('<')
        || lower.starts_with("<!doctype")
        || lower.starts_with("<html")
        || lower.starts_with("<body")
    {
        return None;
    }
    Some(text.chars().take(240).collect())
}

fn extract_error_message_from_response_preview(bytes: &[u8]) -> Option<String> {
    extract_error_message_from_response(bytes).or_else(|| summarize_plaintext_upstream_error(bytes))
}

fn merge_response_capture_reason(
    response_info: &mut ResponseCaptureInfo,
    reason: impl Into<String>,
) {
    let reason = reason.into();
    let combined_reason = if let Some(existing) = response_info.usage_missing_reason.take() {
        format!("{reason};{existing}")
    } else {
        reason
    };
    response_info.usage_missing_reason = Some(combined_reason);
}
