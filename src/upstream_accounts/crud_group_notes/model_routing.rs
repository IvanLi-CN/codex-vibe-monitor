pub(crate) async fn list_upstream_accounts(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListUpstreamAccountsQuery>,
) -> Result<Json<UpstreamAccountListResponse>, (StatusCode, String)> {
    list_upstream_accounts_from_params(state, params).await
}

pub(crate) async fn list_upstream_accounts_from_uri(
    State(state): State<Arc<AppState>>,
    OriginalUri(original_uri): OriginalUri,
) -> Result<Json<UpstreamAccountListResponse>, (StatusCode, String)> {
    let params = parse_list_upstream_accounts_query(&original_uri)
        .map_err(|err| (StatusCode::BAD_REQUEST, err))?;
    list_upstream_accounts_from_params(state, params).await
}

pub(crate) async fn list_upstream_account_action_events(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListUpstreamAccountActionEventsQuery>,
) -> Result<Json<UpstreamAccountActionEventListResponse>, (StatusCode, String)> {
    list_upstream_account_action_events_from_params(state, params).await
}

pub(crate) async fn get_model_routing_live(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ModelRoutingLiveQuery>,
) -> Result<Json<ModelRoutingLiveResponse>, (StatusCode, String)> {
    let (window_minutes, model, route_state, limit) =
        normalize_model_routing_live_query(&params)
            .map_err(|message| (StatusCode::BAD_REQUEST, message))?;
    let accounts = load_api_key_model_routing_live_accounts(
        &state.pool,
        model.as_deref(),
        route_state.as_deref(),
    )
    .await
    .map_err(internal_error_tuple)?;
    let visible_route_keys = route_state.as_ref().map(|_| {
        accounts
            .iter()
            .map(|account| (account.account_id, account.route.model.clone()))
            .collect::<std::collections::BTreeSet<_>>()
    });
    let records = load_model_routing_timeline_entries(
        &state.pool,
        None,
        model.as_deref(),
        window_minutes,
        if visible_route_keys.is_some() {
            MODEL_ROUTING_LIVE_DEFAULT_LIMIT
        } else {
            limit
        },
        None,
        visible_route_keys.as_ref(),
    )
    .await
    .map_err(internal_error_tuple)?
    .into_iter()
    .filter(|entry| match &visible_route_keys {
        Some(route_keys) => {
            route_keys.contains(&(entry.record.account_id, entry.record.model.clone()))
        }
        None => true,
    })
    .take(limit)
    .map(|entry| entry.record)
    .collect();
    let mut groups = std::collections::BTreeMap::<String, Vec<ModelRoutingLiveAccount>>::new();
    for account in accounts {
        groups
            .entry(account.route.model.clone())
            .or_default()
            .push(account);
    }
    Ok(Json(ModelRoutingLiveResponse {
        generated_at: Utc::now().to_rfc3339(),
        groups: groups
            .into_iter()
            .map(|(model, accounts)| ModelRoutingLiveModelGroup { model, accounts })
            .collect(),
        records,
    }))
}

pub(crate) async fn list_upstream_account_model_routing_events(
    State(state): State<Arc<AppState>>,
    AxumPath(account_id): AxumPath<i64>,
    Query(params): Query<ModelRoutingHistoryQuery>,
) -> Result<Json<ModelRoutingHistoryResponse>, (StatusCode, String)> {
    let model = normalize_required_model_routing_model(&params.model)
        .map_err(|message| (StatusCode::BAD_REQUEST, message))?;
    let page_size = normalize_model_routing_page_size(params.page_size);
    let kind =
        sqlx::query_scalar::<_, String>("SELECT kind FROM pool_upstream_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal_error_tuple)?
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    "upstream account was not found".to_string(),
                )
            })?;
    if kind != UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX {
        return Err((
            StatusCode::NOT_FOUND,
            "model routing history is only available for API Key accounts".to_string(),
        ));
    }
    let cursor = params
        .cursor
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(decode_model_routing_history_cursor)
        .transpose()
        .map_err(|message| (StatusCode::BAD_REQUEST, message))?;
    let mut entries = load_model_routing_timeline_entries(
        &state.pool,
        Some(account_id),
        Some(model.as_str()),
        MODEL_ROUTING_HISTORY_HOURS * 60,
        page_size.saturating_add(1),
        cursor.as_ref(),
        None,
    )
    .await
    .map_err(internal_error_tuple)?;
    let has_more = entries.len() > page_size;
    entries.truncate(page_size);
    let next_cursor = if has_more {
        entries.last().map(|entry| {
            encode_model_routing_history_cursor(&ModelRoutingHistoryCursor {
                occurred_epoch_ms: entry.occurred_epoch_ms,
                kind_rank: entry.kind_rank,
                id: entry.id,
            })
        })
    } else {
        None
    };
    Ok(Json(ModelRoutingHistoryResponse {
        items: entries.into_iter().map(|entry| entry.record).collect(),
        next_cursor,
    }))
}

