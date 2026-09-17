pub(crate) async fn get_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SettingsResponse>, ApiError> {
    let pricing = state.pricing_catalog.read().await.clone();
    let proxy = state.proxy_model_settings.read().await.clone();
    let forward_proxy = build_forward_proxy_settings_response(state.as_ref()).await?;
    Ok(Json(SettingsResponse {
        proxy: ProxyModelSettingsResponse::from_settings(proxy),
        forward_proxy,
        pricing: PricingSettingsResponse::from_catalog(&pricing),
    }))
}

pub(crate) async fn removed_proxy_model_settings_endpoint() -> (StatusCode, &'static str) {
    (
        StatusCode::NOT_FOUND,
        "endpoint removed; legacy reverse proxy settings are no longer supported",
    )
}

pub(crate) async fn put_proxy_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ProxyModelSettingsUpdateRequest>,
) -> Result<Json<ProxyModelSettingsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }

    let ProxyModelSettingsUpdateRequest {
        hijack_enabled,
        merge_upstream_enabled,
        fast_mode_rewrite_mode: _legacy_fast_mode_rewrite_mode,
        upstream_429_max_retries,
        websocket_enabled,
        upstream_websocket_default_enabled,
        request_body_logging_enabled,
        response_body_logging_enabled,
        encrypted_session_owner_routing_enabled,
        enabled_models,
    } = payload;

    let _update_guard = state.proxy_model_settings_update_lock.lock().await;
    let current = state.proxy_model_settings.read().await.clone();
    let next = ProxyModelSettings {
        hijack_enabled,
        merge_upstream_enabled,
        upstream_429_max_retries: upstream_429_max_retries
            .unwrap_or(current.upstream_429_max_retries),
        websocket_enabled: websocket_enabled.unwrap_or(current.websocket_enabled),
        upstream_websocket_default_enabled: upstream_websocket_default_enabled
            .unwrap_or(current.upstream_websocket_default_enabled),
        request_body_logging_enabled: request_body_logging_enabled
            .unwrap_or(current.request_body_logging_enabled),
        response_body_logging_enabled: response_body_logging_enabled
            .unwrap_or(current.response_body_logging_enabled),
        encrypted_session_owner_routing_enabled: encrypted_session_owner_routing_enabled
            .unwrap_or(current.encrypted_session_owner_routing_enabled),
        enabled_preset_models: enabled_models,
    }
    .normalized();
    save_proxy_model_settings(&state.pool, next.clone())
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let mut guard = state.proxy_model_settings.write().await;
    *guard = next.clone();
    Ok(Json(ProxyModelSettingsResponse::from_settings(next)))
}

pub(crate) async fn put_pricing_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<PricingSettingsUpdateRequest>,
) -> Result<Json<PricingSettingsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }

    let next = payload.normalized()?;
    let _update_guard = state.pricing_settings_update_lock.lock().await;
    save_pricing_catalog(&state.pool, &next)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    {
        let mut guard = state.pricing_catalog.write().await;
        *guard = next.clone();
    }
    if let Err(err) = wake_startup_backfill_tasks_with_pricing_catalog(
        &state.pool,
        &[StartupBackfillTask::ProxyCost],
        Some(&next),
        "pricing_catalog_updated",
    )
    .await
    {
        warn!(error = %err, "failed to wake ProxyCost backfill after pricing catalog update");
    }
    Ok(Json(PricingSettingsResponse::from_catalog(&next)))
}

pub(crate) async fn get_versions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<VersionResponse>, ApiError> {
    let (backend, frontend) = detect_versions(state.config.static_dir.as_deref());
    Ok(Json(VersionResponse { backend, frontend }))
}

#[derive(Debug, Default)]
pub(crate) struct BroadcastStateCache {
    pub(crate) quota: Option<QuotaSnapshotResponse>,
}

