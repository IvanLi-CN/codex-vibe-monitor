#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocateInvocationResponse {
    pub(crate) anchor_id: String,
    pub(crate) snapshot_id: i64,
    pub(crate) invoke_id: String,
    pub(crate) attempt_id: Option<String>,
    pub(crate) total: i64,
    pub(crate) page: i64,
    pub(crate) page_size: i64,
    pub(crate) records: Vec<ApiInvocation>,
    pub(crate) target_index: usize,
    pub(crate) target_absolute_index: i64,
}

#[derive(Debug, FromRow)]
struct InvocationLocateRow {
    id: i64,
    occurred_at: String,
}

#[derive(Debug, FromRow)]
struct InvocationLocateAttemptRow {
    invoke_id: String,
    upstream_account_id: Option<i64>,
}

pub(crate) async fn locate_invocation_page(
    state: Arc<AppState>,
    params: &LocateInvocationQuery,
) -> Result<Option<LocateInvocationResponse>, ApiError> {
    let Some(context) = resolve_invocation_locate_context(state.as_ref(), params).await? else {
        return Ok(None);
    };
    let db_rank = query_invocation_locate_db_rank(state.as_ref(), &context).await?;
    let rank = load_invocation_locate_rank(state.as_ref(), &context, db_rank).await?;
    let candidate_pages = [
        rank.target_page,
        rank.target_page.saturating_sub(1).max(1),
        rank.target_page.saturating_add(1),
    ];
    for page in candidate_pages {
        let Json(response) = list_invocations_with_runtime_overlay(
            state.clone(),
            ListQuery {
                page: Some(page),
                page_size: Some(context.page_size),
                snapshot_id: Some(context.snapshot_id),
                sort_by: Some("occurredAt".to_string()),
                sort_order: Some("desc".to_string()),
                upstream_account_id: context.upstream_account_id,
                ..Default::default()
            },
            Some(rank.anchor_runtime_records.clone()),
        )
        .await?;
        if let Some(target_index) = response.records.iter().position(|record| {
            runtime_text_equals(Some(record.invoke_id.as_str()), context.request_id.as_str())
        }) {
            let anchor_id = store_invocation_anchor_snapshot(
                context.snapshot_id,
                context.upstream_account_id,
                rank.anchor_runtime_records,
            );
            return Ok(Some(LocateInvocationResponse {
                anchor_id,
                snapshot_id: response.snapshot_id,
                invoke_id: context.request_id,
                attempt_id: context.resolved_attempt_id,
                total: response.total,
                page: response.page,
                page_size: response.page_size,
                records: response.records,
                target_index,
                target_absolute_index: (response.page - 1) * response.page_size
                    + target_index as i64,
            }));
        }
    }

    Ok(None)
}

struct InvocationLocateContext {
    request_id: String,
    resolved_attempt_id: Option<String>,
    upstream_account_id: Option<i64>,
    page_size: i64,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
    base_filters: InvocationRecordsFilters,
    runtime_records: Vec<ApiInvocation>,
    target_occurred_at: String,
    target_id: i64,
}

struct InvocationLocateRank {
    anchor_runtime_records: Vec<ApiInvocation>,
    target_page: i64,
}