fn normalize_model_routing_live_query(
    params: &ModelRoutingLiveQuery,
) -> Result<(i64, Option<String>, Option<String>, usize), String> {
    let window_minutes = match params
        .window
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("1h")
    {
        "15m" => 15,
        "1h" => 60,
        "6h" => 360,
        "24h" => 1_440,
        _ => return Err("window must be one of: 15m, 1h, 6h, 24h".to_string()),
    };
    let model = params
        .model
        .as_deref()
        .map(normalize_required_model_routing_model)
        .transpose()?;
    let route_state = params
        .state
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if let Some(route_state) = route_state.as_deref()
        && !matches!(
            route_state,
            MODEL_ROUTE_STATE_AVAILABLE
                | MODEL_ROUTE_STATE_DEGRADED
                | MODEL_ROUTE_STATE_COOLING_DOWN
        )
    {
        return Err("state must be one of: available, degraded, cooling_down".to_string());
    }
    let limit = params
        .limit
        .unwrap_or(MODEL_ROUTING_LIVE_DEFAULT_LIMIT)
        .clamp(1, MODEL_ROUTING_LIVE_DEFAULT_LIMIT);
    Ok((window_minutes, model, route_state, limit))
}

fn normalize_required_model_routing_model(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("model is required".to_string());
    }
    if value.chars().count() > 256 {
        return Err("model must be at most 256 characters".to_string());
    }
    Ok(value.to_string())
}

fn normalize_model_routing_page_size(value: Option<usize>) -> usize {
    value
        .unwrap_or(MODEL_ROUTING_HISTORY_DEFAULT_PAGE_SIZE)
        .clamp(1, MODEL_ROUTING_MAX_PAGE_SIZE)
}

fn normalized_model_routing_timestamp(value: String) -> String {
    crate::stats::parse_to_utc_datetime(&value)
        .map(crate::stats::format_utc_iso)
        .unwrap_or(value)
}

fn encode_model_routing_history_cursor(cursor: &ModelRoutingHistoryCursor) -> String {
    let raw = serde_json::to_vec(cursor).expect("model routing history cursor serializes");
    URL_SAFE_NO_PAD.encode(raw)
}

fn decode_model_routing_history_cursor(value: &str) -> Result<ModelRoutingHistoryCursor, String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value.trim())
        .map_err(|_| "cursor is invalid".to_string())?;
    let cursor = serde_json::from_slice::<ModelRoutingHistoryCursor>(&bytes)
        .map_err(|_| "cursor is invalid".to_string())?;
    if cursor.occurred_epoch_ms <= 0 || cursor.id <= 0 || !(0..=1).contains(&cursor.kind_rank) {
        return Err("cursor is invalid".to_string());
    }
    Ok(cursor)
}

fn model_routing_timeline_entry_from_attempt(
    row: ModelRoutingAttemptRow,
) -> ModelRoutingTimelineEntry {
    let audit = row
        .routing_selection_audit_json
        .as_deref()
        .and_then(|value| serde_json::from_str::<PoolRoutingSelectionAudit>(value).ok());
    let total_latency_ms = model_routing_total_latency_ms(
        row.connect_latency_ms,
        row.first_byte_latency_ms,
        row.stream_latency_ms,
    );
    let record = ModelRoutingTimelineRecord {
        id: format!("attempt:{}", row.id),
        kind: "attempt".to_string(),
        occurred_at: normalized_model_routing_timestamp(row.occurred_at),
        account_id: row.account_id,
        account_display_name: None,
        model: row.model,
        attempt_id: Some(row.attempt_id),
        invoke_id: Some(row.invoke_id),
        attempt_index: Some(row.attempt_index),
        same_account_retry_index: Some(row.same_account_retry_index),
        routing_source: row.routing_source,
        routing_selection_audit: audit,
        status: Some(row.status),
        http_status: row.http_status,
        failure_kind: row.failure_kind,
        total_latency_ms,
        action: row.event_action,
        source: row.event_source,
        reason_code: row.event_reason_code,
        model_route_state_before: row.event_state_before,
        model_route_state_after: row.event_state_after,
        model_route_priority_before: row.event_priority_before,
        model_route_priority_after: row.event_priority_after,
        model_route_failure_count: row.event_failure_count,
        model_route_cooldown_until: row.event_cooldown_until,
    };
    ModelRoutingTimelineEntry {
        occurred_epoch_ms: row.occurred_epoch_ms,
        kind_rank: 1,
        id: row.id,
        record,
    }
}