static DASHBOARD_ACTIVITY_LIVE_REVISION: AtomicU64 = AtomicU64::new(0);
pub(crate) const DASHBOARD_RUNTIME_PROJECTION_RECONCILE_INTERVAL: Duration =
    Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardActivityLiveAccount {
    pub(crate) account_key: String,
    pub(crate) upstream_account_id: Option<i64>,
    #[serde(skip)]
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) in_progress_invocation_count: i64,
    pub(crate) in_progress_phase_counts: InvocationPhaseCountsResponse,
    pub(crate) retry_invocation_count: i64,
    #[serde(skip)]
    pub(crate) in_progress_wait_sum_ms: f64,
    #[serde(skip)]
    pub(crate) in_progress_wait_sample_count: i64,
    pub(crate) upload_bytes_per_second: f64,
    pub(crate) download_bytes_per_second: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) network_live_bucket: Option<DashboardNetworkTimeseriesPointResponse>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardActivityLiveSnapshot {
    pub(crate) revision: u64,
    pub(crate) generated_at: String,
    pub(crate) in_progress_invocation_count: i64,
    pub(crate) in_progress_phase_counts: InvocationPhaseCountsResponse,
    pub(crate) retry_invocation_count: i64,
    #[serde(skip)]
    pub(crate) in_progress_wait_sum_ms: f64,
    #[serde(skip)]
    pub(crate) in_progress_wait_sample_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) network_live_bucket: Option<DashboardNetworkTimeseriesPointResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) network_realtime_rate: Option<DashboardNetworkRealtimeRateResponse>,
    pub(crate) accounts: Vec<DashboardActivityLiveAccount>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DashboardCurrentProjectionAccountSlice {
    pub(crate) account_key: String,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) in_progress_invocation_count: i64,
    pub(crate) in_progress_phase_counts: InvocationPhaseCountsResponse,
    pub(crate) retry_invocation_count: i64,
    pub(crate) in_progress_wait_sum_ms: f64,
    pub(crate) in_progress_wait_sample_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DashboardCurrentProjectionSlice {
    pub(crate) revision: u64,
    pub(crate) in_progress_invocation_count: i64,
    pub(crate) in_progress_phase_counts: InvocationPhaseCountsResponse,
    pub(crate) retry_invocation_count: i64,
    pub(crate) in_progress_wait_sum_ms: f64,
    pub(crate) in_progress_wait_sample_count: i64,
    pub(crate) accounts: Vec<DashboardCurrentProjectionAccountSlice>,
}

