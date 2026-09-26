use super::*;

#[derive(Debug, FromRow)]
pub(crate) struct PersistedInvocationIdentityRow {
    pub(crate) id: i64,
    pub(crate) status: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) source: String,
    pub(crate) model: Option<String>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reported_cache_write_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) error_message: Option<String>,
    pub(crate) payload: Option<String>,
}

pub(crate) async fn load_persisted_invocation_identity_tx(
    tx: &mut SqliteConnection,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<Option<PersistedInvocationIdentityRow>> {
    sqlx::query_as::<_, PersistedInvocationIdentityRow>(
        r#"
        SELECT id, status, failure_kind, source, model, input_tokens, output_tokens,
               cache_input_tokens, reported_cache_write_tokens, reasoning_tokens,
               total_tokens, error_message, payload
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

pub(crate) fn websocket_terminal_usage_refresh_allowed(
    existing: &PersistedInvocationIdentityRow,
    incoming: &ProxyCaptureRecord,
) -> bool {
    let Some(existing_status) = existing.status.as_deref() else {
        return false;
    };
    matches!(
        existing_status.trim().to_ascii_lowercase().as_str(),
        "success" | "failed"
    ) && existing_status.eq_ignore_ascii_case(&incoming.status)
        && existing.failure_kind.as_deref() == incoming.failure_kind.as_deref()
        && existing.error_message == incoming.error_message
        && existing.source.eq_ignore_ascii_case(SOURCE_PROXY)
        && websocket_terminal_payload(existing.payload.as_deref())
        && websocket_terminal_payload(incoming.payload.as_deref())
        && websocket_usage_is_strictly_richer(
            &ParsedUsage {
                input_tokens: existing.input_tokens,
                output_tokens: existing.output_tokens,
                cache_input_tokens: existing.cache_input_tokens,
                reported_cache_write_tokens: existing.reported_cache_write_tokens,
                reasoning_tokens: existing.reasoning_tokens,
                total_tokens: existing.total_tokens,
            },
            &incoming.usage,
        )
}

pub(crate) fn preserve_websocket_terminal_rollup_metadata(
    record: &mut ProxyCaptureRecord,
    existing: &PersistedInvocationIdentityRow,
) {
    record.model = record.model.take().or_else(|| existing.model.clone());
    record.status = existing
        .status
        .clone()
        .unwrap_or_else(|| record.status.clone());
    record.failure_kind = existing.failure_kind.clone();
    record.error_message = existing.error_message.clone();
    record.payload = merge_websocket_terminal_payload_metadata(
        existing.payload.as_deref(),
        record.payload.as_deref(),
    )
    .or_else(|| existing.payload.clone());
}

fn merge_websocket_terminal_payload_metadata(
    existing_payload: Option<&str>,
    incoming_payload: Option<&str>,
) -> Option<String> {
    let mut existing = serde_json::from_str::<Value>(existing_payload?).ok()?;
    let incoming = serde_json::from_str::<Value>(incoming_payload?).ok()?;
    let existing_object = existing.as_object_mut()?;
    let incoming_object = incoming.as_object()?;

    for key in ["serviceTier", "billingServiceTier"] {
        if let Some(value) = incoming_object.get(key).filter(|value| !value.is_null()) {
            existing_object.insert(key.to_string(), value.clone());
        }
    }

    serde_json::to_string(&existing).ok()
}

pub(crate) fn websocket_terminal_payload(payload: Option<&str>) -> bool {
    payload
        .and_then(|payload| serde_json::from_str::<Value>(payload).ok())
        .is_some_and(|value| {
            value
                .get("transport")
                .and_then(Value::as_str)
                .is_some_and(|transport| transport.eq_ignore_ascii_case("websocket"))
                && value
                    .get("streamTerminalEvent")
                    .and_then(Value::as_str)
                    .is_some_and(|event| !event.trim().is_empty())
        })
}

fn websocket_refresh_has_unsupported_actual_tier(payload: Option<&str>) -> bool {
    let Some(value) = payload.and_then(|payload| serde_json::from_str::<Value>(payload).ok())
    else {
        return false;
    };
    let tier = ["billingServiceTier", "serviceTier"]
        .into_iter()
        .find_map(|key| value.get(key).and_then(Value::as_str))
        .and_then(normalize_service_tier);
    tier.is_some_and(|tier| !matches!(tier.as_str(), "standard" | "default" | "fast" | "priority"))
}

pub(crate) async fn refresh_websocket_terminal_usage_tx(
    tx: &mut SqliteConnection,
    id: i64,
    existing: &PersistedInvocationIdentityRow,
    incoming: &ProxyCaptureRecord,
) -> Result<bool> {
    let preserve_existing_cost = incoming.cost.is_none()
        && !websocket_refresh_has_unsupported_actual_tier(incoming.payload.as_deref());
    let result = sqlx::query(
        r#"
        UPDATE codex_invocations
        SET
            model = COALESCE(?2, model),
            input_tokens = COALESCE(?3, input_tokens),
            output_tokens = COALESCE(?4, output_tokens),
            cache_input_tokens = COALESCE(?5, cache_input_tokens),
            reported_cache_write_tokens = COALESCE(?6, reported_cache_write_tokens),
            reasoning_tokens = COALESCE(?7, reasoning_tokens),
            total_tokens = COALESCE(?8, total_tokens),
            cost = CASE WHEN ?17 = 1 THEN COALESCE(?9, cost) ELSE ?9 END,
            cost_input = CASE WHEN ?17 = 1 THEN COALESCE(?10, cost_input) ELSE ?10 END,
            cost_cache_write = CASE WHEN ?17 = 1 THEN COALESCE(?11, cost_cache_write) ELSE ?11 END,
            cost_cache_read = CASE WHEN ?17 = 1 THEN COALESCE(?12, cost_cache_read) ELSE ?12 END,
            cost_output = CASE WHEN ?17 = 1 THEN COALESCE(?13, cost_output) ELSE ?13 END,
            cost_reasoning = CASE WHEN ?17 = 1 THEN COALESCE(?14, cost_reasoning) ELSE ?14 END,
            cost_estimated = CASE WHEN ?17 = 1 THEN cost_estimated ELSE ?15 END,
            price_version = CASE WHEN ?17 = 1 THEN COALESCE(?16, price_version) ELSE ?16 END
        WHERE id = ?1
          AND source = ?18
          AND LOWER(TRIM(COALESCE(status, ''))) = LOWER(?19)
          AND failure_kind IS ?20
          AND error_message IS ?21
          AND input_tokens IS ?22
          AND output_tokens IS ?23
          AND cache_input_tokens IS ?24
          AND reported_cache_write_tokens IS ?25
          AND reasoning_tokens IS ?26
          AND total_tokens IS ?27
          AND json_valid(payload)
          AND LOWER(TRIM(COALESCE(json_extract(payload, '$.transport'), ''))) = 'websocket'
          AND json_extract(payload, '$.streamTerminalEvent') IS NOT NULL
        "#,
    )
    .bind(id)
    .bind(incoming.model.as_deref())
    .bind(incoming.usage.input_tokens)
    .bind(incoming.usage.output_tokens)
    .bind(incoming.usage.cache_input_tokens)
    .bind(incoming.usage.reported_cache_write_tokens)
    .bind(incoming.usage.reasoning_tokens)
    .bind(incoming.usage.total_tokens)
    .bind(incoming.cost)
    .bind(incoming.cost_breakdown.map(|cost| cost.input))
    .bind(incoming.cost_breakdown.map(|cost| cost.cache_write))
    .bind(incoming.cost_breakdown.map(|cost| cost.cache_read))
    .bind(incoming.cost_breakdown.map(|cost| cost.output))
    .bind(incoming.cost_breakdown.map(|cost| cost.reasoning))
    .bind(incoming.cost_estimated as i64)
    .bind(incoming.price_version.as_deref())
    .bind(preserve_existing_cost as i64)
    .bind(SOURCE_PROXY)
    .bind(existing.status.as_deref())
    .bind(existing.failure_kind.as_deref())
    .bind(existing.error_message.as_deref())
    .bind(existing.input_tokens)
    .bind(existing.output_tokens)
    .bind(existing.cache_input_tokens)
    .bind(existing.reported_cache_write_tokens)
    .bind(existing.reasoning_tokens)
    .bind(existing.total_tokens)
    .execute(&mut *tx)
    .await?;

    if result.rows_affected() == 0 {
        return Ok(false);
    }

    if let Some(payload) = incoming.payload.as_deref() {
        sqlx::query("UPDATE codex_invocations SET payload = ?1 WHERE id = ?2")
            .bind(payload)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    Ok(true)
}

pub(crate) async fn enqueue_websocket_terminal_usage_refresh(
    state: &AppState,
    record: ProxyCaptureRecord,
) -> Result<bool> {
    let persisted = api_invocation_from_runtime_record(&record);
    let invoke_id = persisted.invoke_id.clone();
    let enqueue = state
        .sqlite_batch_writer
        .enqueue_terminal(BatchedTerminalInvocationWrite {
            record,
            capture_started: None,
            raw_capture: false,
            dashboard_terminal_sequence: None,
            terminal_projection_event_ids: Vec::new(),
            startup_backfill_tasks: startup_backfill_tasks_for_terminal(&persisted),
        });
    if enqueue.enqueued {
        schedule_proxy_capture_follow_up_after_terminal_enqueue(
            state,
            &invoke_id,
            "websocket_terminal_usage_refresh",
        );
    }
    Ok(enqueue.enqueued)
}