fn model_routing_total_latency_ms(
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
) -> Option<f64> {
    let normalize_component = |value: Option<f64>, strictly_positive: bool| {
        value.map_or(Some(0.0), |value| {
            (value.is_finite()
                && if strictly_positive {
                    value > 0.0
                } else {
                    value >= 0.0
                })
            .then_some(value)
        })
    };
    let connect_latency_ms = normalize_component(connect_latency_ms, false)?;
    let first_byte_latency_ms = normalize_component(first_byte_latency_ms, false)?;
    let stream_latency_ms = normalize_component(stream_latency_ms, true)?;
    let total_latency_ms = connect_latency_ms + first_byte_latency_ms + stream_latency_ms;
    total_latency_ms
        .is_finite()
        .then_some(total_latency_ms)
        .filter(|value| *value > 0.0)
}

fn model_routing_display_name(
    account_id: i64,
    display_names: &std::collections::BTreeMap<i64, String>,
) -> Option<String> {
    display_names
        .get(&account_id)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn model_routing_public_account_name(
    account_id: i64,
    display_names: &std::collections::BTreeMap<i64, String>,
) -> String {
    model_routing_display_name(account_id, display_names)
        .unwrap_or_else(|| format!("API Key #{account_id}"))
}

fn apply_model_routing_display_names(
    entries: &mut [ModelRoutingTimelineEntry],
    display_names: &std::collections::BTreeMap<i64, String>,
) {
    for entry in entries {
        let record = &mut entry.record;
        record.account_display_name = model_routing_display_name(record.account_id, display_names);
        let Some(audit) = record.routing_selection_audit.as_mut() else {
            continue;
        };
        audit.selected_account_name =
            model_routing_public_account_name(audit.selected_account_id, display_names);
        audit.compared_account_name = audit
            .compared_account_id
            .map(|account_id| model_routing_public_account_name(account_id, display_names));
        for candidate in &mut audit.excluded_candidates {
            candidate.account_name =
                model_routing_public_account_name(candidate.account_id, display_names);
        }
    }
}

async fn load_model_routing_display_names(
    pool: &Pool<Sqlite>,
    entries: &[ModelRoutingTimelineEntry],
) -> Result<std::collections::BTreeMap<i64, String>> {
    let mut account_ids = std::collections::BTreeSet::new();
    for entry in entries {
        account_ids.insert(entry.record.account_id);
        if let Some(audit) = entry.record.routing_selection_audit.as_ref() {
            account_ids.insert(audit.selected_account_id);
            if let Some(account_id) = audit.compared_account_id {
                account_ids.insert(account_id);
            }
            account_ids.extend(
                audit
                    .excluded_candidates
                    .iter()
                    .map(|candidate| candidate.account_id),
            );
        }
    }
    if account_ids.is_empty() {
        return Ok(std::collections::BTreeMap::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, display_name FROM pool_upstream_accounts WHERE kind = ",
    );
    query.push_bind(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX);
    query.push(" AND COALESCE(deleted_at, '') = '' AND id IN (");
    let mut separated = query.separated(", ");
    for account_id in account_ids {
        separated.push_bind(account_id);
    }
    separated.push_unseparated(")");
    let rows = query
        .build_query_as::<(i64, String)>()
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().collect())
}

fn model_routing_timeline_entry_from_event(row: ModelRoutingEventRow) -> ModelRoutingTimelineEntry {
    let record = ModelRoutingTimelineRecord {
        id: format!("event:{}", row.id),
        kind: "event".to_string(),
        occurred_at: normalized_model_routing_timestamp(row.occurred_at),
        account_id: row.account_id,
        account_display_name: None,
        model: row.model,
        attempt_id: None,
        invoke_id: None,
        attempt_index: None,
        same_account_retry_index: None,
        routing_source: None,
        routing_selection_audit: None,
        status: row.result,
        http_status: row.http_status,
        failure_kind: row.failure_kind,
        total_latency_ms: None,
        action: Some(row.action),
        source: Some(row.source),
        reason_code: row.reason_code,
        model_route_state_before: row.model_route_state_before,
        model_route_state_after: row.model_route_state_after,
        model_route_priority_before: row.model_route_priority_before,
        model_route_priority_after: row.model_route_priority_after,
        model_route_failure_count: row.model_route_failure_count,
        model_route_cooldown_until: row.model_route_cooldown_until,
    };
    ModelRoutingTimelineEntry {
        occurred_epoch_ms: row.occurred_epoch_ms,
        kind_rank: 0,
        id: row.id,
        record,
    }
}

async fn load_model_routing_timeline_entries(
    pool: &Pool<Sqlite>,
    account_id: Option<i64>,
    model: Option<&str>,
    window_minutes: i64,
    limit: usize,
    cursor: Option<&ModelRoutingHistoryCursor>,
    route_keys: Option<&std::collections::BTreeSet<(i64, String)>>,
) -> Result<Vec<ModelRoutingTimelineEntry>> {
    let cutoff_epoch_ms =
        (Utc::now() - ChronoDuration::minutes(window_minutes.max(1))).timestamp_millis();
    let attempts = build_model_routing_attempt_timeline_query(
        "",
        account_id,
        model,
        cutoff_epoch_ms,
        limit,
        cursor,
        route_keys,
    )
    .build_query_as::<ModelRoutingAttemptRow>()
    .fetch_all(pool)
    .await?;
    let events = build_model_routing_event_timeline_query(
        "",
        account_id,
        model,
        cutoff_epoch_ms,
        limit,
        cursor,
        route_keys,
    )
    .build_query_as::<ModelRoutingEventRow>()
    .fetch_all(pool)
    .await?;

    let mut entries = attempts
        .into_iter()
        .map(model_routing_timeline_entry_from_attempt)
        .chain(
            events
                .into_iter()
                .map(model_routing_timeline_entry_from_event),
        )
        .collect::<Vec<_>>();
    let display_names = load_model_routing_display_names(pool, &entries).await?;
    apply_model_routing_display_names(&mut entries, &display_names);
    entries.sort_by(|left, right| {
        right
            .occurred_epoch_ms
            .cmp(&left.occurred_epoch_ms)
            .then_with(|| right.kind_rank.cmp(&left.kind_rank))
            .then_with(|| right.id.cmp(&left.id))
    });
    entries.truncate(limit);
    Ok(entries)
}

pub(crate) fn build_model_routing_attempt_timeline_query(
    prefix: &str,
    account_id: Option<i64>,
    model: Option<&str>,
    cutoff_epoch_ms: i64,
    limit: usize,
    cursor: Option<&ModelRoutingHistoryCursor>,
    route_keys: Option<&std::collections::BTreeSet<(i64, String)>>,
) -> QueryBuilder<'static, Sqlite> {
    // The attempt row captures the model selected for this individual retry. Fall back to the
    // invocation payload only for older rows that predate `request_model` persistence.
    let model_sql = format!(
        "COALESCE(NULLIF(TRIM(attempts.request_model), ''), {ACCOUNT_ATTEMPT_REQUEST_MODEL_SQL})"
    );
    let mut attempt_query = QueryBuilder::<Sqlite>::new(format!(
        r#"
        {prefix}
        SELECT attempts.id,
               COALESCE(NULLIF(TRIM(attempts.attempt_public_id), ''), printf('legacy-attempt-%lld', attempts.id)) AS attempt_id,
               attempts.invoke_id,
               attempts.occurred_at,
               attempts.occurred_epoch_ms,
               attempts.routing_source,
               attempts.routing_selection_audit_json,
               attempts.upstream_account_id AS account_id,
               {model_sql} AS model,
               attempts.attempt_index,
               attempts.same_account_retry_index,
               attempts.status,
               attempts.http_status,
               attempts.failure_kind,
               attempts.connect_latency_ms,
               attempts.first_byte_latency_ms,
               attempts.stream_latency_ms,
               event.action AS event_action,
               event.source AS event_source,
               event.reason_code AS event_reason_code,
               event.model_route_state_before AS event_state_before,
               event.model_route_state_after AS event_state_after,
               event.model_route_priority_before AS event_priority_before,
               event.model_route_priority_after AS event_priority_after,
               event.model_route_failure_count AS event_failure_count,
               event.model_route_cooldown_until AS event_cooldown_until
          FROM pool_upstream_request_attempts AS attempts
          CROSS JOIN pool_upstream_accounts AS accounts
          LEFT JOIN codex_invocations AS inv ON inv.invoke_id = attempts.invoke_id
              AND inv.occurred_at = attempts.occurred_at
          LEFT JOIN pool_upstream_account_events AS event ON event.id = (
              SELECT latest.id
               FROM pool_upstream_account_events AS latest
               WHERE latest.attempt_id = attempts.id
                 AND (
                     latest.model_route_state_before IS NOT NULL
                     OR latest.model_route_state_after IS NOT NULL
                     OR latest.model_route_priority_before IS NOT NULL
                     OR latest.model_route_priority_after IS NOT NULL
                 )
               ORDER BY latest.occurred_epoch_ms DESC, latest.id DESC
               LIMIT 1
          )
         WHERE accounts.id = attempts.upstream_account_id
           AND accounts.kind = "#,
    ));
    attempt_query.push_bind(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX);
    append_model_routing_attempt_timeline_filters(
        &mut attempt_query,
        account_id,
        model,
        cutoff_epoch_ms,
        cursor,
        route_keys,
        &model_sql,
    );
    attempt_query
        .push(" ORDER BY attempts.occurred_epoch_ms DESC, attempts.id DESC LIMIT ")
        .push_bind(limit as i64);
    attempt_query
}