async fn resolve_invocation_locate_context(
    state: &AppState,
    params: &LocateInvocationQuery,
) -> Result<Option<InvocationLocateContext>, ApiError> {
    let request_id = normalize_query_text(params.invoke_id.as_deref())
        .or_else(|| normalize_query_text(params.request_id.as_deref()));
    let attempt_id = normalize_query_text(params.attempt_id.as_deref());
    if request_id.is_none() && attempt_id.is_none() {
        return Err(ApiError::bad_request(anyhow!(
            "invokeId or attemptId is required"
        )));
    }
    let upstream_account_id = match params.upstream_account_id {
        Some(value) if value > 0 => Some(value),
        Some(_) => {
            return Err(ApiError::bad_request(anyhow!(
                "upstreamAccountId must be positive"
            )));
        }
        None => None,
    };

    let page_size = params
        .page_size
        .unwrap_or(50)
        .clamp(1, state.config.list_limit_max as i64);
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
    let Some((request_id, resolved_attempt_id, upstream_account_id)) =
        resolve_invocation_locate_request_identity(
            state,
            request_id,
            attempt_id,
            upstream_account_id,
        )
        .await?
    else {
        return Ok(None);
    };
    let base_filters = InvocationRecordsFilters {
        upstream_account_id,
        ..Default::default()
    };
    let runtime_records = runtime_overlay_snapshot(state);
    let runtime_target = runtime_records.iter().find(|record| {
        runtime_text_equals(Some(record.invoke_id.as_str()), request_id.as_str())
            && runtime_record_matches_filters(record, &base_filters, source_scope)
    });

    let mut target_query =
        QueryBuilder::<Sqlite>::new("SELECT id, occurred_at FROM codex_invocations WHERE 1 = 1");
    apply_invocation_records_filters(
        &mut target_query,
        &base_filters,
        source_scope,
        Some(SnapshotConstraint::UpTo(snapshot_id)),
    );
    target_query
        .push(" AND invoke_id = ")
        .push_bind(request_id.clone())
        .push(" ORDER BY occurred_at DESC, id DESC LIMIT 1");
    let db_target = target_query
        .build_query_as::<InvocationLocateRow>()
        .fetch_optional(&state.pool)
        .await?;

    let (target_occurred_at, target_id) = if let Some(row) = db_target {
        (row.occurred_at, row.id)
    } else if let Some(record) = runtime_target {
        (record.occurred_at.clone(), record.id)
    } else {
        return Ok(None);
    };
    Ok(Some(InvocationLocateContext {
        request_id,
        resolved_attempt_id,
        upstream_account_id,
        page_size,
        source_scope,
        snapshot_id,
        base_filters,
        runtime_records,
        target_occurred_at,
        target_id,
    }))
}

async fn resolve_invocation_locate_request_identity(
    state: &AppState,
    request_id: Option<String>,
    attempt_id: Option<String>,
    upstream_account_id: Option<i64>,
) -> Result<Option<(String, Option<String>, Option<i64>)>, ApiError> {
    let Some(attempt_public_id) = attempt_id else {
        return Ok(Some((
            request_id.unwrap_or_default(),
            None,
            upstream_account_id,
        )));
    };
    let target = sqlx::query_as::<_, InvocationLocateAttemptRow>(
        r#"
        SELECT invoke_id, upstream_account_id
        FROM pool_upstream_request_attempts
        WHERE attempt_public_id = ?1
        LIMIT 1
        "#,
    )
    .bind(&attempt_public_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some(target) = target else {
        return Ok(None);
    };
    if upstream_account_id.is_some() && upstream_account_id != target.upstream_account_id {
        return Ok(None);
    }
    Ok(Some((
        target.invoke_id,
        Some(attempt_public_id),
        upstream_account_id.or(target.upstream_account_id),
    )))
}

async fn query_invocation_locate_db_rank(
    state: &AppState,
    context: &InvocationLocateContext,
) -> Result<i64, ApiError> {
    #[derive(Debug, FromRow)]
    struct CountRow {
        total: i64,
    }

    let mut rank_query =
        QueryBuilder::<Sqlite>::new("SELECT COUNT(*) AS total FROM codex_invocations WHERE 1 = 1");
    apply_invocation_records_filters(
        &mut rank_query,
        &context.base_filters,
        context.source_scope,
        Some(SnapshotConstraint::UpTo(context.snapshot_id)),
    );
    rank_query
        .push(" AND (occurred_at > ")
        .push_bind(context.target_occurred_at.clone())
        .push(" OR (occurred_at = ")
        .push_bind(context.target_occurred_at.clone())
        .push(" AND id > ")
        .push_bind(context.target_id)
        .push("))");
    Ok(rank_query
        .build_query_as::<CountRow>()
        .fetch_one(&state.pool)
        .await?
        .total)
}

async fn load_invocation_locate_rank(
    state: &AppState,
    context: &InvocationLocateContext,
    db_rank: i64,
) -> Result<InvocationLocateRank, ApiError> {
    let snapshot = Some(SnapshotConstraint::UpTo(context.snapshot_id));
    let db_terminal_keys = if context.runtime_records.is_empty() {
        HashSet::new()
    } else {
        query_terminal_db_keys_for_runtime_records(&state.pool, &context.runtime_records, snapshot)
            .await?
    };
    let db_runtime_keys = if context.runtime_records.is_empty() {
        HashSet::new()
    } else {
        query_current_runtime_db_keys(
            &state.pool,
            &context.base_filters,
            context.source_scope,
            snapshot,
        )
        .await?
    };
    let anchor_runtime_records = context
        .runtime_records
        .iter()
        .filter(|record| {
            let key = (record.invoke_id.clone(), record.occurred_at.clone());
            db_runtime_keys.contains(&key)
                || runtime_record_matches_filters(
                    record,
                    &context.base_filters,
                    context.source_scope,
                )
        })
        .cloned()
        .collect::<Vec<_>>();
    let runtime_by_key = context
        .runtime_records
        .iter()
        .map(|record| {
            (
                (record.invoke_id.clone(), record.occurred_at.clone()),
                record,
            )
        })
        .collect::<HashMap<_, _>>();
    let stale_db_runtime_keys = db_runtime_keys
        .iter()
        .filter(|key| {
            runtime_by_key.get(*key).is_some_and(|record| {
                !runtime_record_matches_filters(record, &context.base_filters, context.source_scope)
            })
        })
        .cloned()
        .collect::<HashSet<_>>();
    let stale_db_runtime_before_target = count_stale_runtime_db_rows_before_target(
        &state.pool,
        &stale_db_runtime_keys,
        &context.target_occurred_at,
        context.target_id,
        context.snapshot_id,
    )
    .await?;
    let runtime_new_before_target = context
        .runtime_records
        .iter()
        .filter(|record| {
            let key = (record.invoke_id.clone(), record.occurred_at.clone());
            !db_runtime_keys.contains(&key)
                && !db_terminal_keys.contains(&key)
                && runtime_record_matches_filters(
                    record,
                    &context.base_filters,
                    context.source_scope,
                )
                && (record.occurred_at.as_str() > context.target_occurred_at.as_str()
                    || (record.occurred_at.as_str() == context.target_occurred_at.as_str()
                        && record.id > context.target_id))
        })
        .count() as i64;
    let target_absolute_index = db_rank
        .saturating_sub(stale_db_runtime_before_target)
        .saturating_add(runtime_new_before_target);
    Ok(InvocationLocateRank {
        anchor_runtime_records,
        target_page: target_absolute_index / context.page_size + 1,
    })
}

pub(crate) async fn locate_invocation(
    State(state): State<Arc<AppState>>,
    Query(params): Query<LocateInvocationQuery>,
) -> Result<axum::response::Response, ApiError> {
    match locate_invocation_page(state, &params).await? {
        Some(response) => Ok(Json(response).into_response()),
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(json!({
                "code": "invocation_not_found",
                "message": "invocation record not found",
                "invokeId": normalize_query_text(params.invoke_id.as_deref())
                    .or_else(|| normalize_query_text(params.request_id.as_deref())),
                "attemptId": normalize_query_text(params.attempt_id.as_deref()),
            })),
        )
            .into_response()),
    }
}

