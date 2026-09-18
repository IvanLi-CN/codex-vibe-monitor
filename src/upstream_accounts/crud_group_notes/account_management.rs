pub(crate) async fn list_upstream_accounts_from_params(
    state: Arc<AppState>,
    params: ListUpstreamAccountsQuery,
) -> Result<Json<UpstreamAccountListResponse>, (StatusCode, String)> {
    let started_at = Instant::now();

    normalize_upstream_account_kind_filter(params.kind.as_deref())
        .map_err(|err| (StatusCode::BAD_REQUEST, err))?;

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
    let groups = if params.kind.as_deref() == Some(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX) {
        Vec::new()
    } else {
        load_canonicalized_upstream_account_groups(state.as_ref())
            .await
            .map_err(internal_error_tuple)?
    };
    let load_groups_ms = load_groups_started_at.elapsed().as_millis() as u64;
    let enrich_block_reason_started_at = Instant::now();
    enrich_transport_decode_sticky_escape_routing_block_reasons(state.as_ref(), &mut all_items)
        .await
        .map_err(internal_error_tuple)?;
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
    let (items, response_page, response_page_size) =
        paginate_upstream_account_summaries(&filtered_items, page, page_size, include_all);
    let roster_core_ms = started_at.elapsed().as_millis() as u64;
    let usage_batch_ms = 0_u64;
    let has_ungrouped_accounts =
        has_ungrouped_upstream_accounts(&state.pool, params.kind.as_deref())
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

async fn load_api_key_group_migration_preflight(
    pool: &Pool<Sqlite>,
) -> Result<ApiKeyGroupMigrationPreflightResponse> {
    let rows = load_api_key_group_migration_rows(pool).await?;

    let mut blocked = BTreeSet::new();
    let mut group_metadata_by_name = BTreeMap::new();
    let group_names = rows
        .iter()
        .filter_map(|row| {
            row.1
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .collect::<BTreeSet<_>>();
    for group_name in group_names {
        let metadata = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
            "SELECT node_shunt_enabled, single_account_rotation_enabled FROM pool_upstream_account_group_notes WHERE group_name = ?1",
        )
        .bind(group_name)
        .fetch_optional(pool)
        .await?;
        if let Some((node_shunt, rotation)) = metadata {
            if node_shunt.unwrap_or_default() != 0 {
                blocked.insert("node_shunt".to_string());
            }
            if rotation.unwrap_or_default() != 0 {
                blocked.insert("single_account_rotation".to_string());
            }
        }
        group_metadata_by_name.insert(
            group_name.to_string(),
            load_group_metadata(pool, Some(group_name)).await?,
        );
    }
    if rows.iter().any(|row| row.2 != 0) {
        blocked.insert("mother_account".to_string());
    }

    let snapshot = rows
        .iter()
        .map(|row| {
            let group_name = row.1.as_deref().unwrap_or_default();
            let metadata = group_metadata_by_name.get(group_name);
            serde_json::json!({
                "id": row.0,
                "groupName": row.1,
                "isMother": row.2 != 0,
                "upstreamBaseUrl": row.3,
                "localPrimaryLimit": row.4,
                "localSecondaryLimit": row.5,
                "localLimitUnit": row.6,
                "portableGroup": {
                    "note": metadata.and_then(|item| item.note.clone()),
                    "boundProxyKeys": metadata.map(|item| item.bound_proxy_keys.clone()).unwrap_or_default(),
                    "concurrencyLimit": metadata.map(|item| item.concurrency_limit).unwrap_or_default(),
                    "upstream429RetryEnabled": metadata.map(|item| item.upstream_429_retry_enabled).unwrap_or_default(),
                    "upstream429MaxRetries": metadata.map(|item| item.upstream_429_max_retries).unwrap_or_default(),
                },
            })
        })
        .collect::<Vec<_>>();
    let hash_input = serde_json::json!({
        "accounts": snapshot,
        "blockedStrategies": blocked.clone(),
    });
    let confirmation_hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&hash_input)?));

    Ok(ApiKeyGroupMigrationPreflightResponse {
        confirmation_hash,
        api_key_count: rows.len(),
        portable_fields: vec![
            "upstreamBaseUrl".to_string(),
            "localPrimaryLimit".to_string(),
            "localSecondaryLimit".to_string(),
            "localLimitUnit".to_string(),
            "boundProxyKeys".to_string(),
            "note".to_string(),
            "tags".to_string(),
        ],
        // Transit accounts never consume group-only strategies. Keep this field for
        // response compatibility, but do not surface an obsolete confirmation step.
        blocked_strategies: Vec::new(),
        can_migrate: true,
    })
}

pub(crate) async fn preflight_api_key_group_migration(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ApiKeyGroupMigrationPreflightResponse>, (StatusCode, String)> {
    load_api_key_group_migration_preflight(&state.pool)
        .await
        .map(Json)
        .map_err(internal_error_tuple)
}

pub(crate) async fn confirm_api_key_group_migration(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(_payload): Json<ConfirmApiKeyGroupMigrationRequest>,
) -> Result<Json<ApiKeyGroupMigrationResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let preflight = load_api_key_group_migration_preflight(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let migrated_count = ensure_api_key_transit_proxy_bindings_migrated(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(ApiKeyGroupMigrationResponse {
        migrated_count,
        confirmation_hash: preflight.confirmation_hash,
        audit_action: API_KEY_TRANSIT_PROXY_MIGRATION_AUDIT_ACTION.to_string(),
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
            "kind" => params.kind = Some(value.into_owned()),
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

pub(crate) fn normalize_upstream_account_kind_filter(
    value: Option<&str>,
) -> Result<Option<&'static str>, String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    match value {
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX => Ok(Some(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)),
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX => Ok(Some(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX)),
        other => Err(format!(
            "invalid kind value `{other}`; expected oauth_codex or api_key_codex"
        )),
    }
}
