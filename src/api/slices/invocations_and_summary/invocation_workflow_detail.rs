pub(crate) async fn hydrate_upstream_account_attempt_workflow_entries(
    state: &AppState,
    records: &mut [ApiPoolUpstreamRequestAttempt],
) -> Result<(), ApiError> {
    if records.is_empty() {
        return Ok(());
    }

    let selectors = records
        .iter()
        .map(|record| (record.invoke_id.clone(), record.occurred_at.clone()))
        .collect::<HashSet<_>>();
    let pricing_catalog = state.pricing_catalog.read().await.clone();
    let mut hydrated = HashMap::<
        (String, String),
        Option<(
            ApiInvocation,
            HashMap<String, InvocationWorkflowTimelineEntry>,
        )>,
    >::new();

    for (invoke_id, occurred_at) in selectors {
        let record = match load_persisted_api_invocation(&state.pool, &invoke_id, &occurred_at)
            .await
        {
            Ok(record) => record,
            Err(err) => {
                debug!(
                    invoke_id,
                    occurred_at,
                    error = %err,
                    "skipping workflow hydration for account attempt without matching invocation"
                );
                hydrated.insert((invoke_id, occurred_at), None);
                continue;
            }
        };
        let body_row = fetch_invocation_response_body_row_by_id(&state.pool, record.id).await?;
        let payload_value = body_row
            .as_ref()
            .and_then(|row| parse_optional_json_value(row.payload.as_deref()));
        let attempt_rows = query_invocation_workflow_attempt_rows(
            &state.pool,
            &record.invoke_id,
            &record.occurred_at,
        )
        .await?;
        let real_attempt_rows = attempt_rows
            .iter()
            .filter(|attempt| !invocation_workflow_attempt_row_is_pseudo_terminal(attempt))
            .collect::<Vec<_>>();
        let final_attempt_row_id = final_real_attempt_row_id(&real_attempt_rows);
        let last_success_attempt_row_id = last_success_like_attempt_row_id(&real_attempt_rows);
        let usage_cost_audit = (invocation_status_is_success_like(&record)
            && invocation_has_usage_evidence(&record))
        .then(|| build_invocation_cost_audit(&record, &pricing_catalog, true))
        .flatten();
        let workflow_entries = real_attempt_rows
            .into_iter()
            .filter_map(|attempt_row| {
                let attempt = build_workflow_attempt_from_row(
                    &record,
                    attempt_row,
                    payload_value.as_ref(),
                    (last_success_attempt_row_id == Some(attempt_row.attempt_row_id))
                        .then_some(usage_cost_audit.as_ref())
                        .flatten(),
                    final_attempt_row_id == Some(attempt_row.attempt_row_id),
                );
                let attempt_id = attempt.attempt_id.clone()?;
                Some((attempt_id, build_workflow_attempt_timeline_entry(attempt)))
            })
            .collect::<HashMap<_, _>>();
        hydrated.insert(
            (record.invoke_id.clone(), record.occurred_at.clone()),
            Some((record, workflow_entries)),
        );
    }

    for item in records {
        let Some(Some((record, workflow_entries))) =
            hydrated.get(&(item.invoke_id.clone(), item.occurred_at.clone()))
        else {
            continue;
        };
        item.invocation_record = Some(record.clone());
        item.workflow_entry = workflow_entries.get(&item.attempt_id).cloned();
    }

    Ok(())
}

async fn load_invocation_workflow_identity(
    pool: &Pool<Sqlite>,
    id: i64,
) -> Result<Option<InvocationWorkflowIdentityRow>, ApiError> {
    sqlx::query_as::<_, InvocationWorkflowIdentityRow>(
        r#"
        SELECT id, invoke_id, occurred_at, timeline_json
        FROM codex_invocations
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from)
}