pub(crate) async fn fetch_invocation_pool_attempts(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(invoke_id): axum::extract::Path<String>,
) -> Result<Json<Vec<ApiPoolUpstreamRequestAttempt>>, ApiError> {
    Ok(Json(
        query_pool_attempt_records_from_live(&state.pool, &invoke_id).await?,
    ))
}

#[derive(Debug, FromRow)]
struct InvocationWorkflowIdentityRow {
    id: i64,
    invoke_id: String,
    occurred_at: String,
    timeline_json: Option<String>,
}

#[derive(Debug, FromRow)]
struct InvocationWorkflowAttemptRow {
    attempt_row_id: i64,
    attempt_id: Option<String>,
    invoke_id: String,
    occurred_at: String,
    endpoint: String,
    sticky_key: Option<String>,
    routing_source: Option<String>,
    routing_selection_audit_json: Option<String>,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<String>,
    upstream_route_key: Option<String>,
    proxy_binding_key_snapshot: Option<String>,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    requester_ip: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
    status: String,
    phase: Option<String>,
    http_status: Option<i64>,
    downstream_http_status: Option<i64>,
    failure_kind: Option<String>,
    error_message: Option<String>,
    downstream_error_message: Option<String>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
    upstream_request_id: Option<String>,
    upstream_request_compression_algorithm: Option<String>,
    upstream_request_compression_mode: Option<String>,
    upstream_request_logical_body_bytes: Option<i64>,
    upstream_request_transmitted_body_bytes: Option<i64>,
    upstream_request_header_bytes_approx: Option<i64>,
    upstream_response_body_bytes: Option<i64>,
    upstream_response_header_bytes_approx: Option<i64>,
    compact_support_status: Option<String>,
    compact_support_reason: Option<String>,
    request_summary_json: Option<String>,
    response_summary_json: Option<String>,
    response_raw_path: Option<String>,
    response_raw_codec: Option<String>,
    response_raw_size: Option<i64>,
    response_raw_truncated: Option<i64>,
    response_raw_truncated_reason: Option<String>,
    response_content_encoding: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationWorkflowDetailResponse {
    pub(crate) hero: InvocationWorkflowHero,
    pub(crate) timeline: Vec<InvocationWorkflowTimelineEntry>,
    pub(crate) reconstructed: bool,
    pub(crate) partial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) partial_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationWorkflowHero {
    pub(crate) record_id: i64,
    pub(crate) invoke_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) prompt_cache_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) route_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) request_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) final_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) failure_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) downstream_status_code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) upstream_account_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) upstream_account_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(serialize_with = "serialize_opt_finite_nonnegative_timing")]
    pub(crate) total_duration_ms: Option<f64>,
    pub(crate) timeline_attempt_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pool_attempt_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) total_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) occurred_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pool_routing_no_candidate_audit: Option<PoolRoutingNoCandidateAudit>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationWorkflowAttempt {
    pub(crate) synthetic: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) attempt_id: Option<String>,
    pub(crate) occurred_at: String,
    pub(crate) endpoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) sticky_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) routing_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) routing_selection_audit: Option<PoolRoutingSelectionAudit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) upstream_account_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) upstream_account_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) request_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) upstream_route_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) proxy_binding_key_snapshot: Option<String>,
    pub(crate) attempt_index: i64,
    pub(crate) distinct_account_index: i64,
    pub(crate) same_account_retry_index: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) requester_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) finished_at: Option<String>,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) http_status: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) downstream_http_status: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) failure_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) downstream_error_message: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_opt_finite_nonnegative_timing"
    )]
    pub(crate) connect_latency_ms: Option<f64>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_opt_finite_nonnegative_timing"
    )]
    pub(crate) first_token_ms: Option<f64>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_opt_finite_nonnegative_timing"
    )]
    pub(crate) first_byte_latency_ms: Option<f64>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_opt_finite_positive_timing"
    )]
    pub(crate) stream_latency_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) upstream_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) request_summary: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_summary: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationWorkflowResponseBody {
    pub(crate) available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) body_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationWorkflowTimelineEntry {
    pub(crate) block_id: String,
    pub(crate) kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) occurred_at: Option<String>,
    pub(crate) title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) attempt: Option<InvocationWorkflowAttempt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_body: Option<InvocationWorkflowResponseBody>,
}

