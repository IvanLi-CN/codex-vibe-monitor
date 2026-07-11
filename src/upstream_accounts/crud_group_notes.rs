use super::*;
use std::time::Instant;

#[cfg(test)]
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

pub(crate) async fn list_upstream_account_attempts(
    State(state): State<Arc<AppState>>,
    AxumPath(account_id): AxumPath<i64>,
    Query(params): Query<ListUpstreamAccountAttemptsQuery>,
) -> Result<Json<UpstreamAccountAttemptListResponse>, (StatusCode, String)> {
    let page = normalize_upstream_account_list_page(params.page);
    let page_size = normalize_upstream_account_list_page_size(params.page_size);
    let response = load_upstream_account_attempt_page(&state.pool, account_id, page, page_size)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(response))
}

pub(crate) async fn locate_upstream_account_attempt(
    State(state): State<Arc<AppState>>,
    AxumPath(account_id): AxumPath<i64>,
    Query(params): Query<LocateUpstreamAccountAttemptQuery>,
) -> Result<Json<UpstreamAccountAttemptListResponse>, (StatusCode, String)> {
    let page_size = normalize_upstream_account_list_page_size(params.page_size);
    let cutoff = shanghai_local_cutoff_string(7);
    let target_occurred_at = sqlx::query_scalar::<_, String>(
        r#"
        SELECT occurred_at
        FROM pool_upstream_request_attempts
        WHERE id = ?1
          AND upstream_account_id = ?2
          AND occurred_at >= ?3
        "#,
    )
    .bind(params.attempt_id)
    .bind(account_id)
    .bind(&cutoff)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_error_tuple)?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            "upstream account attempt was not found".to_string(),
        )
    })?;
    let newer_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_request_attempts
        WHERE upstream_account_id = ?1
          AND occurred_at >= ?2
          AND (occurred_at > ?3 OR (occurred_at = ?3 AND id > ?4))
        "#,
    )
    .bind(account_id)
    .bind(&cutoff)
    .bind(&target_occurred_at)
    .bind(params.attempt_id)
    .fetch_one(&state.pool)
    .await
    .map_err(internal_error_tuple)?
    .max(0) as usize;
    let page = newer_count / page_size + 1;
    let response = load_upstream_account_attempt_page(&state.pool, account_id, page, page_size)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(response))
}

async fn load_upstream_account_attempt_page(
    pool: &Pool<Sqlite>,
    account_id: i64,
    page: usize,
    page_size: usize,
) -> Result<UpstreamAccountAttemptListResponse> {
    const ATTEMPT_RETENTION_DAYS: u64 = 7;
    let cutoff = shanghai_local_cutoff_string(ATTEMPT_RETENTION_DAYS);
    let total = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_request_attempts
        WHERE upstream_account_id = ?1 AND occurred_at >= ?2
        "#,
    )
    .bind(account_id)
    .bind(&cutoff)
    .fetch_one(pool)
    .await?
    .max(0) as usize;
    let offset = page.saturating_sub(1).saturating_mul(page_size);
    let items = sqlx::query_as::<_, ApiPoolUpstreamRequestAttempt>(
        r#"
        SELECT
            attempts.id,
            attempts.invoke_id,
            attempts.occurred_at,
            attempts.endpoint,
            attempts.sticky_key,
            attempts.upstream_account_id,
            accounts.display_name AS upstream_account_name,
            attempts.upstream_route_key,
            attempts.proxy_binding_key_snapshot,
            attempts.attempt_index,
            attempts.distinct_account_index,
            attempts.same_account_retry_index,
            attempts.requester_ip,
            COALESCE(
                inv.request_model,
                inv.model,
                inv.response_model
            ) AS model,
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
            attempts.compact_support_status,
            attempts.compact_support_reason,
            attempts.created_at
        FROM pool_upstream_request_attempts AS attempts
        LEFT JOIN pool_upstream_accounts AS accounts
            ON accounts.id = attempts.upstream_account_id
        LEFT JOIN codex_invocations AS inv
            ON inv.invoke_id = attempts.invoke_id
           AND inv.occurred_at = attempts.occurred_at
        WHERE attempts.upstream_account_id = ?1 AND attempts.occurred_at >= ?2
        ORDER BY attempts.occurred_at DESC, attempts.id DESC
        LIMIT ?3 OFFSET ?4
        "#,
    )
    .bind(account_id)
    .bind(&cutoff)
    .bind(page_size as i64)
    .bind(offset as i64)
    .fetch_all(pool)
    .await?;
    Ok(UpstreamAccountAttemptListResponse {
        items,
        total,
        page,
        page_size,
    })
}

