#[allow(clippy::too_many_arguments)]
pub(crate) async fn update_existing_proxy_invocation_record_tx(
    tx: &mut SqliteConnection,
    id: i64,
    record: &ProxyCaptureRecord,
    raw_response: &str,
    resp_raw: &RawPayloadMeta,
    failure_kind: Option<&str>,
    failure_class: &str,
    is_actionable: bool,
    t_total_ms: Option<f64>,
    t_req_read_ms: Option<f64>,
    t_req_parse_ms: Option<f64>,
    t_upstream_connect_ms: Option<f64>,
    t_upstream_ttfb_ms: Option<f64>,
    first_token_ms: Option<f64>,
    t_upstream_stream_ms: Option<f64>,
    t_resp_parse_ms: Option<f64>,
    t_persist_ms: Option<f64>,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
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
        "#,
    )
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

pub(crate) fn api_invocation_from_runtime_record(record: &ProxyCaptureRecord) -> ApiInvocation {
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
    let payload_i64 = |key: &str| {
        payload
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(crate::proxy::json_value_to_i64)
    };
    let payload_f64 = |key: &str| {
        payload
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(Value::as_f64)
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
    ApiInvocation {
        id: 0,
        invoke_id: record.invoke_id.clone(),
        occurred_at: record.occurred_at.clone(),
        source: SOURCE_PROXY.to_string(),
        proxy_display_name: payload_text("proxyDisplayName"),
        model: record.model.clone(),
        request_model: payload_text("requestModel"),
        response_model: payload_text("responseModel"),
        input_tokens: record.usage.input_tokens,
        output_tokens: record.usage.output_tokens,
        cache_input_tokens: record.usage.cache_input_tokens,
        reasoning_tokens: record.usage.reasoning_tokens,
        reasoning_effort: payload_text("reasoningEffort"),
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
        downstream_status_code: payload_i64("downstreamStatusCode"),
        failure_kind: failure
            .failure_kind
            .clone()
            .or_else(|| record.failure_kind.clone()),
        blocked_binding,
        blocked_binding_json,
        stream_terminal_event: payload_text("streamTerminalEvent"),
        upstream_error_code: payload_text("upstreamErrorCode"),
        upstream_error_message: payload_text("upstreamErrorMessage"),
        downstream_error_message: payload_text("downstreamErrorMessage"),
        upstream_request_id: payload_text("upstreamRequestId"),
        failure_class: Some(failure.failure_class.as_str().to_string()),
        is_actionable: Some(failure.is_actionable),
        endpoint: payload_text("endpoint"),
        compaction_request_kind: payload_text("compactionRequestKind"),
        compaction_response_kind: payload_text("compactionResponseKind"),
        image_intent: payload_text("imageIntent"),
        requester_ip: payload_text("requesterIp"),
        prompt_cache_key,
        sticky_key,
        route_mode: payload_text("routeMode"),
        upstream_account_id,
        upstream_account_name,
        response_content_encoding: payload_text("responseContentEncoding"),
        request_compression_algorithm: payload_text("requestCompressionAlgorithm"),
        transport: None,
        pool_attempt_count: payload_i64("poolAttemptCount"),
        pool_distinct_account_count: payload_i64("poolDistinctAccountCount"),
        pool_attempt_terminal_reason: payload_text("poolAttemptTerminalReason"),
        requested_service_tier: payload_text("requestedServiceTier"),
        service_tier: payload_text("serviceTier"),
        billing_service_tier: payload_text("billingServiceTier"),
        proxy_weight_delta: payload_f64("proxyWeightDelta"),
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

pub(crate) fn persisted_invocation_allows_proxy_record_update(
    existing_status: Option<&str>,
    existing_failure_kind: Option<&str>,
    incoming_status: &str,
) -> bool {
    invocation_status_is_in_flight(existing_status)
        || (!invocation_status_is_in_flight(Some(incoming_status))
            && invocation_status_is_recoverable_proxy_interrupted(
                existing_status,
                existing_failure_kind,
            ))
}

pub(crate) async fn load_persisted_api_invocation_tx(
    tx: &mut SqliteConnection,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<ApiInvocation> {
    let mut record = sqlx::query_as::<_, ApiInvocation>(
        r#"
        SELECT
            id,
            invoke_id,
            occurred_at,
            source,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.proxyDisplayName') END AS proxy_display_name,
            model,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.requestModel') END AS request_model,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.responseModel') END AS response_model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.reasoningEffort') END AS reasoning_effort,
        total_tokens,
        cost,
        cost_input,
        cost_cache_write,
        cost_cache_read,
        cost_output,
        cost_reasoning,
        MAX(COALESCE(input_tokens, 0) - COALESCE(cache_input_tokens, 0), 0) AS cache_write_tokens,
        status,
            error_message,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.downstreamStatusCode') END AS downstream_status_code,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.endpoint') END AS endpoint,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.compactionRequestKind') END AS compaction_request_kind,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.compactionResponseKind') END AS compaction_response_kind,
            COALESCE(CASE WHEN json_valid(payload) THEN json_extract(payload, '$.failureKind') END, failure_kind) AS failure_kind,
            CASE
              WHEN json_valid(payload) AND json_type(payload, '$.blockedBinding') = 'object'
                THEN json_extract(payload, '$.blockedBinding')
            END AS blocked_binding_json,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.streamTerminalEvent') END AS stream_terminal_event,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamErrorCode') END AS upstream_error_code,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamErrorMessage') END AS upstream_error_message,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.downstreamErrorMessage') END AS downstream_error_message,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.upstreamRequestId') END AS upstream_request_id,
            failure_class,
            is_actionable,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.requesterIp') END AS requester_ip,
            CASE WHEN json_valid(payload) THEN json_extract(payload, '$.promptCacheKey') END AS prompt_cache_key,
            CASE WHEN json_valid(payload) THEN TRIM(COALESCE(CAST(json_extract(payload, '$.stickyKey') AS TEXT), CAST(json_extract(payload, '$.promptCacheKey') AS TEXT))) END AS sticky_key,
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
            first_token_ms,
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
    .await?;
    hydrate_api_invocation_blocked_binding(&mut record);
    Ok(record)
}

