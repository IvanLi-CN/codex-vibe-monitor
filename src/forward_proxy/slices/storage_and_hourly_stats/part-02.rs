pub(crate) async fn build_forward_proxy_binding_node_catalog(
    state: &AppState,
    extra_proxy_keys: &[String],
) -> Result<(Vec<ForwardProxyBindingNodeResponse>, HashSet<String>)> {
    let mut nodes = {
        let manager = state.forward_proxy.lock().await;
        manager.binding_nodes()
    };
    let current_node_keys = nodes
        .iter()
        .map(|node| node.key.clone())
        .collect::<HashSet<_>>();

    let mut seen = current_node_keys.clone();
    let extra_keys = extra_proxy_keys
        .iter()
        .map(|key| key.trim())
        .filter(|key| !key.is_empty())
        .map(ToOwned::to_owned)
        .filter(|key| seen.insert(key.clone()))
        .collect::<Vec<_>>();
    let metadata_lookup_keys = nodes
        .iter()
        .map(|node| node.key.clone())
        .chain(extra_keys.iter().cloned())
        .collect::<Vec<_>>();
    let metadata_map =
        load_forward_proxy_metadata_history(&state.pool, &metadata_lookup_keys).await?;

    for node in &mut nodes {
        if let Some(metadata) = metadata_map.get(&node.key) {
            node.egress_ip = metadata.egress_ip.clone();
            node.egress_ip_checked_at = metadata.egress_ip_checked_at.clone();
            node.egress_ip_provider = metadata.egress_ip_provider.clone();
            node.egress_ip_error = metadata.egress_ip_error.clone();
            node.egress_ip_error_at = metadata.egress_ip_error_at.clone();
        }
    }

    {
        let manager = state.forward_proxy.lock().await;
        for proxy_key in extra_keys {
            let maybe_current_key = manager
                .resolve_current_or_historical_bound_proxy_key(
                    &proxy_key,
                    metadata_map.get(&proxy_key),
                )
                .filter(|candidate| current_node_keys.contains(candidate));
            if let Some(current_key) = maybe_current_key {
                if let Some(node) = nodes.iter_mut().find(|node| node.key == current_key)
                    && node.key != proxy_key
                {
                    node.alias_keys.push(proxy_key.clone());
                    node.alias_keys.sort();
                    node.alias_keys.dedup();
                }
                continue;
            }
            let metadata = metadata_map.get(&proxy_key);
            nodes.push(ForwardProxyBindingNodeResponse {
                key: proxy_key.clone(),
                alias_keys: Vec::new(),
                source: "missing".to_string(),
                display_name: metadata
                    .map(|item| item.display_name.clone())
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| proxy_key.clone()),
                protocol_label: "UNKNOWN".to_string(),
                egress_ip: metadata.and_then(|item| item.egress_ip.clone()),
                egress_ip_checked_at: metadata.and_then(|item| item.egress_ip_checked_at.clone()),
                egress_ip_provider: metadata.and_then(|item| item.egress_ip_provider.clone()),
                egress_ip_error: metadata.and_then(|item| item.egress_ip_error.clone()),
                egress_ip_error_at: metadata.and_then(|item| item.egress_ip_error_at.clone()),
                penalized: false,
                selectable: false,
                last24h: Vec::new(),
            });
        }
    }

    Ok((nodes, current_node_keys))
}

pub(crate) fn apply_forward_proxy_binding_hourly_buckets(
    nodes: &mut [ForwardProxyBindingNodeResponse],
    current_node_keys: &HashSet<String>,
    hourly_map: &HashMap<String, HashMap<i64, ForwardProxyHourlyStatsPoint>>,
    range_start_epoch: i64,
    bucket_seconds: i64,
    bucket_count: i64,
) -> Result<()> {
    for node in nodes {
        let hourly = hourly_map.get(&node.key);
        node.alias_keys.sort();
        node.alias_keys.dedup();
        node.last24h = if current_node_keys.contains(&node.key) || hourly.is_some() {
            build_forward_proxy_hourly_buckets(
                hourly,
                range_start_epoch,
                bucket_seconds,
                bucket_count,
            )?
        } else {
            Vec::new()
        };
    }
    Ok(())
}