impl From<&DashboardActivityLiveSnapshot> for DashboardCurrentProjectionSlice {
    fn from(snapshot: &DashboardActivityLiveSnapshot) -> Self {
        Self {
            revision: snapshot.revision,
            in_progress_invocation_count: snapshot.in_progress_invocation_count,
            in_progress_phase_counts: snapshot.in_progress_phase_counts,
            retry_invocation_count: snapshot.retry_invocation_count,
            in_progress_wait_sum_ms: snapshot.in_progress_wait_sum_ms,
            in_progress_wait_sample_count: snapshot.in_progress_wait_sample_count,
            accounts: snapshot
                .accounts
                .iter()
                .map(|account| DashboardCurrentProjectionAccountSlice {
                    account_key: account.account_key.clone(),
                    upstream_account_id: account.upstream_account_id,
                    upstream_account_name: account.upstream_account_name.clone(),
                    in_progress_invocation_count: account.in_progress_invocation_count,
                    in_progress_phase_counts: account.in_progress_phase_counts,
                    retry_invocation_count: account.retry_invocation_count,
                    in_progress_wait_sum_ms: account.in_progress_wait_sum_ms,
                    in_progress_wait_sample_count: account.in_progress_wait_sample_count,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DashboardNetworkProjectionAccountSlice {
    pub(crate) account_key: String,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upload_bytes_per_second: f64,
    pub(crate) download_bytes_per_second: f64,
    pub(crate) network_live_bucket: Option<DashboardNetworkTimeseriesPointResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DashboardNetworkProjectionSlice {
    pub(crate) revision: u64,
    pub(crate) network_live_bucket: Option<DashboardNetworkTimeseriesPointResponse>,
    pub(crate) network_realtime_rate: Option<DashboardNetworkRealtimeRateResponse>,
    pub(crate) accounts: Vec<DashboardNetworkProjectionAccountSlice>,
    pub(crate) recent: DashboardRecentNetworkWindowResponse,
    #[serde(skip)]
    pub(crate) current_snapshot: DashboardActivityCurrentSnapshot,
    #[serde(skip)]
    pub(crate) current_snapshot_by_account: HashMap<Option<i64>, DashboardActivityCurrentSnapshot>,
}

impl DashboardNetworkProjectionSlice {
    pub(crate) fn from_memory(
        dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
        network_open_buckets: &HashMap<
            DashboardNetworkScopeKey,
            DashboardRuntimeNetworkOpenBucketBaseline,
        >,
        known_account_ids: &std::collections::BTreeSet<Option<i64>>,
    ) -> Self {
        let now = Utc::now();
        let account_rates = dashboard_network_speed_cache.snapshot_account_rates(now);
        let current_snapshot_by_account =
            dashboard_network_speed_cache.snapshot_dashboard_activity_accounts(now);
        let current_snapshot =
            sum_dashboard_activity_current_snapshots(current_snapshot_by_account.values().copied());
        let mut account_ids = account_rates
            .keys()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        account_ids.extend(
            network_open_buckets
                .keys()
                .filter_map(|scope| scope.upstream_account_id()),
        );
        account_ids.extend(current_snapshot_by_account.keys().copied());
        account_ids.extend(known_account_ids.iter().copied());
        let mut accounts = account_ids
            .into_iter()
            .map(|upstream_account_id| {
                let rate = account_rates
                    .get(&upstream_account_id)
                    .copied()
                    .unwrap_or_default();
                DashboardNetworkProjectionAccountSlice {
                    account_key: upstream_account_id
                        .map(|id| format!("upstream:{id}"))
                        .unwrap_or_else(|| "unassigned".to_string()),
                    upstream_account_id,
                    upload_bytes_per_second: rate.upload_bytes_per_second,
                    download_bytes_per_second: rate.download_bytes_per_second,
                    network_live_bucket: Some(dashboard_network_live_bucket_from_memory(
                        dashboard_network_speed_cache,
                        network_open_buckets,
                        DashboardNetworkScopeKey::account_scope(upstream_account_id),
                        now,
                    )),
                }
            })
            .collect::<Vec<_>>();
        accounts.sort_by(|left, right| left.account_key.cmp(&right.account_key));
        Self {
            revision: 0,
            network_live_bucket: Some(dashboard_network_live_bucket_from_memory(
                dashboard_network_speed_cache,
                network_open_buckets,
                DashboardNetworkScopeKey::Global,
                now,
            )),
            network_realtime_rate: Some(build_dashboard_network_realtime_rate_response(
                dashboard_network_speed_cache
                    .snapshot_scope_realtime_bytes(DashboardNetworkScopeKey::Global, now),
            )),
            accounts,
            recent: build_dashboard_recent_network_window_response(
                dashboard_network_speed_cache.snapshot_recent_global_window(now),
            ),
            current_snapshot,
            current_snapshot_by_account,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DashboardTerminalProjectionSlice {
    pub(crate) revision: u64,
    pub(crate) deltas: Vec<DashboardActivityTerminalDelta>,
}

pub(crate) fn current_dashboard_activity_live_revision() -> u64 {
    DASHBOARD_ACTIVITY_LIVE_REVISION.load(Ordering::Acquire)
}

pub(crate) fn reserve_dashboard_activity_live_revision() -> u64 {
    DASHBOARD_ACTIVITY_LIVE_REVISION.fetch_add(1, Ordering::AcqRel) + 1
}

pub(crate) async fn capture_dashboard_activity_live_snapshot(
    state: &AppState,
) -> Result<DashboardActivityLiveSnapshot, ApiError> {
    let pending_window = state
        .proxy_runtime_invocations
        .pending_dashboard_publish_window()
        .filter(|window| window.slice == DashboardProjectionSlice::Current)
        .and_then(|window| {
            state
                .proxy_runtime_invocations
                .begin_dashboard_publish_window(window)
        });
    let capture = capture_dashboard_activity_live_snapshot_with_outcome(state).await?;
    if let Some(window) = pending_window {
        state
            .proxy_runtime_invocations
            .complete_dashboard_publish_window(window);
    }
    let mut snapshot = capture.snapshot;
    if state.proxy_runtime_invocations.mode() == RuntimeProjectionMode::Legacy {
        snapshot = state
            .proxy_runtime_invocations
            .legacy_live_snapshot(snapshot);
    } else {
        state
            .proxy_runtime_invocations
            .apply_network_overlay_to_snapshot(&mut snapshot);
    }
    Ok(snapshot)
}

async fn capture_dashboard_activity_live_snapshot_with_outcome(
    state: &AppState,
) -> Result<DashboardProjectionCapture, ApiError> {
    state
        .proxy_runtime_invocations
        .bind_dashboard_network_speed_cache(state.dashboard_network_speed_cache.clone())?;
    capture_dashboard_activity_live_snapshot_from_runtime(
        &state.pool,
        state.proxy_runtime_invocations.as_ref(),
        state.dashboard_network_speed_cache.as_ref(),
    )
    .await
}

async fn capture_dashboard_activity_live_snapshot_from_runtime(
    pool: &Pool<Sqlite>,
    hub: &RuntimeProjectionHub,
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
) -> Result<DashboardProjectionCapture, ApiError> {
    let started_at = Instant::now();
    let capture = match hub.mode() {
        RuntimeProjectionMode::Legacy => {
            capture_dashboard_activity_live_snapshot_from_persistence(
                pool,
                hub,
                dashboard_network_speed_cache,
                true,
                "legacy",
            )
            .await?
        }
        RuntimeProjectionMode::Auto if hub.is_memory_ready() => {
            match hub.capture_memory_snapshot() {
                Ok(capture) => capture,
                Err(err) => {
                    hub.mark_degraded("memory_snapshot_failed");
                    warn!(
                        ?err,
                        "dashboard runtime projection entered degraded last-good mode"
                    );
                    if let Some(capture) = hub.last_good_capture("last_good") {
                        capture
                    } else {
                        capture_dashboard_activity_live_snapshot_from_persistence(
                            pool,
                            hub,
                            dashboard_network_speed_cache,
                            true,
                            "cold_fallback",
                        )
                        .await?
                    }
                }
            }
        }
        RuntimeProjectionMode::Auto => {
            if let Some(capture) = hub.last_good_capture("last_good") {
                capture
            } else {
                capture_dashboard_activity_live_snapshot_from_persistence(
                    pool,
                    hub,
                    dashboard_network_speed_cache,
                    true,
                    "cold_fallback",
                )
                .await?
            }
        }
    };
    let health = hub.health_snapshot(0);
    tracing::debug!(
        projection = "dashboard_current",
        trigger = "capture",
        revision = capture.snapshot.revision,
        render_elapsed_ms = started_at.elapsed().as_millis() as u64,
        live_path_db_read_count = health.live_path_db_read_count,
        snapshot_origin = capture.snapshot_origin,
        last_good_age_ms = health.last_good_age_ms,
        changed = capture.changed,
        "captured dashboard runtime projection"
    );
    Ok(capture)
}

async fn capture_dashboard_activity_live_snapshot_from_persistence(
    pool: &Pool<Sqlite>,
    hub: &RuntimeProjectionHub,
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
    count_live_path_read: bool,
    snapshot_origin: &'static str,
) -> Result<DashboardProjectionCapture, ApiError> {
    let expected_generation = hub.dashboard_generation();
    if count_live_path_read {
        hub.record_live_path_db_read();
    }
    hub.record_build();
    let (snapshot, baseline) = query_dashboard_activity_live_snapshot_with_baseline_from_runtime(
        pool,
        hub,
        dashboard_network_speed_cache,
        0,
    )
    .await?;
    let expected_generation = if hub.mode() == RuntimeProjectionMode::Legacy {
        hub.dashboard_generation()
    } else {
        expected_generation
    };
    if let Some(capture) = hub.install_persistence_baseline_if_generation(
        snapshot,
        baseline,
        snapshot_origin,
        expected_generation,
    )? {
        return Ok(capture);
    }
    Ok(hub.capture_memory_snapshot()?)
}

pub(crate) async fn warm_dashboard_runtime_projection(state: &AppState) {
    if state.proxy_runtime_invocations.mode() == RuntimeProjectionMode::Legacy {
        return;
    }
    if let Err(err) = capture_dashboard_activity_live_snapshot_from_persistence(
        &state.pool,
        state.proxy_runtime_invocations.as_ref(),
        state.dashboard_network_speed_cache.as_ref(),
        false,
        "startup_restore",
    )
    .await
    {
        state
            .proxy_runtime_invocations
            .mark_degraded("startup_restore_failed");
        warn!(
            ?err,
            "failed to warm dashboard runtime projection from persistence"
        );
    }
}

pub(crate) async fn reconcile_dashboard_runtime_projection_once(
    state: &AppState,
) -> Result<DashboardProjectionCapture, ApiError> {
    capture_dashboard_activity_live_snapshot_from_persistence(
        &state.pool,
        state.proxy_runtime_invocations.as_ref(),
        state.dashboard_network_speed_cache.as_ref(),
        false,
        "reconcile",
    )
    .await
}

pub(crate) fn spawn_dashboard_runtime_projection_reconcile(state: Arc<AppState>) {
    if state.proxy_runtime_invocations.mode() == RuntimeProjectionMode::Legacy {
        return;
    }
    tokio::spawn(async move {
        let mut cadence = tokio::time::interval(DASHBOARD_RUNTIME_PROJECTION_RECONCILE_INTERVAL);
        cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        cadence.tick().await;
        loop {
            tokio::select! {
                _ = state.shutdown.cancelled() => return,
                _ = cadence.tick() => {}
            }
            let pressure_gate = crate::db_pressure::global_db_pressure_gate();
            let _pressure_permit = match pressure_gate
                .try_begin_background("dashboard_runtime_projection_reconcile")
            {
                Ok(permit) => permit,
                Err(reason) => {
                    let reason = match reason {
                        crate::db_pressure::DbPressureDenyReason::PressureCooldown { .. } => {
                            "writer_pressure"
                        }
                        crate::db_pressure::DbPressureDenyReason::BackgroundBusy => {
                            "background_busy"
                        }
                    };
                    state
                        .proxy_runtime_invocations
                        .record_reconcile_deferred(reason);
                    tracing::debug!(
                        projection = "dashboard_current",
                        defer_reason = reason,
                        "deferred dashboard runtime projection reconcile"
                    );
                    continue;
                }
            };
            match reconcile_dashboard_runtime_projection_once(state.as_ref()).await {
                Ok(capture) => {
                    state
                        .subscription_hub
                        .reconcile_dashboard_terminal_window_bases(state.clone())
                        .await;
                    tracing::debug!(
                        projection = "dashboard_current",
                        revision = capture.snapshot.revision,
                        changed = capture.changed,
                        snapshot_origin = capture.snapshot_origin,
                        "reconciled dashboard runtime projection baseline"
                    );
                    if capture.changed
                        && state
                            .subscription_hub
                            .has_active_dashboard_activity_live_topic()
                            .await
                    {
                        let _ = state
                            .broadcaster
                            .send(BroadcastPayload::DashboardCurrentSlice {
                                slice: Box::new(DashboardCurrentProjectionSlice::from(
                                    &capture.snapshot,
                                )),
                            });
                    }
                }
                Err(err) => {
                    let pressure_error = match &err {
                        ApiError::BadRequest(err)
                        | ApiError::Unavailable(err)
                        | ApiError::Internal(err) => pressure_gate
                            .record_error("dashboard_runtime_projection_reconcile", err),
                    };
                    if pressure_error {
                        state
                            .proxy_runtime_invocations
                            .record_reconcile_deferred("writer_pressure");
                    } else {
                        state
                            .proxy_runtime_invocations
                            .record_reconcile_failure("reconcile_failed");
                    }
                    warn!(
                        ?err,
                        pressure_error, "failed to reconcile dashboard runtime projection baseline"
                    );
                }
            }
        }
    });
}

#[derive(Debug, Clone)]
struct DashboardProjectionInvocation {
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<String>,
    is_retry: bool,
    live_phase: Option<String>,
    wait_ms: Option<f64>,
}

fn build_dashboard_activity_live_snapshot_from_projection_records(
    revision: u64,
    records: impl IntoIterator<Item = DashboardProjectionInvocation>,
) -> DashboardActivityLiveSnapshot {
    let mut accounts = HashMap::<Option<i64>, DashboardActivityLiveAccount>::new();
    for record in records {
        let account_id = record.upstream_account_id;
        let account = accounts
            .entry(account_id)
            .or_insert_with(|| DashboardActivityLiveAccount {
                account_key: account_id
                    .map(|id| format!("upstream:{id}"))
                    .unwrap_or_else(|| "unassigned".to_string()),
                upstream_account_id: account_id,
                upstream_account_name: normalize_trimmed_optional_string_local(
                    record.upstream_account_name.clone(),
                ),
                in_progress_invocation_count: 0,
                in_progress_phase_counts: InvocationPhaseCountsResponse::default(),
                retry_invocation_count: 0,
                in_progress_wait_sum_ms: 0.0,
                in_progress_wait_sample_count: 0,
                upload_bytes_per_second: 0.0,
                download_bytes_per_second: 0.0,
                network_live_bucket: None,
            });
        if account.upstream_account_name.is_none() {
            account.upstream_account_name =
                normalize_trimmed_optional_string_local(record.upstream_account_name.clone());
        }
        account.in_progress_invocation_count += 1;
        account
            .in_progress_phase_counts
            .increment_phase_name(record.live_phase.as_deref());
        if record.is_retry {
            account.retry_invocation_count += 1;
        }
        if let Some(wait_ms) = normalized_wait_ms(record.wait_ms) {
            account.in_progress_wait_sum_ms += wait_ms;
            account.in_progress_wait_sample_count += 1;
        }
    }
    let mut accounts = accounts.into_values().collect::<Vec<_>>();
    accounts.sort_by(|left, right| left.account_key.cmp(&right.account_key));
    let mut phase_counts = InvocationPhaseCountsResponse::default();
    let mut in_progress_invocation_count = 0;
    let mut retry_invocation_count = 0;
    let mut in_progress_wait_sum_ms = 0.0;
    let mut in_progress_wait_sample_count = 0;
    for account in &accounts {
        in_progress_invocation_count += account.in_progress_invocation_count;
        retry_invocation_count += account.retry_invocation_count;
        in_progress_wait_sum_ms += account.in_progress_wait_sum_ms;
        in_progress_wait_sample_count += account.in_progress_wait_sample_count;
        phase_counts.queued += account.in_progress_phase_counts.queued;
        phase_counts.requesting += account.in_progress_phase_counts.requesting;
        phase_counts.responding += account.in_progress_phase_counts.responding;
    }
    DashboardActivityLiveSnapshot {
        revision,
        generated_at: format_utc_iso(Utc::now()),
        in_progress_invocation_count,
        in_progress_phase_counts: phase_counts,
        retry_invocation_count,
        in_progress_wait_sum_ms,
        in_progress_wait_sample_count,
        network_live_bucket: None,
        network_realtime_rate: None,
        accounts,
    }
}

pub(crate) fn build_dashboard_activity_live_snapshot(
    revision: u64,
    records: impl IntoIterator<Item = ApiInvocation>,
) -> DashboardActivityLiveSnapshot {
    build_dashboard_activity_live_snapshot_from_projection_records(
        revision,
        records.into_iter().filter_map(|record| {
            matches!(
                normalized_runtime_text(record.status.as_deref()).as_str(),
                "running" | "pending"
            )
            .then(|| {
                let live_phase = runtime_record_live_phase(&record).map(str::to_string);
                DashboardProjectionInvocation {
                    upstream_account_id: record.upstream_account_id,
                    upstream_account_name: record.upstream_account_name,
                    is_retry: record.pool_attempt_count.unwrap_or_default() > 1,
                    live_phase,
                    wait_ms: record.t_upstream_ttfb_ms,
                }
            })
        }),
    )
}

pub(crate) fn build_dashboard_activity_live_snapshot_from_memory(
    revision: u64,
    baseline: Option<DashboardRuntimeProjectionBaseline>,
    runtime_records: impl IntoIterator<Item = ApiInvocation>,
    terminal_tombstones: HashSet<RuntimeInvocationKey>,
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
) -> DashboardActivityLiveSnapshot {
    let (source_scope, baseline_records, network_open_buckets) = baseline.map_or_else(
        || (InvocationSourceScope::All, Vec::new(), HashMap::new()),
        |baseline| {
            (
                baseline.source_scope,
                baseline.records,
                baseline.network_open_buckets,
            )
        },
    );
    let mut projection_records = baseline_records
        .into_iter()
        .map(|record| {
            (
                record.key,
                DashboardProjectionInvocation {
                    upstream_account_id: record.upstream_account_id,
                    upstream_account_name: record.upstream_account_name,
                    is_retry: record.is_retry,
                    live_phase: record.live_phase,
                    wait_ms: record.wait_ms,
                },
            )
        })
        .collect::<HashMap<_, _>>();
    for key in &terminal_tombstones {
        projection_records.remove(key);
    }
    for record in runtime_records {
        let key = RuntimeInvocationKey::new(record.invoke_id.clone(), record.occurred_at.clone());
        if terminal_tombstones.contains(&key) {
            projection_records.remove(&key);
            continue;
        }
        if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
            continue;
        }
        if !matches!(
            normalized_runtime_text(record.status.as_deref()).as_str(),
            "running" | "pending"
        ) {
            projection_records.remove(&key);
            continue;
        }
        let baseline_record = projection_records.get(&key);
        let upstream_account_id = record
            .upstream_account_id
            .or_else(|| baseline_record.and_then(|record| record.upstream_account_id));
        let upstream_account_name = normalize_trimmed_optional_string_local(
            record.upstream_account_name.clone(),
        )
        .or_else(|| baseline_record.and_then(|record| record.upstream_account_name.clone()));
        let is_retry = record.pool_attempt_count.unwrap_or_default() > 1
            || baseline_record.is_some_and(|record| record.is_retry);
        let live_phase =
            runtime_record_live_phase_with_retry(&record, is_retry).map(str::to_string);
        projection_records.insert(
            key,
            DashboardProjectionInvocation {
                upstream_account_id,
                upstream_account_name,
                is_retry,
                live_phase,
                wait_ms: record.t_upstream_ttfb_ms,
            },
        );
    }
    let snapshot = build_dashboard_activity_live_snapshot_from_projection_records(
        revision,
        projection_records.into_values(),
    );
    overlay_dashboard_network_live_snapshot(
        snapshot,
        &network_open_buckets,
        dashboard_network_speed_cache,
    )
}