fn normalize_optional_timestamp(value: Option<&str>) -> Option<String> {
    let normalized = value?.trim();
    if normalized.is_empty() {
        return None;
    }
    parse_to_utc_datetime(normalized)
        .map(format_utc_iso)
        .or_else(|| Some(normalized.to_string()))
}

fn parse_summary_json_or_fallback(
    raw: Option<&str>,
    fallback: impl FnOnce() -> Value,
) -> Option<Value> {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        Some(raw) => serde_json::from_str::<Value>(raw)
            .ok()
            .or_else(|| Some(fallback())),
        None => Some(fallback()),
    }
}

fn sanitize_response_summary_timing(
    mut summary: Value,
    attempt: &InvocationWorkflowAttemptRow,
    is_final_attempt: bool,
) -> Value {
    let Some(latency) = summary.get_mut("latencyMs").and_then(Value::as_object_mut) else {
        return summary;
    };

    for key in [
        "connect",
        "firstByte",
        "requestRead",
        "requestParse",
        "responseParse",
        "persist",
        "total",
    ] {
        if latency.get(key).is_some_and(|value| {
            value
                .as_f64()
                .is_none_or(|value| !value.is_finite() || value < 0.0)
        }) {
            latency.insert(key.to_string(), Value::Null);
        }
    }

    if is_final_attempt || latency.contains_key("stream") {
        let stream = if is_final_attempt {
            final_attempt_has_stream_evidence(attempt)
                .then_some(finite_positive_timing(attempt.stream_latency_ms))
                .flatten()
        } else {
            latency
                .get("stream")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite() && *value > 0.0)
        };
        latency.insert(
            "stream".to_string(),
            stream.map_or(Value::Null, |value| json!(value)),
        );
    }

    summary
}