pub(crate) async fn touch_invocation_upstream_account_last_activity_tx(
    tx: &mut SqliteConnection,
    occurred_at: &str,
    payload: Option<&str>,
) -> Result<()> {
    touch_upstream_account_last_activity_tx(
        tx,
        occurred_at,
        upstream_account_id_from_payload(payload),
    )
    .await
}

pub(crate) async fn touch_upstream_account_last_activity_tx(
    tx: &mut SqliteConnection,
    occurred_at: &str,
    upstream_account_id: Option<i64>,
) -> Result<()> {
    if let Some(upstream_account_id) = upstream_account_id {
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

pub(crate) async fn persist_and_broadcast_proxy_capture_runtime_snapshot(
    state: &AppState,
    record: ProxyCaptureRecord,
) -> Result<()> {
    let started = Instant::now();
    let persisted_record = api_invocation_from_runtime_record(&record);
    let invoke_id = persisted_record.invoke_id.clone();
    let occurred_at = persisted_record.occurred_at.clone();
    let store_outcome = state
        .proxy_runtime_invocations
        .upsert(persisted_record.clone());
    if store_outcome.skipped_terminal {
        let elapsed_ms = started.elapsed().as_millis() as u64;
        debug!(
            invoke_id = %invoke_id,
            occurred_at = %occurred_at,
            elapsed_ms,
            runtime_store_running_count = store_outcome.running_count,
            runtime_store_pruned_count = store_outcome.pruned_count,
            running_snapshot_db_write_skipped = true,
            running_snapshot_skipped_after_terminal = true,
            "stale running proxy capture snapshot skipped after terminal persistence"
        );
        return Ok(());
    }
    state
        .dashboard_network_speed_cache
        .observe_dashboard_activity_runtime_snapshot(&persisted_record, Utc::now());
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::invocation(
            &persisted_record,
            RuntimeMutationKind::RuntimeUpsert,
        ));
    #[cfg(test)]
    broadcast_test_record_payload(state, &persisted_record);
    schedule_dashboard_activity_live_snapshot(state);

    let elapsed_ms = started.elapsed().as_millis() as u64;
    debug!(
        invoke_id = %invoke_id,
        occurred_at = %occurred_at,
        elapsed_ms,
        runtime_store_running_count = store_outcome.running_count,
        runtime_store_pruned_count = store_outcome.pruned_count,
        running_snapshot_db_write_skipped = true,
        running_snapshot_recovery_placeholder_enqueued = false,
        "running proxy capture snapshot stored in memory and broadcast"
    );

    Ok(())
}

pub(crate) fn broadcast_proxy_capture_first_token_runtime_snapshot(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    first_token_ms: f64,
) {
    if !first_token_ms.is_finite() || first_token_ms < 0.0 {
        return;
    }
    let Some(mut record) = state
        .proxy_runtime_invocations
        .snapshot()
        .into_iter()
        .find(|record| record.invoke_id == invoke_id && record.occurred_at == occurred_at)
    else {
        return;
    };
    if record.first_token_ms.is_some() {
        return;
    }
    record.first_token_ms = Some(first_token_ms);
    let outcome = state.proxy_runtime_invocations.upsert(record.clone());
    if outcome.skipped_terminal {
        return;
    }
    state
        .dashboard_network_speed_cache
        .observe_dashboard_activity_runtime_snapshot(&record, Utc::now());
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::invocation(
            &record,
            RuntimeMutationKind::LifecyclePhase,
        ));
    #[cfg(test)]
    broadcast_test_record_payload(state, &record);
    schedule_dashboard_activity_live_snapshot(state);
}