async fn query_invocation_workflow_attempt_rows(
    pool: &Pool<Sqlite>,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<Vec<InvocationWorkflowAttemptRow>, ApiError> {
    sqlx::query_as::<_, InvocationWorkflowAttemptRow>(
        r#"
        SELECT
            attempts.id AS attempt_row_id,
            attempts.attempt_public_id AS attempt_id,
            attempts.invoke_id,
            attempts.occurred_at,
            attempts.endpoint,
            attempts.sticky_key,
            attempts.routing_source,
            attempts.routing_selection_audit_json,
            attempts.upstream_account_id,
            accounts.display_name AS upstream_account_name,
            attempts.upstream_route_key,
            attempts.proxy_binding_key_snapshot,
            attempts.attempt_index,
            attempts.distinct_account_index,
            attempts.same_account_retry_index,
            attempts.requester_ip,
            attempts.started_at,
            attempts.finished_at,
            attempts.status,
            COALESCE(
                attempts.phase,
                CASE
                    WHEN attempts.status = 'pending' THEN 'sending_request'
                    WHEN attempts.status = 'success' THEN 'completed'
                    ELSE 'failed'
                END
            ) AS phase,
            attempts.http_status,
            attempts.downstream_http_status,
            attempts.failure_kind,
            attempts.error_message,
            attempts.downstream_error_message,
            attempts.connect_latency_ms,
            attempts.first_byte_latency_ms,
            attempts.stream_latency_ms,
            attempts.upstream_request_id,
            attempts.upstream_request_compression_algorithm,
            attempts.upstream_request_compression_mode,
            attempts.upstream_request_logical_body_bytes,
            attempts.upstream_request_transmitted_body_bytes,
            attempts.upstream_request_header_bytes_approx,
            attempts.upstream_response_body_bytes,
            attempts.upstream_response_header_bytes_approx,
            attempts.compact_support_status,
            attempts.compact_support_reason,
            attempts.request_summary_json,
            attempts.response_summary_json,
            attempts.response_raw_path,
            attempts.response_raw_codec,
            attempts.response_raw_size,
            attempts.response_raw_truncated,
            attempts.response_raw_truncated_reason,
            attempts.response_content_encoding
        FROM pool_upstream_request_attempts AS attempts
        LEFT JOIN pool_upstream_accounts AS accounts
            ON accounts.id = attempts.upstream_account_id
        WHERE attempts.invoke_id = ?1
          AND attempts.occurred_at = ?2
        ORDER BY attempts.attempt_index ASC, attempts.id ASC
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_all(pool)
    .await
    .map_err(ApiError::from)
}

fn build_final_failure_timeline_entry(
    record: &ApiInvocation,
    body_row: Option<&InvocationResponseBodyRow>,
    raw_path_fallback_root: Option<&Path>,
) -> Option<InvocationWorkflowTimelineEntry> {
    if invocation_status_is_success_like(record) {
        return None;
    }

    let response_body = body_row.map(|row| {
        match resolve_response_body_text_from_row(row, raw_path_fallback_root) {
            Ok((text, _)) => InvocationWorkflowResponseBody {
                available: true,
                body_text: Some(text),
                unavailable_reason: None,
            },
            Err(reason) => InvocationWorkflowResponseBody {
                available: false,
                body_text: None,
                unavailable_reason: Some(reason),
            },
        }
    });

    let occurred_at = record
        .t_total_ms
        .and_then(|total| {
            parse_to_utc_datetime(&record.occurred_at).and_then(|started_at| {
                chrono::Duration::from_std(Duration::from_secs_f64(total.max(0.0) / 1000.0))
                    .ok()
                    .map(|delta| format_utc_iso(started_at + delta))
            })
        })
        .or_else(|| Some(record.occurred_at.clone()));

    Some(InvocationWorkflowTimelineEntry {
        block_id: "system-final-failure".to_string(),
        kind: "systemFinalFailure".to_string(),
        occurred_at,
        title: "Final downstream response".to_string(),
        subtitle: record
            .failure_kind
            .clone()
            .or_else(|| record.failure_class.clone()),
        status: record.status.clone(),
        attempt: None,
        detail: Some(json!({
            "invokeId": record.invoke_id.clone(),
            "downstreamStatusCode": record.downstream_status_code,
            "failureClass": record.failure_class.clone(),
            "failureKind": record.failure_kind.clone(),
            "errorMessage": record.error_message.clone(),
            "downstreamErrorMessage": record.downstream_error_message.clone(),
            "upstreamErrorCode": record.upstream_error_code.clone(),
            "upstreamErrorMessage": record.upstream_error_message.clone(),
            "upstreamRequestId": record.upstream_request_id.clone(),
            "streamTerminalEvent": record.stream_terminal_event.clone(),
            "responseContentEncoding": record.response_content_encoding.clone(),
        })),
        response_body,
    })
}

pub(crate) async fn fetch_invocation_workflow_detail(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<InvocationWorkflowDetailResponse>, ApiError> {
    let identity = load_invocation_workflow_identity(&state.pool, id)
        .await?
        .ok_or_else(|| ApiError::bad_request(anyhow!("record not found")))?;
    let record =
        load_persisted_api_invocation(&state.pool, &identity.invoke_id, &identity.occurred_at)
            .await
            .map_err(ApiError::from)?;
    let body_row = fetch_invocation_response_body_row_by_id(&state.pool, id).await?;
    let payload_value = body_row
        .as_ref()
        .and_then(|row| parse_optional_json_value(row.payload.as_deref()));
    let attempt_rows = query_invocation_workflow_attempt_rows(
        &state.pool,
        &identity.invoke_id,
        &identity.occurred_at,
    )
    .await?;
    let pseudo_attempt_rows = attempt_rows
        .iter()
        .filter(|attempt| invocation_workflow_attempt_row_is_pseudo_terminal(attempt))
        .collect::<Vec<_>>();
    let real_attempt_rows = attempt_rows
        .iter()
        .filter(|attempt| !invocation_workflow_attempt_row_is_pseudo_terminal(attempt))
        .collect::<Vec<_>>();
    let last_success_attempt_row_id = last_success_like_attempt_row_id(&real_attempt_rows);
    let pricing_catalog = state.pricing_catalog.read().await.clone();
    let usage_cost_audit = (invocation_status_is_success_like(&record)
        && invocation_has_usage_evidence(&record))
    .then(|| build_invocation_cost_audit(&record, &pricing_catalog, true))
    .flatten();
    let pool_route = normalized_runtime_text(record.route_mode.as_deref()) == "pool";
    let render_route_only = pool_route
        && !invocation_status_is_success_like(&record)
        && real_attempt_rows.is_empty()
        && (!pseudo_attempt_rows.is_empty() || record.pool_attempt_count.unwrap_or_default() == 0);
    let (route_only_attempt, attempts) = build_workflow_detail_attempts(
        &record,
        payload_value.as_ref(),
        &pseudo_attempt_rows,
        &real_attempt_rows,
        last_success_attempt_row_id,
        usage_cost_audit.as_ref(),
        render_route_only,
    );
    let failure_entry = build_final_failure_timeline_entry(
        &record,
        body_row.as_ref(),
        state.config.database_path.parent(),
    );
    let partial = pool_route
        && record.pool_attempt_count.unwrap_or_default() > 0
        && real_attempt_rows.is_empty()
        && pseudo_attempt_rows.is_empty();
    let timeline_attempt_count = attempts.len();
    let response = InvocationWorkflowDetailResponse {
        hero: build_workflow_hero(&record, payload_value.as_ref(), timeline_attempt_count),
        timeline: build_workflow_timeline_entries(
            &record,
            &attempts,
            route_only_attempt.as_ref(),
            failure_entry,
        ),
        reconstructed: identity.timeline_json.is_none(),
        partial,
        partial_reason: partial.then(|| "attempt_rows_missing".to_string()),
    };
    Ok(Json(response))
}

fn build_workflow_detail_attempts(
    record: &ApiInvocation,
    payload: Option<&Value>,
    pseudo_attempt_rows: &[&InvocationWorkflowAttemptRow],
    real_attempt_rows: &[&InvocationWorkflowAttemptRow],
    last_success_attempt_row_id: Option<i64>,
    usage_cost_audit: Option<&InvocationCostAudit>,
    render_route_only: bool,
) -> (
    Option<InvocationWorkflowAttempt>,
    Vec<InvocationWorkflowAttempt>,
) {
    let route_only_attempt = render_route_only.then(|| {
        pseudo_attempt_rows
            .last()
            .map(|attempt| build_workflow_attempt_from_row(record, attempt, payload, None, false))
            .unwrap_or_else(|| build_synthetic_workflow_attempt(record, payload, None))
    });
    let attempts = if real_attempt_rows.is_empty() {
        if route_only_attempt.is_some() {
            Vec::new()
        } else {
            vec![build_synthetic_workflow_attempt(
                record,
                payload,
                usage_cost_audit,
            )]
        }
    } else {
        let final_attempt_row_id = final_real_attempt_row_id(real_attempt_rows);
        real_attempt_rows
            .iter()
            .map(|attempt| {
                build_workflow_attempt_from_row(
                    record,
                    attempt,
                    payload,
                    (last_success_attempt_row_id == Some(attempt.attempt_row_id))
                        .then_some(usage_cost_audit)
                        .flatten(),
                    final_attempt_row_id == Some(attempt.attempt_row_id),
                )
            })
            .collect::<Vec<_>>()
    };
    (route_only_attempt, attempts)
}

#[derive(Debug, FromRow)]
pub(crate) struct InvocationResponseBodyRow {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) payload: Option<String>,
    pub(crate) raw_response: String,
    pub(crate) request_raw_path: Option<String>,
    pub(crate) request_raw_size: Option<i64>,
    pub(crate) request_raw_truncated: Option<i64>,
    pub(crate) request_raw_truncated_reason: Option<String>,
    pub(crate) response_raw_path: Option<String>,
    pub(crate) response_raw_size: Option<i64>,
    pub(crate) response_raw_truncated: Option<i64>,
    pub(crate) response_raw_truncated_reason: Option<String>,
    pub(crate) detail_level: String,
    pub(crate) detail_prune_reason: Option<String>,
    pub(crate) response_content_encoding: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) upstream_request_id: Option<String>,
    pub(crate) attempt_public_id: Option<String>,
}

pub(crate) fn is_abnormal_invocation_failure(failure_class: Option<&str>) -> bool {
    matches!(
        failure_class
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        Some("service_failure" | "client_failure" | "client_abort")
    )
}

pub(crate) fn truncate_response_preview_text(value: &str) -> (String, bool) {
    let mut end = value.len();
    let mut count = 0usize;
    for (index, _) in value.char_indices() {
        if count == INVOCATION_RESPONSE_BODY_PREVIEW_CHAR_LIMIT {
            end = index;
            break;
        }
        count += 1;
    }
    if count < INVOCATION_RESPONSE_BODY_PREVIEW_CHAR_LIMIT {
        return (value.to_string(), false);
    }
    (value[..end].to_string(), true)
}

pub(crate) fn raw_response_fallback_reason(row: &InvocationResponseBodyRow) -> String {
    if row.attempt_public_id.is_some() && row.response_raw_path.is_none() {
        "attempt_response_body_not_captured".to_string()
    } else if row.detail_level == DETAIL_LEVEL_STRUCTURED_ONLY {
        "detail_pruned".to_string()
    } else if row.response_raw_truncated.unwrap_or_default() != 0 {
        row.response_raw_truncated_reason
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(|reason| format!("preview_only:{reason}"))
            .unwrap_or_else(|| "preview_only".to_string())
    } else {
        "missing_body".to_string()
    }
}

pub(crate) fn raw_request_fallback_reason(row: &InvocationResponseBodyRow) -> String {
    if row.detail_level == DETAIL_LEVEL_STRUCTURED_ONLY {
        "detail_pruned".to_string()
    } else if row.request_raw_truncated.unwrap_or_default() != 0 {
        row.request_raw_truncated_reason
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "capture_unavailable".to_string())
    } else {
        "missing_body".to_string()
    }
}