pub(crate) fn build_model_routing_event_timeline_query(
    prefix: &str,
    account_id: Option<i64>,
    model: Option<&str>,
    cutoff_epoch_ms: i64,
    limit: usize,
    cursor: Option<&ModelRoutingHistoryCursor>,
    route_keys: Option<&std::collections::BTreeSet<(i64, String)>>,
) -> QueryBuilder<'static, Sqlite> {
    let mut event_query = QueryBuilder::<Sqlite>::new(format!(
        r#"
        {prefix}
        SELECT event.id,
               event.occurred_at,
               event.occurred_epoch_ms,
               event.account_id,
               event.model,
               event.action,
               event.source,
               event.result,
               event.http_status,
               event.failure_kind,
               event.reason_code,
               event.model_route_state_before,
               event.model_route_state_after,
               event.model_route_priority_before,
               event.model_route_priority_after,
               event.model_route_failure_count,
               event.model_route_cooldown_until
          FROM pool_upstream_account_events AS event
          CROSS JOIN pool_upstream_accounts AS accounts
         WHERE accounts.id = event.account_id
           AND accounts.kind = "#,
    ));
    event_query.push_bind(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX);
    event_query.push(" AND COALESCE(accounts.deleted_at, '') = ''");
    event_query.push(" AND event.attempt_id IS NULL AND event.model IS NOT NULL");
    event_query
        .push(" AND ")
        .push("event.occurred_epoch_ms")
        .push(" >= ");
    event_query.push_bind(cutoff_epoch_ms);
    if let Some(account_id) = account_id {
        event_query.push(" AND event.account_id = ");
        event_query.push_bind(account_id);
    }
    if let Some(model) = model {
        event_query.push(" AND event.model = ");
        event_query.push_bind(model.to_string());
    }
    append_model_routing_route_key_filter(
        &mut event_query,
        route_keys,
        "event.account_id",
        "event.model",
    );
    if let Some(cursor) = cursor {
        event_query
            .push(" AND (")
            .push("event.occurred_epoch_ms")
            .push(" < ");
        event_query.push_bind(cursor.occurred_epoch_ms);
        event_query
            .push(" OR (")
            .push("event.occurred_epoch_ms")
            .push(" = ");
        event_query.push_bind(cursor.occurred_epoch_ms);
        event_query.push(" AND (0 < ");
        event_query.push_bind(cursor.kind_rank);
        event_query.push(" OR (0 = ");
        event_query.push_bind(cursor.kind_rank);
        event_query.push(" AND event.id < ");
        event_query.push_bind(cursor.id);
        event_query.push("))))");
    }
    event_query
        .push(" ORDER BY event.occurred_epoch_ms DESC, event.id DESC LIMIT ")
        .push_bind(limit as i64);
    event_query
}

fn append_model_routing_route_key_filter(
    query: &mut QueryBuilder<'_, Sqlite>,
    route_keys: Option<&std::collections::BTreeSet<(i64, String)>>,
    account_sql: &str,
    model_sql: &str,
) {
    let Some(route_keys) = route_keys else {
        return;
    };
    if route_keys.is_empty() {
        query.push(" AND 0 = 1");
        return;
    }
    query.push(" AND (");
    for (index, (account_id, model)) in route_keys.iter().enumerate() {
        if index > 0 {
            query.push(" OR ");
        }
        query
            .push(account_sql)
            .push(" = ")
            .push_bind(*account_id)
            .push(" AND ")
            .push(model_sql)
            .push(" = ")
            .push_bind(model.clone());
    }
    query.push(")");
}