pub(crate) fn remove_proxy_runtime_snapshot_for_terminal(
    state: &AppState,
    record: &ApiInvocation,
) -> bool {
    state
        .dashboard_network_speed_cache
        .finalize_dashboard_activity_invocation(record, Utc::now());
    state
        .dashboard_network_speed_cache
        .finish_invocation(&record.invoke_id, &record.occurred_at);
    let remove_outcome = state
        .proxy_runtime_invocations
        .upsert_terminal(record.clone());
    debug!(
        invoke_id = %record.invoke_id,
        occurred_at = %record.occurred_at,
        terminal_overlay_emitted = true,
        terminal_removed_runtime_snapshot = remove_outcome.removed,
        terminal_already_tombstoned = remove_outcome.already_terminal,
        "terminal proxy capture record stored in memory runtime overlay"
    );
    remove_outcome.already_terminal
}

pub(crate) fn remove_proxy_runtime_snapshot_by_key(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    reason: &'static str,
) -> bool {
    state
        .dashboard_network_speed_cache
        .drop_dashboard_activity_invocation(invoke_id, occurred_at);
    state
        .dashboard_network_speed_cache
        .finish_invocation(invoke_id, occurred_at);
    let removed_runtime_snapshot = state
        .proxy_runtime_invocations
        .remove_non_terminal(invoke_id, occurred_at);
    if let Some(record) = &removed_runtime_snapshot {
        state
            .subscription_hub
            .publish_runtime_mutation(RuntimeMutation::invocation(
                record,
                RuntimeMutationKind::RuntimeRemoved,
            ));
    }
    debug!(
        invoke_id,
        occurred_at,
        reason,
        terminal_removed_runtime_snapshot = removed_runtime_snapshot.is_some(),
        terminal_already_tombstoned = false,
        "non-terminal proxy runtime snapshot removed by key"
    );
    removed_runtime_snapshot.is_some()
}

pub(crate) fn terminalize_proxy_runtime_snapshot_by_key(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    reason: &'static str,
) -> bool {
    let Some(mut record) = state
        .proxy_runtime_invocations
        .remove_non_terminal(invoke_id, occurred_at)
    else {
        debug!(
            invoke_id,
            occurred_at,
            reason,
            terminal_removed_runtime_snapshot = false,
            terminal_already_tombstoned = false,
            "no non-terminal proxy runtime snapshot found for terminal cleanup"
        );
        return false;
    };

    record.status = Some(INVOCATION_STATUS_INTERRUPTED.to_string());
    record.error_message = Some(format!(
        "[{PROXY_FAILURE_INVOCATION_INTERRUPTED}] proxy request ended before a terminal record was written"
    ));
    record.failure_kind = Some(PROXY_FAILURE_INVOCATION_INTERRUPTED.to_string());
    record.failure_class = Some(FAILURE_CLASS_SERVICE.to_string());
    record.is_actionable = Some(true);
    record.pool_attempt_terminal_reason = Some(PROXY_FAILURE_INVOCATION_INTERRUPTED.to_string());
    state
        .dashboard_network_speed_cache
        .finalize_dashboard_activity_invocation(&record, Utc::now());
    state
        .dashboard_network_speed_cache
        .finish_invocation(invoke_id, occurred_at);

    let remove_outcome = state
        .proxy_runtime_invocations
        .upsert_terminal(record.clone());
    debug!(
        invoke_id,
        occurred_at,
        reason,
        terminal_removed_runtime_snapshot = true,
        terminal_already_tombstoned = remove_outcome.already_terminal,
        terminal_delta_skipped_runtime_only = true,
        "non-terminal proxy runtime snapshot terminalized by key"
    );
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::invocation(
            &record,
            RuntimeMutationKind::TerminalCommitted,
        ));
    #[cfg(test)]
    broadcast_test_record_payload(state, &record);
    true
}