fn merge_attempt_request_summary_json(raw: Option<&str>, fallback: Value) -> Option<Value> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Some(fallback);
    };
    let Ok(stored) = serde_json::from_str::<Value>(raw) else {
        return Some(fallback);
    };
    let (Some(mut fallback), Some(stored)) = (fallback.as_object().cloned(), stored.as_object())
    else {
        return Some(fallback);
    };
    for (key, value) in stored {
        fallback.insert(key.clone(), value.clone());
    }
    Some(Value::Object(fallback))
}

fn parse_optional_json_value(raw: Option<&str>) -> Option<Value> {
    raw.and_then(|value| {
        let normalized = value.trim();
        if normalized.is_empty() {
            None
        } else {
            serde_json::from_str::<Value>(normalized).ok()
        }
    })
}

fn payload_value<'a>(payload: Option<&'a Value>, keys: &[&str]) -> Option<&'a Value> {
    let object = payload?.as_object()?;
    keys.iter().find_map(|key| object.get(*key))
}

fn payload_string(payload: Option<&Value>, keys: &[&str]) -> Option<String> {
    payload_value(payload, keys)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn payload_bool(payload: Option<&Value>, keys: &[&str]) -> Option<bool> {
    payload_value(payload, keys).and_then(Value::as_bool)
}

fn payload_i64(payload: Option<&Value>, keys: &[&str]) -> Option<i64> {
    payload_value(payload, keys).and_then(Value::as_i64)
}

fn payload_u64(payload: Option<&Value>, keys: &[&str]) -> Option<u64> {
    payload_value(payload, keys).and_then(Value::as_u64)
}

fn payload_f64(payload: Option<&Value>, keys: &[&str]) -> Option<f64> {
    payload_value(payload, keys).and_then(Value::as_f64)
}

fn payload_string_array(payload: Option<&Value>, keys: &[&str]) -> Option<Vec<String>> {
    payload_value(payload, keys)
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|entries| !entries.is_empty())
}

fn payload_clone(payload: Option<&Value>, keys: &[&str]) -> Option<Value> {
    payload_value(payload, keys).cloned()
}

fn request_compression_value(
    algorithm: Option<String>,
    mode: Option<String>,
    derived: RequestCompressionDerivedFields,
) -> Value {
    json!({
        "algorithm": algorithm,
        "mode": mode,
        "logicalBodyBytes": derived.logical_body_bytes,
        "transmittedBodyBytes": derived.transmitted_body_bytes,
        "savedBytes": derived.saved_bytes,
        "ratioPct": derived.ratio_pct,
        "approxUploadBytes": derived.approx_upload_bytes,
        "approxDownloadBytes": derived.approx_download_bytes,
    })
}

fn derive_request_compression_from_payload(
    payload: Option<&Value>,
) -> RequestCompressionDerivedFields {
    let mut derived = derive_request_compression_fields(
        payload_i64(payload, &["requestCompressionLogicalBodyBytes"]),
        payload_i64(payload, &["requestCompressionTransmittedBodyBytes"]),
        None,
        None,
        None,
        payload_bool(payload, &["requestCompressionTransmissionComplete"]).unwrap_or(false),
    );
    derived.approx_upload_bytes =
        payload_i64(payload, &["upstreamApproxUploadBytes"]).filter(|value| *value >= 0);
    derived.approx_download_bytes =
        payload_i64(payload, &["upstreamApproxDownloadBytes"]).filter(|value| *value >= 0);
    derived
}

fn build_request_header_snapshot(payload: Option<&Value>) -> Value {
    json!({
        "userAgent": payload_string(payload, &["requestUserAgent"]),
        "xForwardedFor": payload_string(payload, &["requestXForwardedFor"]),
        "forwarded": payload_string(payload, &["requestForwarded"]),
        "xRealIp": payload_string(payload, &["requestXRealIp"]),
    })
}