pub(crate) async fn list_upstream_accounts_from_params(
    state: Arc<AppState>,
    params: ListUpstreamAccountsQuery,
) -> Result<Json<UpstreamAccountListResponse>, (StatusCode, String)> {
    let started_at = Instant::now();

    expire_pending_login_sessions(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let page = normalize_upstream_account_list_page(params.page);
    let page_size = normalize_upstream_account_list_page_size(params.page_size);
    let include_all = params.include_all.unwrap_or(false);
    let filters = normalize_upstream_account_list_filters(&params);
    let load_summaries_started_at = Instant::now();
    let mut all_items =
        load_upstream_account_summaries_for_query(&state.pool, &state.config, &params)
            .await
            .map_err(internal_error_tuple)?;
    let load_summaries_ms = load_summaries_started_at.elapsed().as_millis() as u64;
    let load_groups_started_at = Instant::now();
    let groups = load_canonicalized_upstream_account_groups(state.as_ref())
        .await
        .map_err(internal_error_tuple)?;
    let load_groups_ms = load_groups_started_at.elapsed().as_millis() as u64;
    let enrich_block_reason_started_at = Instant::now();
    enrich_node_shunt_routing_block_reasons(state.as_ref(), &mut all_items)
        .await
        .map_err(internal_error_tuple)?;
    enrich_current_forward_proxy_for_summaries(state.as_ref(), &groups, &mut all_items)
        .await
        .map_err(internal_error_tuple)?;
    let enrich_block_reason_ms = enrich_block_reason_started_at.elapsed().as_millis() as u64;
    let filtered_items = filter_upstream_account_summaries(all_items, &filters);
    let total = filtered_items.len();
    let metrics = build_upstream_account_list_metrics(&filtered_items);
    let forward_proxy_catalog_keys = collect_forward_proxy_catalog_keys(&groups, &filtered_items);
    let offset = page.saturating_sub(1).saturating_mul(page_size);
    let items = if include_all {
        filtered_items.clone()
    } else if offset >= total {
        Vec::new()
    } else {
        filtered_items
            .iter()
            .skip(offset)
            .take(page_size)
            .cloned()
            .collect::<Vec<_>>()
    };
    let response_page = if include_all { 1 } else { page };
    let response_page_size = if include_all {
        total.max(page_size)
    } else {
        page_size
    };
    let roster_core_ms = started_at.elapsed().as_millis() as u64;
    let usage_batch_ms = 0_u64;
    let has_ungrouped_accounts = has_ungrouped_upstream_accounts(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let load_routing_started_at = Instant::now();
    let routing = load_pool_routing_settings_seeded(&state.pool, &state.config)
        .await
        .map_err(internal_error_tuple)?;
    let load_routing_ms = load_routing_started_at.elapsed().as_millis() as u64;
    let load_forward_proxy_catalog_started_at = Instant::now();
    let forward_proxy_nodes =
        build_forward_proxy_binding_nodes_response(state.as_ref(), &forward_proxy_catalog_keys)
            .await
            .map_err(internal_error_tuple)?;
    let load_forward_proxy_catalog_ms =
        load_forward_proxy_catalog_started_at.elapsed().as_millis() as u64;
    let total_ms = started_at.elapsed().as_millis() as u64;
    tracing::info!(
        page = response_page,
        page_size = response_page_size,
        include_all,
        total,
        roster_core_ms,
        load_summaries_ms,
        enrich_block_reason_ms,
        usage_batch_ms,
        load_groups_ms,
        load_routing_ms,
        load_forward_proxy_catalog_ms,
        total_ms,
        "upstream accounts roster request completed"
    );
    Ok(Json(UpstreamAccountListResponse {
        writes_enabled: state.upstream_accounts.writes_enabled(),
        items,
        total,
        page: response_page,
        page_size: response_page_size,
        metrics,
        groups,
        forward_proxy_nodes,
        has_ungrouped_accounts,
        routing: build_pool_routing_settings_response(state.as_ref(), &routing),
    }))
}

pub(crate) async fn list_upstream_account_action_events_from_params(
    state: Arc<AppState>,
    params: ListUpstreamAccountActionEventsQuery,
) -> Result<Json<UpstreamAccountActionEventListResponse>, (StatusCode, String)> {
    let page = normalize_upstream_account_list_page(params.page);
    let page_size = normalize_upstream_account_list_page_size(params.page_size);
    let result_filter =
        normalize_upstream_account_action_event_result_filter(params.result.as_deref())
            .map_err(|err| (StatusCode::BAD_REQUEST, err))?;
    let account_filter = normalize_optional_search_filter(params.account.as_deref());
    let group_filter = normalize_optional_search_filter(params.group.as_deref());
    let proxy_key_filter = normalize_optional_exact_filter(params.proxy_key.as_deref());

    let mut conditions = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(filter) = account_filter.as_deref() {
        conditions.push(
            "(lower(COALESCE(event.account_display_name, account.display_name)) LIKE lower(?) OR lower(COALESCE(account.email, '')) LIKE lower(?) OR CAST(event.account_id AS TEXT) LIKE ?)"
                .to_string(),
        );
        let wildcard = format!("%{filter}%");
        binds.push(wildcard.clone());
        binds.push(wildcard.clone());
        binds.push(wildcard);
    }
    if let Some(filter) = group_filter.as_deref() {
        conditions.push(
            "lower(COALESCE(event.account_group_name, account.group_name, '')) LIKE lower(?)"
                .to_string(),
        );
        binds.push(format!("%{filter}%"));
    }
    if let Some(filter) = proxy_key_filter.as_deref() {
        conditions.push("lower(COALESCE(event.forward_proxy_key, '')) = lower(?)".to_string());
        binds.push(filter.to_string());
    }
    if let Some(filter) = result_filter.as_deref() {
        conditions.push("lower(COALESCE(event.result, '')) = lower(?)".to_string());
        binds.push(filter.to_string());
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let total_sql = format!(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_account_events event
        INNER JOIN pool_upstream_accounts account ON account.id = event.account_id
        {where_clause}
        "#
    );
    let mut total_query = sqlx::query_scalar::<_, i64>(&total_sql);
    for bind in &binds {
        total_query = total_query.bind(bind);
    }
    let total = total_query
        .fetch_one(&state.pool)
        .await
        .map_err(internal_error_tuple)? as usize;

    let offset = page.saturating_sub(1).saturating_mul(page_size);
    let items_sql = format!(
        r#"
        SELECT
            event.id,
            event.occurred_at,
            event.action,
            event.source,
            COALESCE(event.account_display_name, account.display_name) AS account_display_name,
            COALESCE(event.account_group_name, account.group_name) AS account_group_name,
            event.forward_proxy_key,
            event.forward_proxy_display_name,
            event.forward_proxy_egress_ip,
            event.result,
            event.result_description,
            event.reason_code,
            event.reason_message,
            event.http_status,
            event.failure_kind,
            event.invoke_id,
            event.attempt_id,
            event.sticky_key,
            event.created_at
        FROM pool_upstream_account_events event
        INNER JOIN pool_upstream_accounts account ON account.id = event.account_id
        {where_clause}
        ORDER BY event.occurred_at DESC, event.id DESC
        LIMIT ? OFFSET ?
        "#
    );
    let mut items_query = sqlx::query_as::<_, UpstreamAccountActionEventRow>(&items_sql);
    for bind in &binds {
        items_query = items_query.bind(bind);
    }
    let rows = items_query
        .bind(page_size as i64)
        .bind(offset as i64)
        .fetch_all(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let items = rows
        .iter()
        .map(build_action_event_from_row)
        .collect::<Vec<_>>();

    Ok(Json(UpstreamAccountActionEventListResponse {
        items,
        total,
        page,
        page_size,
    }))
}

pub(crate) async fn get_upstream_account_window_usage(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<UpstreamAccountWindowUsageRequest>,
) -> Result<Json<UpstreamAccountWindowUsageResponse>, (StatusCode, String)> {
    let started_at = Instant::now();
    let mut account_ids = payload
        .account_ids
        .into_iter()
        .filter(|account_id| *account_id > 0)
        .collect::<Vec<_>>();
    account_ids.sort_unstable();
    account_ids.dedup();

    if account_ids.is_empty() {
        return Ok(Json(UpstreamAccountWindowUsageResponse {
            items: Vec::new(),
        }));
    }

    let load_summaries_started_at = Instant::now();
    let mut summaries =
        load_upstream_account_window_usage_summaries(&state.pool, &state.config, &account_ids)
            .await
            .map_err(internal_error_tuple)?;
    let load_summaries_ms = load_summaries_started_at.elapsed().as_millis() as u64;
    let usage_batch_started_at = Instant::now();
    enrich_window_actual_usage_for_summaries(state.as_ref(), &mut summaries)
        .await
        .map_err(internal_error_tuple)?;
    let usage_batch_ms = usage_batch_started_at.elapsed().as_millis() as u64;
    let total_ms = started_at.elapsed().as_millis() as u64;
    tracing::info!(
        account_count = account_ids.len(),
        load_summaries_ms,
        usage_batch_ms,
        total_ms,
        "upstream account window usage batch completed"
    );

    Ok(Json(UpstreamAccountWindowUsageResponse {
        items: summaries
            .into_iter()
            .map(|summary| UpstreamAccountWindowUsageItem {
                account_id: summary.id,
                primary_actual_usage: summary
                    .primary_window
                    .and_then(|window| window.actual_usage),
                secondary_actual_usage: summary
                    .secondary_window
                    .and_then(|window| window.actual_usage),
            })
            .collect(),
    }))
}

pub(crate) async fn list_forward_proxy_binding_nodes(
    State(state): State<Arc<AppState>>,
    uri: Uri,
) -> Result<Json<Vec<ForwardProxyBindingNodeResponse>>, (StatusCode, String)> {
    let params = parse_list_forward_proxy_binding_nodes_query(&uri)
        .map_err(|message| (StatusCode::BAD_REQUEST, message))?;
    let requested_keys = params
        .key
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if requested_keys.is_empty() && !params.include_current && params.group_name.is_none() {
        return Ok(Json(Vec::new()));
    }
    let nodes = if let Some(group_name) = params.group_name.as_deref() {
        build_group_forward_proxy_binding_nodes_response(
            state.as_ref(),
            &requested_keys,
            group_name,
        )
        .await
    } else {
        build_forward_proxy_binding_nodes_response(state.as_ref(), &requested_keys).await
    }
    .map_err(internal_error_tuple)?;
    Ok(Json(nodes))
}

pub(crate) fn parse_list_forward_proxy_binding_nodes_query(
    uri: &Uri,
) -> Result<ListForwardProxyBindingNodesQuery, String> {
    let mut params = ListForwardProxyBindingNodesQuery::default();
    let Some(raw_query) = uri.query() else {
        return Ok(params);
    };
    for (key, value) in url::form_urlencoded::parse(raw_query.as_bytes()) {
        match key.as_ref() {
            "key" => params.key.push(value.into_owned()),
            "groupName" => params.group_name = normalize_optional_text(Some(value.into_owned())),
            "includeCurrent" => {
                params.include_current = match value.as_ref() {
                    "" | "0" | "false" | "False" | "FALSE" | "no" | "off" => false,
                    "1" | "true" | "True" | "TRUE" | "yes" | "on" => true,
                    other => {
                        return Err(format!(
                            "invalid includeCurrent value `{other}`; expected true/false"
                        ));
                    }
                };
            }
            _ => {}
        }
    }
    Ok(params)
}

pub(crate) fn parse_list_upstream_accounts_query(
    uri: &Uri,
) -> Result<ListUpstreamAccountsQuery, String> {
    let base = Query::<ListUpstreamAccountsBaseQuery>::try_from_uri(uri)
        .map_err(|err| err.body_text())?
        .0;
    let mut params = ListUpstreamAccountsQuery {
        group_search: base.group_search,
        group_ungrouped: base.group_ungrouped,
        status: base.status,
        page: base.page,
        page_size: base.page_size,
        ..ListUpstreamAccountsQuery::default()
    };

    for (key, value) in url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()) {
        match key.as_ref() {
            "groupExact" => params.group_exact.push(value.into_owned()),
            "workStatus" => params.work_status.push(value.into_owned()),
            "enableStatus" => params.enable_status.push(value.into_owned()),
            "healthStatus" => params.health_status.push(value.into_owned()),
            "tagIds" => {
                let tag_id = value
                    .parse::<i64>()
                    .map_err(|_| format!("invalid tagIds value `{value}`; expected integer"))?;
                params.tag_ids.push(tag_id);
            }
            "includeAll" => {
                params.include_all = match value.as_ref() {
                    "" | "0" | "false" | "False" | "FALSE" | "no" | "off" => Some(false),
                    "1" | "true" | "True" | "TRUE" | "yes" | "on" => Some(true),
                    other => {
                        return Err(format!(
                            "invalid includeAll value `{other}`; expected true/false"
                        ));
                    }
                };
            }
            _ => {}
        }
    }

    Ok(params)
}

fn normalize_optional_search_filter(value: Option<&str>) -> Option<String> {
    let normalized = value?.trim();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

fn normalize_optional_exact_filter(value: Option<&str>) -> Option<String> {
    normalize_optional_search_filter(value)
}

fn normalize_upstream_account_action_event_result_filter(
    value: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(normalized) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let lower = normalized.to_ascii_lowercase();
    match lower.as_str() {
        "success" | "failed" | "deferred" => Ok(Some(lower)),
        other => Err(format!(
            "invalid result value `{other}`; expected success, failed, or deferred"
        )),
    }
}

pub(crate) async fn list_tags(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListTagsQuery>,
) -> Result<Json<TagListResponse>, (StatusCode, String)> {
    let items = load_tag_summaries(&state.pool, &params)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(TagListResponse {
        writes_enabled: false,
        items,
    }))
}

pub(crate) async fn create_tag(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateTagRequest>,
) -> Result<Json<TagDetail>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let name = normalize_tag_name(&payload.name)?;
    let rule = normalize_tag_rule(
        payload.allow_cut_out,
        payload.allow_cut_in,
        payload.priority_tier.as_deref(),
        payload.fast_mode_rewrite_mode.as_deref(),
        payload.concurrency_limit,
        payload.upstream_429_retry_enabled,
        payload.upstream_429_max_retries,
        Some(payload.available_models),
    )?;
    let detail = insert_tag(&state.pool, &name, &rule)
        .await
        .map_err(map_tag_write_error)?;
    Ok(Json(detail))
}

pub(crate) async fn get_tag(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<TagDetail>, (StatusCode, String)> {
    let detail = load_tag_detail(&state.pool, id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "tag not found".to_string()))?;
    Ok(Json(detail))
}

pub(crate) async fn update_tag(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    Json(payload): Json<UpdateTagRequest>,
) -> Result<Json<TagDetail>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let existing = load_tag_row(&state.pool, id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "tag not found".to_string()))?;
    if existing.protected != 0 {
        return Err((
            StatusCode::CONFLICT,
            "system tag cannot be edited".to_string(),
        ));
    }
    let name = match payload.name {
        Some(value) => normalize_tag_name(&value)?,
        None => existing.name.clone(),
    };
    let rule = normalize_tag_rule(
        payload.allow_cut_out.unwrap_or(existing.allow_cut_out != 0),
        payload.allow_cut_in.unwrap_or(existing.allow_cut_in != 0),
        payload
            .priority_tier
            .as_deref()
            .or(Some(existing.priority_tier.as_str())),
        payload
            .fast_mode_rewrite_mode
            .as_deref()
            .or(Some(existing.fast_mode_rewrite_mode.as_str())),
        payload
            .concurrency_limit
            .or(Some(existing.concurrency_limit)),
        payload
            .upstream_429_retry_enabled
            .or(Some(existing.upstream_429_retry_enabled != 0)),
        payload
            .upstream_429_max_retries
            .or(Some(decode_group_upstream_429_max_retries(
                existing.upstream_429_max_retries,
            ))),
        Some(match payload.available_models {
            OptionalField::Missing => {
                parse_string_array_json(existing.available_models_json.as_deref())
            }
            OptionalField::Null => Vec::new(),
            OptionalField::Value(value) => value,
        }),
    )?;
    let detail = persist_tag_update(&state.pool, id, &name, &rule)
        .await
        .map_err(map_tag_write_error)?;
    Ok(Json(detail))
}

