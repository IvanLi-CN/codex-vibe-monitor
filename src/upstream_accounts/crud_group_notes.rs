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
    let filters = normalize_upstream_account_list_filters(&params);
    let load_summaries_started_at = Instant::now();
    let mut all_items = load_upstream_account_summaries_for_query(&state.pool, &params)
        .await
        .map_err(internal_error_tuple)?;
    let load_summaries_ms = load_summaries_started_at.elapsed().as_millis() as u64;
    let enrich_block_reason_started_at = Instant::now();
    enrich_node_shunt_routing_block_reasons(state.as_ref(), &mut all_items)
        .await
        .map_err(internal_error_tuple)?;
    let enrich_block_reason_ms = enrich_block_reason_started_at.elapsed().as_millis() as u64;
    let filtered_items = filter_upstream_account_summaries(all_items, &filters);
    let total = filtered_items.len();
    let metrics = build_upstream_account_list_metrics(&filtered_items);
    let offset = page.saturating_sub(1).saturating_mul(page_size);
    let mut items = if offset >= total {
        Vec::new()
    } else {
        filtered_items
            .into_iter()
            .skip(offset)
            .take(page_size)
            .collect::<Vec<_>>()
    };
    let enrich_window_usage_started_at = Instant::now();
    enrich_window_actual_usage_for_summaries(state.as_ref(), &mut items)
        .await
        .map_err(internal_error_tuple)?;
    let enrich_window_usage_ms = enrich_window_usage_started_at.elapsed().as_millis() as u64;
    let load_groups_started_at = Instant::now();
    let mut groups = load_upstream_account_groups(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    for group in &mut groups {
        group.bound_proxy_keys =
            canonicalize_forward_proxy_bound_keys(state.as_ref(), &group.bound_proxy_keys)
                .await
                .map_err(internal_error_tuple)?;
    }
    let load_groups_ms = load_groups_started_at.elapsed().as_millis() as u64;
    let has_ungrouped_accounts = has_ungrouped_upstream_accounts(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let load_routing_started_at = Instant::now();
    let routing = load_pool_routing_settings_seeded(&state.pool, &state.config)
        .await
        .map_err(internal_error_tuple)?;
    let load_routing_ms = load_routing_started_at.elapsed().as_millis() as u64;
    let total_ms = started_at.elapsed().as_millis() as u64;
    tracing::info!(
        page,
        page_size,
        total,
        load_summaries_ms,
        enrich_block_reason_ms,
        enrich_window_usage_ms,
        load_groups_ms,
        load_routing_ms,
        total_ms,
        "upstream accounts roster request completed"
    );
    Ok(Json(UpstreamAccountListResponse {
        writes_enabled: state.upstream_accounts.writes_enabled(),
        items,
        total,
        page,
        page_size,
        metrics,
        groups,
        forward_proxy_nodes: Vec::new(),
        has_ungrouped_accounts,
        routing: build_pool_routing_settings_response(state.as_ref(), &routing),
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
    if requested_keys.is_empty() && !params.include_current {
        return Ok(Json(Vec::new()));
    }
    let nodes = build_forward_proxy_binding_nodes_response_with_options(
        state.as_ref(),
        &requested_keys,
        false,
    )
    .await
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
        tag_ids: base.tag_ids,
        ..ListUpstreamAccountsQuery::default()
    };

    for (key, value) in url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()) {
        match key.as_ref() {
            "workStatus" => params.work_status.push(value.into_owned()),
            "enableStatus" => params.enable_status.push(value.into_owned()),
            "healthStatus" => params.health_status.push(value.into_owned()),
            _ => {}
        }
    }

    Ok(params)
}

pub(crate) async fn list_tags(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListTagsQuery>,
) -> Result<Json<TagListResponse>, (StatusCode, String)> {
    let items = load_tag_summaries(&state.pool, &params)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(TagListResponse {
        writes_enabled: state.upstream_accounts.writes_enabled(),
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
        payload.guard_enabled,
        payload.lookback_hours,
        payload.max_conversations,
        payload.allow_cut_out,
        payload.allow_cut_in,
        payload.priority_tier.as_deref(),
        payload.fast_mode_rewrite_mode.as_deref(),
        payload.concurrency_limit,
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
    let name = match payload.name {
        Some(value) => normalize_tag_name(&value)?,
        None => existing.name.clone(),
    };
    let rule = normalize_tag_rule(
        payload.guard_enabled.unwrap_or(existing.guard_enabled != 0),
        payload.lookback_hours.or(existing.lookback_hours),
        payload.max_conversations.or(existing.max_conversations),
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
    if !group_has_accounts_conn(tx.as_mut(), &group_name)
        .await
        .map_err(internal_error_tuple)?
    {
        return Err((StatusCode::NOT_FOUND, "group not found".to_string()));
    }
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
    tx.commit().await.map_err(internal_error_tuple)?;

    let saved = load_group_metadata(&state.pool, Some(&group_name))
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(UpstreamAccountGroupSummary {
        group_name,
        note: saved.note,
        bound_proxy_keys: saved.bound_proxy_keys,
        node_shunt_enabled: saved.node_shunt_enabled,
        upstream_429_retry_enabled: saved.upstream_429_retry_enabled,
        upstream_429_max_retries: saved.upstream_429_max_retries,
        concurrency_limit: saved.concurrency_limit,
    }))
}

pub(crate) async fn get_upstream_account(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    let detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    Ok(Json(detail))
}