pub(crate) fn resolve_request_body_text_from_row(
    row: &InvocationResponseBodyRow,
    raw_path_fallback_root: Option<&Path>,
) -> Result<(String, bool), String> {
    let Some(path) = row.request_raw_path.as_deref() else {
        return Err(raw_request_fallback_reason(row));
    };

    match read_proxy_raw_bytes(path, raw_path_fallback_root) {
        Ok(bytes) => Ok((String::from_utf8_lossy(&bytes).to_string(), true)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Err("raw_file_missing".to_string()),
        Err(err) => Err(format!("raw_file_unreadable:{err}")),
    }
}

pub(crate) fn resolve_response_body_text_from_row(
    row: &InvocationResponseBodyRow,
    raw_path_fallback_root: Option<&Path>,
) -> Result<(String, bool), String> {
    if let Some(path) = row.response_raw_path.as_deref() {
        match read_proxy_raw_bytes(path, raw_path_fallback_root) {
            Ok(bytes) => {
                let (decoded, _) = decode_response_payload_for_usage(
                    &bytes,
                    row.response_content_encoding.as_deref(),
                );
                return Ok((String::from_utf8_lossy(decoded.as_ref()).to_string(), true));
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let raw_preview = row.raw_response.trim();
                let preview_len = row.raw_response.len() as i64;
                if !raw_preview.is_empty()
                    && row.response_raw_size.unwrap_or(preview_len) <= preview_len
                {
                    return Ok((row.raw_response.clone(), false));
                }
                return Err("raw_file_missing".to_string());
            }
            Err(err) => {
                let raw_preview = row.raw_response.trim();
                let preview_len = row.raw_response.len() as i64;
                if !raw_preview.is_empty()
                    && row.response_raw_size.unwrap_or(preview_len) <= preview_len
                {
                    return Ok((row.raw_response.clone(), false));
                }
                return Err(format!("raw_file_unreadable:{err}"));
            }
        }
    }

    let raw_preview = row.raw_response.trim();
    if raw_preview.is_empty() {
        return Err(raw_response_fallback_reason(row));
    }

    let preview_len = row.raw_response.len() as i64;
    let fits_in_preview = row.response_raw_size.unwrap_or(preview_len) <= preview_len;
    if fits_in_preview && row.response_raw_truncated.unwrap_or_default() == 0 {
        return Ok((row.raw_response.clone(), false));
    }

    Err(raw_response_fallback_reason(row))
}

pub(crate) async fn fetch_invocation_response_body_row_by_id(
    pool: &Pool<Sqlite>,
    id: i64,
) -> Result<Option<InvocationResponseBodyRow>, ApiError> {
    let sql = format!(
        "SELECT \
         id, \
         invoke_id, \
         payload, \
         raw_response, \
         request_raw_path, \
         request_raw_size, \
         request_raw_truncated, \
         request_raw_truncated_reason, \
         response_raw_path, \
         response_raw_size, \
         response_raw_truncated, \
         response_raw_truncated_reason, \
         detail_level, \
         detail_prune_reason, \
         {response_content_encoding} AS response_content_encoding, \
         {resolved_failure} AS failure_class, \
         NULL AS upstream_request_id, \
         NULL AS attempt_public_id \
         FROM codex_invocations \
         WHERE id = ?1 \
         LIMIT 1",
        response_content_encoding = INVOCATION_RESPONSE_CONTENT_ENCODING_SQL,
        resolved_failure = INVOCATION_RESOLVED_FAILURE_CLASS_SQL,
    );

    sqlx::query_as::<_, InvocationResponseBodyRow>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(ApiError::from)
}

fn build_request_body_capture_summary(
    row: &InvocationResponseBodyRow,
    capture_source: Option<&str>,
) -> Value {
    json!({
        "source": capture_source,
        "size": row.request_raw_size,
        "truncated": row.request_raw_truncated.unwrap_or_default() != 0,
        "truncatedReason": row.request_raw_truncated_reason.clone(),
        "detailLevel": Some(row.detail_level.clone()),
        "detailPruneReason": row.detail_prune_reason.clone(),
    })
}

fn build_response_body_capture_summary(
    row: &InvocationResponseBodyRow,
    capture_source: Option<&str>,
) -> Value {
    json!({
        "source": capture_source,
        "size": row.response_raw_size,
        "truncated": row.response_raw_truncated.unwrap_or_default() != 0,
        "truncatedReason": row.response_raw_truncated_reason.clone(),
        "detailLevel": Some(row.detail_level.clone()),
        "detailPruneReason": row.detail_prune_reason.clone(),
    })
}

fn build_request_body_routing_snapshot(
    row: &InvocationResponseBodyRow,
    payload: Option<&Value>,
) -> Value {
    json!({
        "routeMode": payload_string(payload, &["routeMode"]),
        "upstreamScope": payload_string(payload, &["upstreamScope"]),
        "stickyKey": payload_string(payload, &["stickyKey"]),
        "promptCacheKey": payload_string(payload, &["promptCacheKey"]),
        "proxyDisplayName": payload_string(payload, &["proxyDisplayName"]),
        "clientFingerprint": payload_string(payload, &["clientFingerprint"]),
        "clientHeaderFingerprints": payload_clone(payload, &["clientHeaderFingerprints"]),
        "oauthForwardedHeaderNames": payload_string_array(payload, &["oauthForwardedHeaderNames"]),
        "oauthPromptCacheHeaderForwarded": payload_bool(payload, &["oauthPromptCacheHeaderForwarded"]),
        "client": build_request_client_snapshot(payload),
        "invokeId": Some(row.invoke_id.clone()),
    })
}

fn build_response_body_header_snapshot(
    row: &InvocationResponseBodyRow,
    payload: Option<&Value>,
) -> Value {
    json!({
        "contentEncoding": row
            .response_content_encoding
            .clone()
            .or_else(|| payload_string(payload, &["responseContentEncoding"])),
        "contentEncodingChain": payload_string(payload, &["contentEncodingChain"]),
        "upstreamRequestId": row
            .upstream_request_id
            .clone()
            .or_else(|| payload_string(payload, &["upstreamRequestId"])),
        "cvmInvokeId": Some(row.invoke_id.clone()),
    })
}

fn build_request_body_response(
    row: &InvocationResponseBodyRow,
    raw_path_fallback_root: Option<&Path>,
) -> InvocationResponseBodyResponse {
    let payload = parse_optional_json_value(row.payload.as_deref());
    let headers = Some(build_request_header_snapshot(payload.as_ref()));
    let routing = Some(build_request_body_routing_snapshot(row, payload.as_ref()));
    match resolve_request_body_text_from_row(row, raw_path_fallback_root) {
        Ok((body_text, from_full_body)) => InvocationResponseBodyResponse {
            available: true,
            body_text: Some(body_text),
            unavailable_reason: None,
            headers,
            routing,
            body_size: row.request_raw_size,
            body_truncated: Some(row.request_raw_truncated.unwrap_or_default() != 0),
            body_truncated_reason: row.request_raw_truncated_reason.clone(),
            detail_level: Some(row.detail_level.clone()),
            detail_prune_reason: row.detail_prune_reason.clone(),
            capture_source: Some(
                if from_full_body {
                    "raw_file"
                } else {
                    "preview"
                }
                .to_string(),
            ),
        },
        Err(reason) => InvocationResponseBodyResponse {
            available: false,
            body_text: None,
            unavailable_reason: Some(reason),
            headers,
            routing,
            body_size: row.request_raw_size,
            body_truncated: Some(row.request_raw_truncated.unwrap_or_default() != 0),
            body_truncated_reason: row.request_raw_truncated_reason.clone(),
            detail_level: Some(row.detail_level.clone()),
            detail_prune_reason: row.detail_prune_reason.clone(),
            capture_source: None,
        },
    }
}

fn build_response_body_response(
    row: &InvocationResponseBodyRow,
    raw_path_fallback_root: Option<&Path>,
) -> InvocationResponseBodyResponse {
    build_response_body_response_with_source(row, raw_path_fallback_root, None)
}

fn build_response_body_response_with_source(
    row: &InvocationResponseBodyRow,
    raw_path_fallback_root: Option<&Path>,
    capture_source_override: Option<&str>,
) -> InvocationResponseBodyResponse {
    let payload = parse_optional_json_value(row.payload.as_deref());
    let headers = Some(build_response_body_header_snapshot(row, payload.as_ref()));
    let routing = Some(build_response_delivery_snapshot(payload.as_ref()));
    match resolve_response_body_text_from_row(row, raw_path_fallback_root) {
        Ok((body_text, from_full_body)) => InvocationResponseBodyResponse {
            available: true,
            body_text: Some(body_text),
            unavailable_reason: None,
            headers,
            routing,
            body_size: row.response_raw_size,
            body_truncated: Some(row.response_raw_truncated.unwrap_or_default() != 0),
            body_truncated_reason: row.response_raw_truncated_reason.clone(),
            detail_level: Some(row.detail_level.clone()),
            detail_prune_reason: row.detail_prune_reason.clone(),
            capture_source: Some(
                if from_full_body {
                    capture_source_override.unwrap_or("raw_file")
                } else {
                    "preview"
                }
                .to_string(),
            ),
        },
        Err(reason) => InvocationResponseBodyResponse {
            available: false,
            body_text: None,
            unavailable_reason: Some(reason),
            headers,
            routing,
            body_size: row.response_raw_size,
            body_truncated: Some(row.response_raw_truncated.unwrap_or_default() != 0),
            body_truncated_reason: row.response_raw_truncated_reason.clone(),
            detail_level: Some(row.detail_level.clone()),
            detail_prune_reason: row.detail_prune_reason.clone(),
            capture_source: None,
        },
    }
}