pub(crate) async fn delete_tag(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    delete_tag_by_id(&state.pool, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn update_upstream_account_group(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(group_name): AxumPath<String>,
    Json(payload): Json<UpdateUpstreamAccountGroupRequest>,
) -> Result<Json<UpstreamAccountGroupSummary>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;

    let group_name = normalize_optional_text(Some(group_name)).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "group name is required".to_string(),
        )
    })?;
    let note = normalize_optional_text(payload.note);
    let bound_proxy_keys_was_updated = payload.bound_proxy_keys.is_some();
    let mut bound_proxy_keys = payload
        .bound_proxy_keys
        .map(normalize_bound_proxy_keys)
        .unwrap_or_else(Vec::new);
    let node_shunt_enabled_was_updated = payload.node_shunt_enabled.is_some();
    let single_account_rotation_enabled_was_updated =
        payload.single_account_rotation_enabled.is_some();
    let upstream_429_retry_enabled_was_updated = payload.upstream_429_retry_enabled.is_some();
    let upstream_429_max_retries_was_updated = payload.upstream_429_max_retries.is_some();
    let normalized_upstream_429_max_retries = payload
        .upstream_429_max_retries
        .map(normalize_group_upstream_429_max_retries)
        .unwrap_or_default();
    let concurrency_limit =
        normalize_concurrency_limit(payload.concurrency_limit.or(Some(0)), "concurrencyLimit")?;

    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    let existing_metadata = load_group_metadata_conn(tx.as_mut(), &group_name)
        .await
        .map_err(internal_error_tuple)?
        .unwrap_or_default();
    if bound_proxy_keys_was_updated {
        bound_proxy_keys = canonicalize_forward_proxy_bound_keys(state.as_ref(), &bound_proxy_keys)
            .await
            .map_err(internal_error_tuple)?;
    }
    let next_bound_proxy_keys = if bound_proxy_keys_was_updated {
        bound_proxy_keys
    } else {
        existing_metadata.bound_proxy_keys.clone()
    };
    let next_node_shunt_enabled = if node_shunt_enabled_was_updated {
        payload.node_shunt_enabled.unwrap_or(false)
    } else {
        existing_metadata.node_shunt_enabled
    };
    let node_shunt_was_disabled = existing_metadata.node_shunt_enabled
        && node_shunt_enabled_was_updated
        && !next_node_shunt_enabled;
    if next_node_shunt_enabled && next_bound_proxy_keys.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            missing_group_bound_proxy_error_message(group_name.trim()),
        ));
    }
    if !next_node_shunt_enabled
        && !next_bound_proxy_keys.is_empty()
        && (bound_proxy_keys_was_updated || node_shunt_was_disabled)
    {
        let has_selectable_bound_proxy_keys = {
            let manager = state.forward_proxy.lock().await;
            manager.has_selectable_bound_proxy_keys(&next_bound_proxy_keys)
        };
        if !has_selectable_bound_proxy_keys {
            return Err((
                StatusCode::BAD_REQUEST,
                "select at least one available proxy node or clear bindings before saving"
                    .to_string(),
            ));
        }
    }
    save_group_metadata_record_conn(
        tx.as_mut(),
        &group_name,
        UpstreamAccountGroupMetadata {
            note,
            bound_proxy_keys: next_bound_proxy_keys,
            node_shunt_enabled: next_node_shunt_enabled,
            single_account_rotation_enabled: if single_account_rotation_enabled_was_updated {
                payload.single_account_rotation_enabled.unwrap_or(false)
            } else {
                existing_metadata.single_account_rotation_enabled
            },
            upstream_429_retry_enabled: if upstream_429_retry_enabled_was_updated {
                payload.upstream_429_retry_enabled.unwrap_or(false)
            } else {
                existing_metadata.upstream_429_retry_enabled
            },
            upstream_429_max_retries: if upstream_429_max_retries_was_updated {
                normalized_upstream_429_max_retries
            } else {
                existing_metadata.upstream_429_max_retries
            },
            concurrency_limit: payload
                .concurrency_limit
                .map(|_| concurrency_limit)
                .unwrap_or(existing_metadata.concurrency_limit),
        },
    )
    .await
    .map_err(internal_error_tuple)?;
    if let Some(routing_rule) = payload.routing_rule.as_ref() {
        let policy_concurrency_limit = match routing_rule.concurrency_limit {
            OptionalField::Value(value) => Some(normalize_concurrency_limit(
                Some(value),
                "concurrencyLimit",
            )?),
            OptionalField::Missing | OptionalField::Null => None,
        };
        let policy_priority_tier = routing_rule
            .priority_tier_value()
            .map(|value| normalize_tag_priority_tier(Some(value)).map(|tier| tier.as_str()))
            .transpose()?;
        let policy_fast_mode_rewrite_mode = routing_rule
            .fast_mode_rewrite_mode_value()
            .map(|value| {
                normalize_tag_fast_mode_rewrite_mode(Some(value)).map(|mode| mode.as_str())
            })
            .transpose()?;
        let policy_image_tool_rewrite_mode = routing_rule
            .image_tool_rewrite_mode_value()
            .map(|value| {
                super::sync::normalize_upstream_image_tool_rewrite_mode(Some(value))
                    .map(|mode| mode.as_str())
            })
            .transpose()?;
        let available_models_json = match &routing_rule.available_models {
            OptionalField::Missing | OptionalField::Null => None,
            OptionalField::Value(value) => Some(
                encode_string_array_json(&normalize_available_models(
                    Some(value.clone()),
                    "availableModels",
                )?)
                .map_err(internal_error_tuple)?,
            ),
        };
        let timeout_patch = routing_rule.timeouts.clone().unwrap_or_default();
        let responses_first_byte_timeout_secs = normalize_optional_timeout_override_secs(
            &timeout_patch.responses_first_byte_timeout_secs,
            "responsesFirstByteTimeoutSecs",
        )?;
        let compact_first_byte_timeout_secs = normalize_optional_timeout_override_secs(
            &timeout_patch.compact_first_byte_timeout_secs,
            "compactFirstByteTimeoutSecs",
        )?;
        let responses_stream_timeout_secs = normalize_optional_timeout_override_secs(
            &timeout_patch.responses_stream_timeout_secs,
            "responsesStreamTimeoutSecs",
        )?;
        let compact_stream_timeout_secs = normalize_optional_timeout_override_secs(
            &timeout_patch.compact_stream_timeout_secs,
            "compactStreamTimeoutSecs",
        )?;
        let status_change_upstream_http_401 = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_401)
            .map_err(internal_error_tuple)?;
        let status_change_upstream_http_402 = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_402)
            .map_err(internal_error_tuple)?;
        let status_change_upstream_http_403 = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_403)
            .map_err(internal_error_tuple)?;
        let status_change_reauth_required = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED)
            .map_err(internal_error_tuple)?;
        let status_change_upstream_http_429_rate_limit = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT)
            .map_err(internal_error_tuple)?;
        let status_change_upstream_http_429_quota_exhausted = routing_rule
            .status_change_reason_field(
                UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            )
            .map_err(internal_error_tuple)?;
        let status_change_usage_snapshot_exhausted = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED)
            .map_err(internal_error_tuple)?;
        let status_change_quota_still_exhausted = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
            .map_err(internal_error_tuple)?;
        let status_change_transport_failure = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE)
            .map_err(internal_error_tuple)?;
        let status_change_upstream_server_overloaded = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED)
            .map_err(internal_error_tuple)?;
        let status_change_upstream_http_5xx = routing_rule
            .status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_5XX)
            .map_err(internal_error_tuple)?;
        sqlx::query(
            r#"
            UPDATE pool_upstream_account_group_notes
            SET policy_allow_cut_out = CASE WHEN ?2 != 0 THEN policy_allow_cut_out ELSE ?3 END,
                policy_allow_cut_in = CASE WHEN ?4 != 0 THEN policy_allow_cut_in ELSE ?5 END,
                policy_priority_tier = CASE WHEN ?6 != 0 THEN policy_priority_tier ELSE ?7 END,
                policy_fast_mode_rewrite_mode = CASE WHEN ?8 != 0 THEN policy_fast_mode_rewrite_mode ELSE ?9 END,
                policy_image_tool_rewrite_mode = CASE WHEN ?10 != 0 THEN policy_image_tool_rewrite_mode ELSE ?11 END,
                policy_concurrency_limit = CASE WHEN ?12 != 0 THEN policy_concurrency_limit ELSE ?13 END,
                policy_upstream_429_retry_enabled = CASE WHEN ?14 != 0 THEN policy_upstream_429_retry_enabled ELSE ?15 END,
                policy_upstream_429_max_retries = CASE WHEN ?16 != 0 THEN policy_upstream_429_max_retries ELSE ?17 END,
                policy_available_models_json = CASE
                    WHEN ?18 != 0 THEN policy_available_models_json
                    ELSE ?19
                END,
                policy_status_change_upstream_http_401 = CASE WHEN ?20 != 0 THEN policy_status_change_upstream_http_401 ELSE ?21 END,
                policy_status_change_upstream_http_402 = CASE WHEN ?22 != 0 THEN policy_status_change_upstream_http_402 ELSE ?23 END,
                policy_status_change_upstream_http_403 = CASE WHEN ?24 != 0 THEN policy_status_change_upstream_http_403 ELSE ?25 END,
                policy_status_change_reauth_required = CASE WHEN ?26 != 0 THEN policy_status_change_reauth_required ELSE ?27 END,
                policy_status_change_upstream_http_429_rate_limit = CASE WHEN ?28 != 0 THEN policy_status_change_upstream_http_429_rate_limit ELSE ?29 END,
                policy_status_change_upstream_http_429_quota_exhausted = CASE WHEN ?30 != 0 THEN policy_status_change_upstream_http_429_quota_exhausted ELSE ?31 END,
                policy_status_change_usage_snapshot_exhausted = CASE WHEN ?32 != 0 THEN policy_status_change_usage_snapshot_exhausted ELSE ?33 END,
                policy_status_change_quota_still_exhausted = CASE WHEN ?34 != 0 THEN policy_status_change_quota_still_exhausted ELSE ?35 END,
                policy_status_change_transport_failure = CASE WHEN ?36 != 0 THEN policy_status_change_transport_failure ELSE ?37 END,
                policy_status_change_upstream_server_overloaded = CASE WHEN ?38 != 0 THEN policy_status_change_upstream_server_overloaded ELSE ?39 END,
                policy_status_change_upstream_http_5xx = CASE WHEN ?40 != 0 THEN policy_status_change_upstream_http_5xx ELSE ?41 END,
                policy_responses_first_byte_timeout_secs = CASE WHEN ?42 != 0 THEN policy_responses_first_byte_timeout_secs ELSE ?43 END,
                policy_compact_first_byte_timeout_secs = CASE WHEN ?44 != 0 THEN policy_compact_first_byte_timeout_secs ELSE ?45 END,
                policy_responses_stream_timeout_secs = CASE WHEN ?46 != 0 THEN policy_responses_stream_timeout_secs ELSE ?47 END,
                policy_compact_stream_timeout_secs = CASE WHEN ?48 != 0 THEN policy_compact_stream_timeout_secs ELSE ?49 END
            WHERE group_name = ?1
            "#,
        )
        .bind(&group_name)
        .bind(if matches!(routing_rule.allow_cut_out, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&routing_rule.allow_cut_out))
        .bind(if matches!(routing_rule.allow_cut_in, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&routing_rule.allow_cut_in))
        .bind(if matches!(routing_rule.priority_tier, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(policy_priority_tier)
        .bind(if matches!(routing_rule.fast_mode_rewrite_mode, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(policy_fast_mode_rewrite_mode)
        .bind(if matches!(routing_rule.image_tool_rewrite_mode, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(policy_image_tool_rewrite_mode)
        .bind(if matches!(routing_rule.concurrency_limit, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(policy_concurrency_limit)
        .bind(if matches!(routing_rule.upstream_429_retry_enabled, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&routing_rule.upstream_429_retry_enabled))
        .bind(if matches!(routing_rule.upstream_429_max_retries, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_retry_count_to_i64(&routing_rule.upstream_429_max_retries))
        .bind(if matches!(routing_rule.available_models, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(available_models_json)
        .bind(if matches!(status_change_upstream_http_401, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_upstream_http_401))
        .bind(if matches!(status_change_upstream_http_402, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_upstream_http_402))
        .bind(if matches!(status_change_upstream_http_403, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_upstream_http_403))
        .bind(if matches!(status_change_reauth_required, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_reauth_required))
        .bind(if matches!(status_change_upstream_http_429_rate_limit, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_upstream_http_429_rate_limit))
        .bind(if matches!(status_change_upstream_http_429_quota_exhausted, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_upstream_http_429_quota_exhausted))
        .bind(if matches!(status_change_usage_snapshot_exhausted, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_usage_snapshot_exhausted))
        .bind(if matches!(status_change_quota_still_exhausted, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_quota_still_exhausted))
        .bind(if matches!(status_change_transport_failure, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_transport_failure))
        .bind(if matches!(status_change_upstream_server_overloaded, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_upstream_server_overloaded))
        .bind(if matches!(status_change_upstream_http_5xx, OptionalField::Missing) { 1_i64 } else { 0_i64 })
        .bind(optional_bool_to_i64(&status_change_upstream_http_5xx))
        .bind(if responses_first_byte_timeout_secs.is_none() { 1_i64 } else { 0_i64 })
        .bind(responses_first_byte_timeout_secs.flatten())
        .bind(if compact_first_byte_timeout_secs.is_none() { 1_i64 } else { 0_i64 })
        .bind(compact_first_byte_timeout_secs.flatten())
        .bind(if responses_stream_timeout_secs.is_none() { 1_i64 } else { 0_i64 })
        .bind(responses_stream_timeout_secs.flatten())
        .bind(if compact_stream_timeout_secs.is_none() { 1_i64 } else { 0_i64 })
        .bind(compact_stream_timeout_secs.flatten())
        .execute(tx.as_mut())
        .await
        .map_err(internal_error_tuple)?;
    }
    tx.commit().await.map_err(internal_error_tuple)?;

    let saved = load_group_metadata(&state.pool, Some(&group_name))
        .await
        .map_err(internal_error_tuple)?;
    let mut conn = state.pool.acquire().await.map_err(internal_error_tuple)?;
    let account_count = group_account_count_conn(&mut conn, &group_name)
        .await
        .map_err(internal_error_tuple)?;
    let routing_rule = load_group_routing_rule(&state.pool, &group_name)
        .await
        .map_err(internal_error_tuple)?
        .clone();
    let (effective_timeouts, timeout_field_sources, _) =
        load_effective_request_path_timeouts_for_group(
            &state.pool,
            &state.config,
            Some(&group_name),
        )
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(UpstreamAccountGroupSummary {
        group_name: group_name.clone(),
        account_count,
        note: saved.note,
        bound_proxy_keys: saved.bound_proxy_keys,
        node_shunt_enabled: saved.node_shunt_enabled,
        single_account_rotation_enabled: saved.single_account_rotation_enabled,
        upstream_429_retry_enabled: saved.upstream_429_retry_enabled,
        upstream_429_max_retries: saved.upstream_429_max_retries,
        concurrency_limit: saved.concurrency_limit,
        routing_rule,
        effective_timeouts,
        timeout_field_sources,
    }))
}

pub(crate) async fn delete_upstream_account_group(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(group_name): AxumPath<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;

    let group_name = normalize_optional_text(Some(group_name)).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "group name is required".to_string(),
        )
    })?;

    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    let account_count = group_account_count_conn(tx.as_mut(), &group_name)
        .await
        .map_err(internal_error_tuple)?;
    if account_count > 0 {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "group still has {account_count} account{}; move them out before deleting",
                if account_count == 1 { "" } else { "s" }
            ),
        ));
    }
    let deleted = sqlx::query(
        r#"
        DELETE FROM pool_upstream_account_group_notes
        WHERE group_name = ?1
        "#,
    )
    .bind(&group_name)
    .execute(tx.as_mut())
    .await
    .map_err(internal_error_tuple)?
    .rows_affected();
    if deleted == 0 {
        return Err((StatusCode::NOT_FOUND, "group not found".to_string()));
    }
    tx.commit().await.map_err(internal_error_tuple)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn get_upstream_account(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
    Query(params): Query<GetUpstreamAccountQuery>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    let detail = load_upstream_account_detail_with_actual_usage_options(
        state.as_ref(),
        id,
        LoadUpstreamAccountDetailOptions {
            include_recent_actions: params.include_recent_actions.unwrap_or(false),
        },
    )
    .await
    .map_err(internal_error_tuple)?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    Ok(Json(detail))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GetUpstreamAccountQuery {
    pub(crate) include_recent_actions: Option<bool>,
}