pub(crate) fn terminalize_proxy_runtime_snapshot_with_error(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
    status: StatusCode,
    failure_kind: &'static str,
    error_message: &str,
    reason: &'static str,
) -> bool {
    let Some(mut record) = state
        .proxy_runtime_invocations
        .remove_non_terminal(invoke_id, occurred_at)
    else {
        debug!(
            invoke_id,
            occurred_at,
            reason,
            terminal_overlay_emitted = false,
            terminal_removed_runtime_snapshot = false,
            "no non-terminal proxy runtime snapshot found for terminal error overlay"
        );
        return false;
    };

    record.status = Some(if status.is_server_error() {
        format!("http_{}", status.as_u16())
    } else {
        "failed".to_string()
    });
    record.error_message = Some(format!("[{failure_kind}] {error_message}"));
    record.failure_kind = Some(failure_kind.to_string());
    record.failure_class = Some(
        if status.is_client_error() {
            FAILURE_CLASS_CLIENT
        } else {
            FAILURE_CLASS_SERVICE
        }
        .to_string(),
    );
    record.is_actionable = Some(true);
    record.pool_attempt_terminal_reason = Some(failure_kind.to_string());
    state
        .dashboard_network_speed_cache
        .finalize_dashboard_activity_invocation(&record, Utc::now());
    state
        .dashboard_network_speed_cache
        .finish_invocation(invoke_id, occurred_at);

    let remove_outcome = state
        .proxy_runtime_invocations
        .upsert_terminal(record.clone());
    debug!(
        invoke_id,
        occurred_at,
        reason,
        status = %status,
        failure_kind,
        terminal_overlay_emitted = true,
        terminal_removed_runtime_snapshot = true,
        terminal_already_tombstoned = remove_outcome.already_terminal,
        terminal_delta_skipped_runtime_only = true,
        "non-terminal proxy runtime snapshot terminalized with error overlay"
    );
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::invocation(
            &record,
            RuntimeMutationKind::TerminalCommitted,
        ));
    #[cfg(test)]
    broadcast_test_record_payload(state, &record);
    true
}

pub(crate) async fn observe_successful_proxy_capture_model_route_cache(
    state: &AppState,
    record: &ProxyCaptureRecord,
) {
    if record.status != "success" {
        return;
    }

    let metadata = terminal_payload_metadata(record.payload.as_deref());
    match observe_proxy_cache_hit_if_success(state, record).await {
        Ok(outcome) => {
            if outcome.observed {
                state
                    .subscription_hub
                    .publish_runtime_mutation(RuntimeMutation::ModelRoutingChanged);
            }
            if outcome.availability_increased {
                let account_allows_publish = match metadata.upstream_account_id {
                    Some(account_id) => match pool_account_allows_model_route_availability_publish(
                        &state.pool,
                        account_id,
                    )
                    .await
                    {
                        Ok(allowed) => allowed,
                        Err(err) => {
                            warn!(
                                invoke_id = %record.invoke_id,
                                account_id,
                                error = %err,
                                "failed to verify account fence before publishing model route availability"
                            );
                            false
                        }
                    },
                    None => false,
                };
                if account_allows_publish {
                    publish_pool_routing_availability(state);
                } else {
                    debug!(
                        invoke_id = %record.invoke_id,
                        upstream_account_id = metadata.upstream_account_id,
                        "model cache observation increased capacity without publishing because the account remains fenced"
                    );
                }
            }
        }
        Err(err) => {
            warn!(
                invoke_id = %record.invoke_id,
                error = %err,
                "failed to observe model route cache hit"
            );
        }
    }
}