fn build_request_routing_snapshot(
    record: &ApiInvocation,
    attempt: Option<&InvocationWorkflowAttemptRow>,
    payload: Option<&Value>,
) -> Value {
    json!({
        "routeMode": record.route_mode.clone(),
        "upstreamScope": payload_string(payload, &["upstreamScope"]),
        "stickyKey": attempt
            .and_then(|row| row.sticky_key.clone())
            .or_else(|| record.sticky_key.clone())
            .or_else(|| payload_string(payload, &["stickyKey"])),
        "promptCacheKey": record
            .prompt_cache_key
            .clone()
            .or_else(|| payload_string(payload, &["promptCacheKey"])),
        "proxyDisplayName": payload_string(payload, &["proxyDisplayName"]),
        "upstreamRouteKey": attempt
            .and_then(|row| row.upstream_route_key.clone())
            .or_else(|| payload_string(payload, &["upstreamRouteKey"])),
        "proxyBindingKey": attempt
            .and_then(|row| row.proxy_binding_key_snapshot.clone())
            .or_else(|| payload_string(payload, &["proxyBindingKey", "proxyBindingKeySnapshot"])),
        "clientFingerprint": payload_string(payload, &["clientFingerprint"]),
        "clientHeaderFingerprints": payload_clone(payload, &["clientHeaderFingerprints"]),
        "oauthForwardedHeaderNames": payload_string_array(payload, &["oauthForwardedHeaderNames"]),
        "oauthPromptCacheHeaderForwarded": payload_bool(payload, &["oauthPromptCacheHeaderForwarded"]),
    })
}

fn build_request_client_snapshot(payload: Option<&Value>) -> Value {
    json!({
        "requestContainsEncryptedContent": payload_bool(payload, &["requestContainsEncryptedContent"]),
        "requestParseError": payload_string(payload, &["requestParseError"]),
        "oauthAccountHeaderAttached": payload_bool(payload, &["oauthAccountHeaderAttached"]),
        "oauthAccountIdShape": payload_string(payload, &["oauthAccountIdShape"]),
        "oauthRequestBodyPrefixFingerprint": payload_string(payload, &["oauthRequestBodyPrefixFingerprint"]),
        "oauthRequestBodyPrefixBytes": payload_u64(payload, &["oauthRequestBodyPrefixBytes"]),
        "oauthRequestBodySnapshotKind": payload_string(payload, &["oauthRequestBodySnapshotKind"]),
        "oauthResponsesBodyMode": payload_string(payload, &["oauthResponsesBodyMode"]),
        "oauthResponsesRewrite": payload_clone(payload, &["oauthResponsesRewrite"]),
    })
}

fn build_response_header_snapshot(
    record: &ApiInvocation,
    attempt: Option<&InvocationWorkflowAttemptRow>,
    payload: Option<&Value>,
    allow_invocation_level_fields: bool,
) -> Value {
    let content_encoding = attempt
        .and_then(|attempt| attempt.response_content_encoding.clone())
        .or_else(|| {
            allow_invocation_level_fields
                .then(|| {
                    record
                        .response_content_encoding
                        .clone()
                        .or_else(|| payload_string(payload, &["responseContentEncoding"]))
                })
                .flatten()
        });
    let content_encoding_chain = allow_invocation_level_fields
        .then(|| payload_string(payload, &["contentEncodingChain"]))
        .flatten();
    let upstream_request_id = attempt
        .and_then(|attempt| attempt.upstream_request_id.clone())
        .or_else(|| {
            allow_invocation_level_fields
                .then(|| {
                    record
                        .upstream_request_id
                        .clone()
                        .or_else(|| payload_string(payload, &["upstreamRequestId"]))
                })
                .flatten()
        });

    json!({
        "contentEncoding": content_encoding,
        "contentEncodingChain": content_encoding_chain,
        "upstreamRequestId": upstream_request_id,
        "cvmInvokeId": Some(record.invoke_id.clone()),
    })
}

fn build_response_delivery_snapshot(payload: Option<&Value>) -> Value {
    json!({
        "forwardedChunkCount": payload_u64(payload, &["forwardedChunkCount"]),
        "forwardedBytes": payload_u64(payload, &["forwardedBytes"]),
        "usageObserved": payload_bool(payload, &["usageObserved"]),
        "downstreamClosePhase": payload_string(payload, &["downstreamClosePhase"]),
        "downstreamWriteErrorKind": payload_string(payload, &["downstreamWriteErrorKind"]),
        "lastUpstreamChunkGapMs": payload_u64(payload, &["lastUpstreamChunkGapMs"]),
        "streamFailureOrigin": payload_string(payload, &["streamFailureOrigin"]),
        "upstreamReadErrorKind": payload_string(payload, &["upstreamReadErrorKind"]),
        "responseContainsEncryptedContent": payload_bool(payload, &["responseContainsEncryptedContent"]),
    })
}

fn invocation_status_is_success_like(record: &ApiInvocation) -> bool {
    matches!(
        normalized_runtime_text(record.status.as_deref()).as_str(),
        "success" | "completed" | "warning_success"
    )
}