pub(crate) async fn build_forward_proxy_binding_nodes_response(
    state: &AppState,
    extra_proxy_keys: &[String],
) -> Result<Vec<ForwardProxyBindingNodeResponse>> {
    build_forward_proxy_binding_nodes_response_with_options(state, extra_proxy_keys, true).await
}

pub(crate) async fn build_group_forward_proxy_binding_nodes_response(
    state: &AppState,
    extra_proxy_keys: &[String],
    _group_name: &str,
) -> Result<Vec<ForwardProxyBindingNodeResponse>> {
    build_forward_proxy_binding_nodes_response_with_options(state, extra_proxy_keys, false).await
}

pub(crate) async fn build_forward_proxy_binding_nodes_response_with_options(
    state: &AppState,
    extra_proxy_keys: &[String],
    catch_up_hourly_rollups: bool,
) -> Result<Vec<ForwardProxyBindingNodeResponse>> {
    const BUCKET_SECONDS: i64 = 3600;
    const BUCKET_COUNT: i64 = 24;

    let _ = catch_up_hourly_rollups;
    let now_epoch = Utc::now().timestamp();
    let range_end_epoch = align_bucket_epoch(now_epoch, BUCKET_SECONDS, 0) + BUCKET_SECONDS;
    let range_start_epoch = range_end_epoch - BUCKET_COUNT * BUCKET_SECONDS;
    let range_start = Utc
        .timestamp_opt(range_start_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid owner-facing node health archive range start epoch"))?;
    let range_end = Utc
        .timestamp_opt(range_end_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid owner-facing node health archive range end epoch"))?;
    let pending_archive_file_paths = load_pending_pool_upstream_node_health_archive_file_paths(
        &state.pool,
        &db_occurred_at_lower_bound(range_start),
        &db_occurred_at_lower_bound(range_end),
    )
    .await?;

    let (mut nodes, current_node_keys) =
        build_forward_proxy_binding_node_catalog(state, extra_proxy_keys).await?;
    if nodes.is_empty() {
        return Ok(nodes);
    }

    let final_node_keys = nodes
        .iter()
        .map(|node| node.key.clone())
        .collect::<HashSet<_>>();
    let hourly_map = query_pool_upstream_binding_hourly_stats(
        state,
        range_start_epoch,
        range_end_epoch,
        Some(&final_node_keys),
        &pending_archive_file_paths,
        None,
    )
    .await?;

    apply_forward_proxy_binding_hourly_buckets(
        &mut nodes,
        &current_node_keys,
        &hourly_map,
        range_start_epoch,
        BUCKET_SECONDS,
        BUCKET_COUNT,
    )?;
    nodes.sort_by(|lhs, rhs| lhs.display_name.cmp(&rhs.display_name));

    Ok(nodes)
}

pub(crate) fn build_forward_proxy_hourly_buckets(
    hourly: Option<&HashMap<i64, ForwardProxyHourlyStatsPoint>>,
    range_start_epoch: i64,
    bucket_seconds: i64,
    bucket_count: i64,
) -> Result<Vec<ForwardProxyHourlyBucketResponse>> {
    (0..bucket_count)
        .map(|index| {
            let bucket_start_epoch = range_start_epoch + index * bucket_seconds;
            let bucket_end_epoch = bucket_start_epoch + bucket_seconds;
            let point = hourly
                .and_then(|items| items.get(&bucket_start_epoch))
                .cloned()
                .unwrap_or_default();
            let bucket_start = Utc
                .timestamp_opt(bucket_start_epoch, 0)
                .single()
                .ok_or_else(|| anyhow!("invalid forward proxy bucket start epoch"))?;
            let bucket_end = Utc
                .timestamp_opt(bucket_end_epoch, 0)
                .single()
                .ok_or_else(|| anyhow!("invalid forward proxy bucket end epoch"))?;
            Ok(ForwardProxyHourlyBucketResponse {
                bucket_start: format_utc_iso(bucket_start),
                bucket_end: format_utc_iso(bucket_end),
                success_count: point.success_count,
                failure_count: point.failure_count,
            })
        })
        .collect::<Result<Vec<_>>>()
}

pub(crate) async fn build_forward_proxy_live_stats_response(
    state: &AppState,
) -> Result<ForwardProxyLiveStatsResponse> {
    const BUCKET_SECONDS: i64 = 3600;
    const BUCKET_COUNT: i64 = 24;

    let now_utc = Utc::now();
    let window_end_at = db_occurred_at_lower_bound(now_utc + ChronoDuration::seconds(1));
    let widest_window_start_at = db_occurred_at_lower_bound(now_utc - ChronoDuration::days(7));
    let pending_archive_file_paths = load_pending_pool_upstream_node_health_archive_file_paths(
        &state.pool,
        &widest_window_start_at,
        &window_end_at,
    )
    .await?;

    let (runtime_rows, runtime_health_key_by_proxy_key) = {
        let manager = state.forward_proxy.lock().await;
        let mut runtime_rows = manager.snapshot_runtime().into_iter().collect::<Vec<_>>();
        ensure_owner_facing_direct_runtime_row(
            &mut runtime_rows,
            manager.algo,
            manager.settings.insert_direct,
        );
        let runtime_health_key_by_proxy_key = runtime_rows
            .iter()
            .map(|runtime| {
                let health_key = manager
                    .canonicalize_bound_proxy_key(&runtime.proxy_key, None)
                    .unwrap_or_else(|| runtime.proxy_key.clone());
                (runtime.proxy_key.clone(), health_key)
            })
            .collect::<HashMap<_, _>>();
        (runtime_rows, runtime_health_key_by_proxy_key)
    };
    let runtime_proxy_keys = runtime_rows
        .iter()
        .map(|runtime| runtime.proxy_key.clone())
        .collect::<Vec<_>>();
    let runtime_health_keys = runtime_health_key_by_proxy_key
        .values()
        .cloned()
        .collect::<HashSet<_>>();
    let pending_archive_rows = load_pending_pool_upstream_binding_attempt_rows(
        &pending_archive_file_paths,
        &widest_window_start_at,
        &window_end_at,
    )
    .await?;

    let windows = [
        (ChronoDuration::minutes(1), 0usize),
        (ChronoDuration::minutes(15), 1usize),
        (ChronoDuration::hours(1), 2usize),
        (ChronoDuration::days(1), 3usize),
        (ChronoDuration::days(7), 4usize),
    ];
    let now_utc = Utc::now();
    let window_end_at = db_occurred_at_lower_bound(now_utc + ChronoDuration::seconds(1));
    let mut window_maps: Vec<HashMap<String, ForwardProxyAttemptWindowStats>> = Vec::new();
    for (window_duration, _) in &windows {
        let window_start_at = db_occurred_at_lower_bound(now_utc - *window_duration);
        window_maps.push(
            query_pool_upstream_binding_window_stats(
                state,
                &window_start_at,
                &window_end_at,
                Some(&runtime_health_keys),
                &pending_archive_file_paths,
                Some(&pending_archive_rows),
            )
            .await?,
        );
    }

    let now_epoch = now_utc.timestamp();
    let range_end_epoch = align_bucket_epoch(now_epoch, BUCKET_SECONDS, 0) + BUCKET_SECONDS;
    let range_start_epoch = range_end_epoch - BUCKET_COUNT * BUCKET_SECONDS;
    let hourly_map = query_pool_upstream_binding_hourly_stats(
        state,
        range_start_epoch,
        range_end_epoch,
        Some(&runtime_health_keys),
        &pending_archive_file_paths,
        Some(&pending_archive_rows),
    )
    .await?;
    let weight_hourly_map =
        query_forward_proxy_weight_hourly_stats(&state.pool, range_start_epoch, range_end_epoch)
            .await?;
    let weight_carry_map =
        query_forward_proxy_weight_last_before(&state.pool, range_start_epoch, &runtime_proxy_keys)
            .await?;

    let mut nodes = runtime_rows
        .into_iter()
        .map(|runtime| {
            let proxy_key = runtime.proxy_key.clone();
            let health_key = runtime_health_key_by_proxy_key
                .get(&proxy_key)
                .unwrap_or(&proxy_key);
            let penalized = runtime.is_penalized();
            let runtime_weight = runtime.weight;
            let stats_for = |index: usize, key: &str| {
                window_maps[index]
                    .get(key)
                    .cloned()
                    .map(ForwardProxyWindowStatsResponse::from)
                    .unwrap_or_default()
            };
            let hourly = hourly_map.get(health_key);
            let weight_hourly = weight_hourly_map.get(&proxy_key);
            let mut carry_weight = weight_carry_map
                .get(&proxy_key)
                .copied()
                .unwrap_or(runtime_weight);
            let one_minute = stats_for(0, health_key);
            let fifteen_minutes = stats_for(1, health_key);
            let one_hour = stats_for(2, health_key);
            let one_day = stats_for(3, health_key);
            let seven_days = stats_for(4, health_key);
            let last24h = build_forward_proxy_hourly_buckets(
                hourly,
                range_start_epoch,
                BUCKET_SECONDS,
                BUCKET_COUNT,
            )?;
            let weight24h = (0..BUCKET_COUNT)
                .map(|index| {
                    let bucket_start_epoch = range_start_epoch + index * BUCKET_SECONDS;
                    let bucket_end_epoch = bucket_start_epoch + BUCKET_SECONDS;
                    let point = weight_hourly.and_then(|items| items.get(&bucket_start_epoch));
                    let (sample_count, min_weight, max_weight, avg_weight, last_weight) =
                        if let Some(point) = point {
                            carry_weight = point.last_weight;
                            (
                                point.sample_count,
                                point.min_weight,
                                point.max_weight,
                                point.avg_weight,
                                point.last_weight,
                            )
                        } else {
                            (0, carry_weight, carry_weight, carry_weight, carry_weight)
                        };
                    let bucket_start = Utc
                        .timestamp_opt(bucket_start_epoch, 0)
                        .single()
                        .ok_or_else(|| {
                            anyhow!("invalid forward proxy weight bucket start epoch")
                        })?;
                    let bucket_end = Utc
                        .timestamp_opt(bucket_end_epoch, 0)
                        .single()
                        .ok_or_else(|| anyhow!("invalid forward proxy weight bucket end epoch"))?;
                    Ok(ForwardProxyWeightHourlyBucketResponse {
                        bucket_start: format_utc_iso(bucket_start),
                        bucket_end: format_utc_iso(bucket_end),
                        sample_count,
                        min_weight,
                        max_weight,
                        avg_weight,
                        last_weight,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(ForwardProxyLiveNodeResponse {
                key: proxy_key,
                source: runtime.source,
                display_name: runtime.display_name,
                endpoint_url: runtime.endpoint_url,
                weight: runtime_weight,
                penalized,
                stats: ForwardProxyStatsResponse {
                    one_minute,
                    fifteen_minutes,
                    one_hour,
                    one_day,
                    seven_days,
                },
                last24h,
                weight24h,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    nodes.sort_by(|lhs, rhs| lhs.display_name.cmp(&rhs.display_name));

    let range_start = Utc
        .timestamp_opt(range_start_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid forward proxy range start epoch"))?;
    let range_end = Utc
        .timestamp_opt(range_end_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid forward proxy range end epoch"))?;

    Ok(ForwardProxyLiveStatsResponse {
        range_start: format_utc_iso(range_start),
        range_end: format_utc_iso(range_end),
        bucket_seconds: BUCKET_SECONDS,
        nodes,
    })
}

pub(crate) async fn build_forward_proxy_timeseries_response(
    state: &AppState,
    range_window: RangeWindow,
) -> Result<ForwardProxyTimeseriesResponse> {
    const BUCKET_SECONDS: i64 = 3600;

    let (runtime_rows, runtime_health_key_by_proxy_key) = {
        let manager = state.forward_proxy.lock().await;
        let mut runtime_rows = manager.snapshot_runtime().into_iter().collect::<Vec<_>>();
        ensure_owner_facing_direct_runtime_row(
            &mut runtime_rows,
            manager.algo,
            manager.settings.insert_direct,
        );
        let runtime_health_key_by_proxy_key = runtime_rows
            .iter()
            .map(|runtime| {
                let health_key = manager
                    .canonicalize_bound_proxy_key(&runtime.proxy_key, None)
                    .unwrap_or_else(|| runtime.proxy_key.clone());
                (runtime.proxy_key.clone(), health_key)
            })
            .collect::<HashMap<_, _>>();
        (runtime_rows, runtime_health_key_by_proxy_key)
    };
    let runtime_map = runtime_rows
        .into_iter()
        .map(|runtime| (runtime.proxy_key.clone(), runtime))
        .collect::<HashMap<_, _>>();
    let covered_health_keys = runtime_health_key_by_proxy_key
        .values()
        .cloned()
        .collect::<HashSet<_>>();

    let start_epoch = range_window.start.timestamp();
    let end_epoch = range_window.end.timestamp();
    let query_start_epoch = align_bucket_epoch(start_epoch, BUCKET_SECONDS, 0);
    let query_end_epoch = ceil_hour_epoch(end_epoch);
    let query_start = Utc
        .timestamp_opt(query_start_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid owner-facing node health archive timeseries start"))?;
    let query_end = Utc
        .timestamp_opt(query_end_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid owner-facing node health archive timeseries end"))?;
    let pending_archive_file_paths = load_pending_pool_upstream_node_health_archive_file_paths(
        &state.pool,
        &db_occurred_at_lower_bound(query_start),
        &db_occurred_at_lower_bound(query_end),
    )
    .await?;
    let fill_start_epoch = query_start_epoch;
    let fill_end_epoch = query_end_epoch;

    let hourly_map = query_pool_upstream_binding_hourly_stats(
        state,
        query_start_epoch,
        query_end_epoch,
        None,
        &pending_archive_file_paths,
        None,
    )
    .await?;
    let weight_hourly_map =
        query_forward_proxy_weight_hourly_stats(&state.pool, query_start_epoch, query_end_epoch)
            .await?;

    let mut seen = HashSet::new();
    let mut proxy_keys = Vec::new();
    for key in runtime_map.keys() {
        if seen.insert(key.clone()) {
            proxy_keys.push(key.clone());
        }
    }
    for key in hourly_map.keys() {
        if !covered_health_keys.contains(key) && seen.insert(key.clone()) {
            proxy_keys.push(key.clone());
        }
    }
    for key in weight_hourly_map.keys() {
        if seen.insert(key.clone()) {
            proxy_keys.push(key.clone());
        }
    }
    proxy_keys.sort();

    let metadata_map = load_forward_proxy_metadata_history(&state.pool, &proxy_keys).await?;
    let weight_carry_map =
        query_forward_proxy_weight_last_before(&state.pool, fill_start_epoch, &proxy_keys).await?;

    let mut nodes = proxy_keys
        .into_iter()
        .map(|proxy_key| {
            let runtime = runtime_map.get(&proxy_key);
            let metadata = metadata_map.get(&proxy_key);
            let request_lookup_key = runtime_health_key_by_proxy_key
                .get(&proxy_key)
                .map(String::as_str)
                .unwrap_or(proxy_key.as_str());
            let request_points = hourly_map.get(request_lookup_key);
            let weight_points = weight_hourly_map.get(&proxy_key);
            let fallback_weight = weight_carry_map
                .get(&proxy_key)
                .copied()
                .or_else(|| {
                    weight_points
                        .and_then(|items| items.iter().next().map(|(_, point)| point.last_weight))
                })
                .or_else(|| runtime.map(|item| item.weight))
                .unwrap_or(1.0);
            let mut carry_weight = fallback_weight;

            let bucket_count = (fill_end_epoch - fill_start_epoch).max(0) / BUCKET_SECONDS;
            let buckets = (0..bucket_count)
                .map(|index| {
                    let bucket_start_epoch = fill_start_epoch + index * BUCKET_SECONDS;
                    let bucket_end_epoch = bucket_start_epoch + BUCKET_SECONDS;
                    let point = request_points
                        .and_then(|items| items.get(&bucket_start_epoch))
                        .cloned()
                        .unwrap_or_default();
                    let bucket_start = Utc
                        .timestamp_opt(bucket_start_epoch, 0)
                        .single()
                        .ok_or_else(|| anyhow!("invalid forward proxy bucket start epoch"))?;
                    let bucket_end = Utc
                        .timestamp_opt(bucket_end_epoch, 0)
                        .single()
                        .ok_or_else(|| anyhow!("invalid forward proxy bucket end epoch"))?;
                    Ok(ForwardProxyHourlyBucketResponse {
                        bucket_start: format_utc_iso(bucket_start),
                        bucket_end: format_utc_iso(bucket_end),
                        success_count: point.success_count,
                        failure_count: point.failure_count,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let weight_buckets = (0..bucket_count)
                .map(|index| {
                    let bucket_start_epoch = fill_start_epoch + index * BUCKET_SECONDS;
                    let bucket_end_epoch = bucket_start_epoch + BUCKET_SECONDS;
                    let point = weight_points.and_then(|items| items.get(&bucket_start_epoch));
                    let (sample_count, min_weight, max_weight, avg_weight, last_weight) =
                        if let Some(point) = point {
                            carry_weight = point.last_weight;
                            (
                                point.sample_count,
                                point.min_weight,
                                point.max_weight,
                                point.avg_weight,
                                point.last_weight,
                            )
                        } else {
                            (0, carry_weight, carry_weight, carry_weight, carry_weight)
                        };
                    let bucket_start = Utc
                        .timestamp_opt(bucket_start_epoch, 0)
                        .single()
                        .ok_or_else(|| {
                            anyhow!("invalid forward proxy weight bucket start epoch")
                        })?;
                    let bucket_end = Utc
                        .timestamp_opt(bucket_end_epoch, 0)
                        .single()
                        .ok_or_else(|| anyhow!("invalid forward proxy weight bucket end epoch"))?;
                    Ok(ForwardProxyWeightHourlyBucketResponse {
                        bucket_start: format_utc_iso(bucket_start),
                        bucket_end: format_utc_iso(bucket_end),
                        sample_count,
                        min_weight,
                        max_weight,
                        avg_weight,
                        last_weight,
                    })
                })
                .collect::<Result<Vec<_>>>()?;

            Ok(ForwardProxyTimeseriesNodeResponse {
                key: proxy_key.clone(),
                source: runtime
                    .map(|item| item.source.clone())
                    .or_else(|| metadata.map(|item| item.source.clone()))
                    .unwrap_or_else(|| {
                        if proxy_key == FORWARD_PROXY_DIRECT_KEY {
                            FORWARD_PROXY_SOURCE_DIRECT.to_string()
                        } else {
                            "archived".to_string()
                        }
                    }),
                display_name: runtime
                    .map(|item| item.display_name.clone())
                    .or_else(|| metadata.map(|item| item.display_name.clone()))
                    .unwrap_or_else(|| {
                        if proxy_key == FORWARD_PROXY_DIRECT_KEY {
                            FORWARD_PROXY_DIRECT_LABEL.to_string()
                        } else {
                            proxy_key.clone()
                        }
                    }),
                endpoint_url: runtime
                    .and_then(|item| item.endpoint_url.clone())
                    .or_else(|| metadata.and_then(|item| item.endpoint_url.clone())),
                weight: runtime.map(|item| item.weight).unwrap_or(fallback_weight),
                penalized: runtime.map(|item| item.is_penalized()).unwrap_or(false),
                buckets,
                weight_buckets,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    nodes.sort_by(|lhs, rhs| lhs.display_name.cmp(&rhs.display_name));

    Ok(ForwardProxyTimeseriesResponse {
        range_start: format_utc_iso(range_window.start),
        range_end: format_utc_iso(range_window.display_end),
        bucket_seconds: BUCKET_SECONDS,
        effective_bucket: "1h".to_string(),
        available_buckets: vec!["1h".to_string()],
        nodes,
    })
}

pub(crate) async fn put_forward_proxy_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ForwardProxySettingsUpdateRequest>,
) -> Result<Json<ForwardProxySettingsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }

    let next: ForwardProxySettings = payload.into();
    let _update_guard = state.forward_proxy_settings_update_lock.lock().await;

    let (previous_settings, known_subscription_keys_before_settings) = {
        let manager = state.forward_proxy.lock().await;
        let before = snapshot_active_forward_proxy_endpoints(&manager);
        (
            manager.settings.clone(),
            before
                .into_iter()
                .filter(|endpoint| endpoint.source == FORWARD_PROXY_SOURCE_SUBSCRIPTION)
                .map(|endpoint| endpoint.key)
                .collect::<HashSet<_>>(),
        )
    };
    save_forward_proxy_settings(&state.pool, next.clone())
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let added_manual_endpoints = {
        let mut manager = state.forward_proxy.lock().await;
        let before = snapshot_active_forward_proxy_endpoints(&manager);
        manager.apply_settings(next.clone());
        let after = snapshot_active_forward_proxy_endpoints(&manager);
        compute_added_forward_proxy_endpoints(&before, &after)
    };
    if let Err(err) = sync_forward_proxy_routes(state.as_ref()).await {
        if state.shutdown.is_cancelled() {
            let mut manager = state.forward_proxy.lock().await;
            if let Err(rollback_err) =
                save_forward_proxy_settings(&state.pool, previous_settings.clone()).await
            {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(
                        "failed to roll back forward proxy settings after shutdown interruption: {rollback_err}"
                    ),
                ));
            }
            manager.apply_settings(previous_settings);
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                format!("forward proxy settings update interrupted by shutdown: {err}"),
            ));
        }
        warn!(
            error = %err,
            "failed to sync forward proxy routes after settings update"
        );
    }
    if let Err(err) = refresh_forward_proxy_subscriptions(
        state.clone(),
        true,
        Some(known_subscription_keys_before_settings),
    )
    .await
    {
        warn!(error = %err, "failed to refresh forward proxy subscriptions after settings update");
    }
    if !added_manual_endpoints.is_empty() {
        spawn_forward_proxy_bootstrap_probe_round(
            state.clone(),
            added_manual_endpoints,
            "settings-update",
        );
    }

    let response = build_forward_proxy_settings_response(state.as_ref())
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(Json(response))
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyRefreshSubscriptionsResponse {
    forward_proxy: ForwardProxySettingsResponse,
    subscription_count: usize,
    added_node_count: usize,
    refreshed_at: String,
}

pub(crate) async fn post_forward_proxy_refresh_subscriptions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ForwardProxyRefreshSubscriptionsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }

    let before_subscription_keys = {
        let manager = state.forward_proxy.lock().await;
        snapshot_active_forward_proxy_endpoints(&manager)
            .into_iter()
            .filter(|endpoint| endpoint.source == FORWARD_PROXY_SOURCE_SUBSCRIPTION)
            .map(|endpoint| endpoint.key)
            .collect::<HashSet<_>>()
    };
    let task_run = try_begin_system_task_run_with_admission(
        state.as_ref(),
        crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy,
        SystemTaskKind::ForwardProxySubscriptionRefresh,
        "manual",
        Some("forward proxy manual refresh started".to_string()),
    )
    .await
    .ok()
    .flatten();

    if let Err(err) = refresh_forward_proxy_subscriptions(state.clone(), true, None).await {
        if let Some(run) = task_run.as_ref() {
            finish_system_task_run_batched(
                state.as_ref(),
                run,
                SystemTaskStatus::Failed,
                Some("forward proxy manual refresh failed".to_string()),
                Some(err.to_string()),
            )
            .await;
        }
        return Err((StatusCode::BAD_GATEWAY, err.to_string()));
    }

    let after_subscription_keys = {
        let manager = state.forward_proxy.lock().await;
        snapshot_active_forward_proxy_endpoints(&manager)
            .into_iter()
            .filter(|endpoint| endpoint.source == FORWARD_PROXY_SOURCE_SUBSCRIPTION)
            .map(|endpoint| endpoint.key)
            .collect::<HashSet<_>>()
    };
    let added_node_count = after_subscription_keys
        .difference(&before_subscription_keys)
        .count();
    let forward_proxy = build_forward_proxy_settings_response(state.as_ref())
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    if let Some(run) = task_run.as_ref() {
        finish_system_task_run_batched(
            state.as_ref(),
            run,
            SystemTaskStatus::Success,
            Some(format!(
                "forward proxy manual refresh completed: subscriptions={} added_nodes={}",
                forward_proxy.subscription_urls.len(),
                added_node_count
            )),
            None,
        )
        .await;
    }
    Ok(Json(ForwardProxyRefreshSubscriptionsResponse {
        subscription_count: forward_proxy.subscription_urls.len(),
        forward_proxy,
        added_node_count,
        refreshed_at: format_utc_iso(Utc::now()),
    }))
}

pub(crate) fn parse_forward_proxy_nodes_latency_test_keys(raw_query: &str) -> Vec<String> {
    url::form_urlencoded::parse(raw_query.as_bytes())
        .filter_map(|(key, value)| (key == "key").then(|| value.into_owned()))
        .collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyLatencyProbeTargetResult {
    pub(crate) ok: bool,
    pub(crate) latency_ms: Option<f64>,
    pub(crate) ip: Option<String>,
    pub(crate) http_status: Option<u16>,
    pub(crate) error: Option<String>,
}

impl ForwardProxyLatencyProbeTargetResult {
    fn success_latency(&self) -> Option<f64> {
        self.ok.then_some(self.latency_ms).flatten()
    }
}
