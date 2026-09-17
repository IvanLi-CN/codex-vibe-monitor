use super::*;

pub(crate) async fn refresh_forward_proxy_subscriptions(
    state: Arc<AppState>,
    force: bool,
    known_subscription_keys_override: Option<HashSet<String>>,
) -> Result<()> {
    let (subscription_urls, interval_secs, last_refresh_at) = {
        let manager = state.forward_proxy.lock().await;
        (
            manager.settings.subscription_urls.clone(),
            manager.settings.subscription_update_interval_secs,
            manager.last_subscription_refresh_at,
        )
    };

    if !force
        && let Some(last_refresh_at) = last_refresh_at
        && (Utc::now() - last_refresh_at).num_seconds()
            < i64::try_from(interval_secs).unwrap_or(i64::MAX)
    {
        return Ok(());
    }

    let subscription_proxy_urls =
        fetch_forward_proxy_subscription_urls(&state, &subscription_urls).await?;
    let added_subscription_endpoints = apply_forward_proxy_subscription_urls(
        &state,
        subscription_urls,
        subscription_proxy_urls,
        known_subscription_keys_override,
    )
    .await?;
    sync_forward_proxy_routes(state.as_ref()).await?;
    if !added_subscription_endpoints.is_empty() {
        spawn_forward_proxy_bootstrap_probe_round(
            state.clone(),
            added_subscription_endpoints,
            "subscription-refresh",
        );
    }
    Ok(())
}

async fn fetch_forward_proxy_subscription_urls(
    state: &AppState,
    subscription_urls: &[String],
) -> Result<Vec<String>> {
    let mut urls = Vec::new();
    let mut fetched_any = false;
    for subscription_url in subscription_urls {
        if state.shutdown.is_cancelled() {
            info!("stopping forward proxy subscription refresh because shutdown is in progress");
            return Ok(urls);
        }
        let result = tokio::select! {
            _ = state.shutdown.cancelled() => return Ok(urls),
            result = fetch_subscription_proxy_urls(
                &state.http_clients.shared,
                subscription_url,
                state.config.request_timeout,
            ) => result,
        };
        match result {
            Ok(entries) => {
                fetched_any = true;
                urls.extend(entries);
            }
            Err(err) => {
                warn!(subscription_url, error = %err, "failed to fetch forward proxy subscription")
            }
        }
    }
    if !subscription_urls.is_empty() && !fetched_any {
        bail!("all forward proxy subscriptions failed to refresh");
    }
    Ok(urls)
}

async fn apply_forward_proxy_subscription_urls(
    state: &AppState,
    subscription_urls: Vec<String>,
    proxy_urls: Vec<String>,
    known_subscription_keys_override: Option<HashSet<String>>,
) -> Result<Vec<ForwardProxyEndpoint>> {
    if state.shutdown.is_cancelled() {
        return Ok(Vec::new());
    }
    let _refresh_guard = state.forward_proxy_subscription_refresh_lock.lock().await;
    let mut manager = state.forward_proxy.lock().await;
    if state.shutdown.is_cancelled() {
        return Ok(Vec::new());
    }
    if manager.settings.subscription_urls != subscription_urls {
        debug!("skip stale forward proxy subscription refresh after settings changed");
        return Ok(Vec::new());
    }
    let mut known_keys = snapshot_active_forward_proxy_endpoints(&manager)
        .into_iter()
        .filter(|endpoint| endpoint.source == FORWARD_PROXY_SOURCE_SUBSCRIPTION)
        .map(|endpoint| endpoint.key)
        .collect::<HashSet<_>>();
    if let Some(override_keys) = known_subscription_keys_override {
        known_keys.extend(override_keys);
    }
    manager.apply_subscription_urls(proxy_urls);
    Ok(snapshot_active_forward_proxy_endpoints(&manager)
        .into_iter()
        .filter(|endpoint| endpoint.source == FORWARD_PROXY_SOURCE_SUBSCRIPTION)
        .filter(|endpoint| !known_keys.contains(&endpoint.key))
        .collect())
}

pub(crate) async fn sync_forward_proxy_routes(state: &AppState) -> Result<()> {
    let runtime_snapshot = {
        let mut manager = state.forward_proxy.lock().await;
        let mut xray_supervisor = state.xray_supervisor.lock().await;
        xray_supervisor
            .sync_endpoints(&mut manager.endpoints, &state.shutdown)
            .await?;
        manager.ensure_non_zero_weight();
        manager.snapshot_runtime()
    };
    persist_forward_proxy_runtime_snapshot(state, runtime_snapshot).await
}

pub(crate) async fn persist_forward_proxy_runtime_snapshot(
    state: &AppState,
    runtime_snapshot: Vec<ForwardProxyRuntimeState>,
) -> Result<()> {
    let active_keys = runtime_snapshot
        .iter()
        .map(|entry| entry.proxy_key.clone())
        .collect::<Vec<_>>();
    delete_forward_proxy_runtime_rows_not_in(&state.pool, &active_keys).await?;
    for runtime in &runtime_snapshot {
        persist_forward_proxy_runtime_state(&state.pool, runtime).await?;
    }
    Ok(())
}

pub(crate) async fn fetch_subscription_proxy_urls(
    client: &Client,
    subscription_url: &str,
    request_timeout: Duration,
) -> Result<Vec<String>> {
    let response = timeout(request_timeout, client.get(subscription_url).send())
        .await
        .map_err(|_| anyhow!("subscription request timed out"))?
        .with_context(|| format!("failed to request subscription url: {subscription_url}"))?;
    if !response.status().is_success() {
        bail!(
            "subscription url returned status {}: {subscription_url}",
            response.status()
        );
    }
    let body = timeout(request_timeout, response.text())
        .await
        .map_err(|_| anyhow!("subscription body read timed out"))?
        .context("failed to read subscription body")?;
    Ok(parse_proxy_urls_from_subscription_body(&body))
}

pub(crate) async fn fetch_subscription_proxy_urls_with_validation_budget(
    client: &Client,
    subscription_url: &str,
    total_timeout: Duration,
    started: Instant,
) -> Result<Vec<String>> {
    let request_timeout = remaining_timeout_budget(total_timeout, started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| timeout_error_for_duration(total_timeout))?;
    let response = timeout(request_timeout, client.get(subscription_url).send())
        .await
        .map_err(|_| timeout_error_for_duration(total_timeout))?
        .with_context(|| format!("failed to request subscription url: {subscription_url}"))?;
    if !response.status().is_success() {
        bail!(
            "subscription url returned status {}: {subscription_url}",
            response.status()
        );
    }
    let read_timeout = remaining_timeout_budget(total_timeout, started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| timeout_error_for_duration(total_timeout))?;
    let body = timeout(read_timeout, response.text())
        .await
        .map_err(|_| timeout_error_for_duration(total_timeout))?
        .context("failed to read subscription body")?;
    Ok(parse_proxy_urls_from_subscription_body(&body))
}

pub(crate) fn parse_proxy_urls_from_subscription_body(raw: &str) -> Vec<String> {
    let decoded = decode_subscription_payload(raw);
    normalize_proxy_url_entries(vec![decoded])
}

pub(crate) fn decode_subscription_payload(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.contains("://")
        || trimmed
            .lines()
            .filter(|line| !line.trim().is_empty())
            .any(|line| line.contains("://"))
    {
        return trimmed.to_string();
    }

    let compact = trimmed
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>();
    for engine in [
        base64::engine::general_purpose::STANDARD,
        base64::engine::general_purpose::STANDARD_NO_PAD,
        base64::engine::general_purpose::URL_SAFE,
        base64::engine::general_purpose::URL_SAFE_NO_PAD,
    ] {
        if let Ok(decoded) = engine.decode(compact.as_bytes())
            && let Ok(text) = String::from_utf8(decoded)
            && text.contains("://")
        {
            return text;
        }
    }
    trimmed.to_string()
}

pub(crate) async fn select_forward_proxy_for_request(
    state: &AppState,
) -> Result<SelectedForwardProxy> {
    let mut manager = state.forward_proxy.lock().await;
    manager.select_proxy_for_scope(&ForwardProxyRouteScope::Automatic)
}

pub(crate) async fn canonicalize_forward_proxy_bound_keys(
    state: &AppState,
    bound_proxy_keys: &[String],
) -> Result<Vec<String>> {
    let normalized = bound_proxy_keys
        .iter()
        .map(|key| key.trim())
        .filter(|key| !key.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if normalized.is_empty() {
        return Ok(Vec::new());
    }
    let metadata_map = load_forward_proxy_metadata_history(&state.pool, &normalized).await?;
    let manager = state.forward_proxy.lock().await;
    let mut seen = HashSet::new();
    let mut canonical = Vec::new();
    for key in normalized {
        let next = manager
            .canonicalize_bound_proxy_key(&key, metadata_map.get(&key))
            .unwrap_or(key);
        if seen.insert(next.clone()) {
            canonical.push(next);
        }
    }
    Ok(canonical)
}

pub(crate) async fn canonicalize_forward_proxy_route_scope(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
) -> Result<ForwardProxyRouteScope> {
    match scope {
        ForwardProxyRouteScope::Automatic => Ok(ForwardProxyRouteScope::Automatic),
        ForwardProxyRouteScope::PinnedProxyKey(proxy_key) => {
            Ok(ForwardProxyRouteScope::PinnedProxyKey(proxy_key.clone()))
        }
        ForwardProxyRouteScope::BoundGroup {
            group_name,
            bound_proxy_keys,
        } => Ok(ForwardProxyRouteScope::BoundGroup {
            group_name: group_name.clone(),
            bound_proxy_keys: canonicalize_forward_proxy_bound_keys(state, bound_proxy_keys)
                .await?,
        }),
        ForwardProxyRouteScope::BoundProxyKeys {
            scope_key,
            bound_proxy_keys,
        } => Ok(ForwardProxyRouteScope::BoundProxyKeys {
            scope_key: scope_key.clone(),
            bound_proxy_keys: canonicalize_forward_proxy_bound_keys(state, bound_proxy_keys)
                .await?,
        }),
    }
}

pub(crate) async fn select_forward_proxy_for_scope(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
) -> Result<SelectedForwardProxy> {
    let canonical_scope = canonicalize_forward_proxy_route_scope(state, scope).await?;
    let mut manager = state.forward_proxy.lock().await;
    manager.select_proxy_for_scope(&canonical_scope)
}

pub(crate) async fn record_forward_proxy_scope_result(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
    selected_proxy_key: &str,
    result: ForwardProxyRouteResultKind,
) {
    let canonical_scope = canonicalize_forward_proxy_route_scope(state, scope)
        .await
        .unwrap_or_else(|_| scope.clone());
    let mut manager = state.forward_proxy.lock().await;
    manager.record_scope_result(&canonical_scope, selected_proxy_key, result);
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ForwardProxyAttemptUpdate {
    pub(crate) weight_before: Option<f64>,
    pub(crate) weight_after: Option<f64>,
    pub(crate) weight_delta: Option<f64>,
}

impl ForwardProxyAttemptUpdate {
    pub(crate) fn delta(self) -> Option<f64> {
        self.weight_delta.or_else(|| {
            let (Some(before), Some(after)) = (self.weight_before, self.weight_after) else {
                return None;
            };
            if before.is_finite() && after.is_finite() {
                Some(after - before)
            } else {
                None
            }
        })
    }
}

pub(crate) async fn record_forward_proxy_attempt(
    state: Arc<AppState>,
    selected_proxy: SelectedForwardProxy,
    success: bool,
    latency_ms: Option<f64>,
    failure_kind: Option<&str>,
    is_probe: bool,
) -> ForwardProxyAttemptUpdate {
    let (updated_runtime, probe_candidate, attempt_update) = update_forward_proxy_attempt_state(
        &state,
        &selected_proxy,
        success,
        latency_ms,
        failure_kind,
        is_probe,
    )
    .await;
    persist_forward_proxy_attempt(
        state.as_ref(),
        &selected_proxy,
        success,
        latency_ms,
        failure_kind,
        is_probe,
    )
    .await;
    persist_forward_proxy_runtime_update(state.as_ref(), updated_runtime).await;

    if let Some(candidate) = probe_candidate {
        spawn_penalized_forward_proxy_probe(state, candidate);
    }

    attempt_update
}

async fn update_forward_proxy_attempt_state(
    state: &AppState,
    selected_proxy: &SelectedForwardProxy,
    success: bool,
    latency_ms: Option<f64>,
    failure_kind: Option<&str>,
    is_probe: bool,
) -> (
    Option<ForwardProxyRuntimeState>,
    Option<SelectedForwardProxy>,
    ForwardProxyAttemptUpdate,
) {
    let mut manager = state.forward_proxy.lock().await;
    let runtime_active = manager
        .endpoints
        .iter()
        .any(|endpoint| endpoint.key == selected_proxy.key);
    let weight_before = runtime_active
        .then(|| {
            manager
                .runtime
                .get(&selected_proxy.key)
                .map(|runtime| runtime.weight)
        })
        .flatten();
    manager.record_attempt(&selected_proxy.key, success, latency_ms, is_probe);
    let updated_runtime = runtime_active
        .then(|| manager.runtime.get(&selected_proxy.key).cloned())
        .flatten();
    let weight_after = updated_runtime.as_ref().map(|runtime| runtime.weight);
    let weight_delta = match (weight_before, weight_after) {
        (Some(before), Some(after)) if before.is_finite() && after.is_finite() => {
            Some(after - before)
        }
        _ => None,
    };
    let probe_candidate = (!is_probe
        && failure_kind != Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429))
    .then(|| manager.mark_probe_started())
    .flatten();
    (
        updated_runtime,
        probe_candidate,
        ForwardProxyAttemptUpdate {
            weight_before,
            weight_after,
            weight_delta,
        },
    )
}

async fn persist_forward_proxy_attempt(
    state: &AppState,
    selected_proxy: &SelectedForwardProxy,
    success: bool,
    latency_ms: Option<f64>,
    failure_kind: Option<&str>,
    is_probe: bool,
) {
    if let Err(err) = insert_forward_proxy_attempt(
        &state.pool,
        &selected_proxy.key,
        success,
        latency_ms,
        failure_kind,
        is_probe,
    )
    .await
    {
        warn!(proxy_key_ref = %forward_proxy_log_ref(&selected_proxy.key), error = %err, "failed to persist forward proxy attempt");
    }
}

async fn persist_forward_proxy_runtime_update(
    state: &AppState,
    runtime: Option<ForwardProxyRuntimeState>,
) {
    let Some(runtime) = runtime else { return };
    let sample_epoch_us = Utc::now().timestamp_micros();
    let bucket_start_epoch = align_bucket_epoch(sample_epoch_us.div_euclid(1_000_000), 3600, 0);
    if let Err(err) = persist_forward_proxy_runtime_state(&state.pool, &runtime).await {
        warn!(proxy_key_ref = %forward_proxy_log_ref(&runtime.proxy_key), error = %err, "failed to persist forward proxy runtime state");
    }
    if let Err(err) = upsert_forward_proxy_weight_hourly_bucket(
        &state.pool,
        &runtime.proxy_key,
        bucket_start_epoch,
        runtime.weight,
        sample_epoch_us,
    )
    .await
    {
        warn!(proxy_key_ref = %forward_proxy_log_ref(&runtime.proxy_key), error = %err, "failed to persist forward proxy weight bucket");
    }
}

pub(crate) fn spawn_penalized_forward_proxy_probe(
    state: Arc<AppState>,
    candidate: SelectedForwardProxy,
) {
    tokio::spawn(async move {
        let shutdown = state.shutdown.clone();
        if shutdown.is_cancelled() {
            info!(
                proxy_key_ref = %forward_proxy_log_ref(&candidate.key),
                "skipping penalized forward proxy probe because shutdown is in progress"
            );
            let mut manager = state.forward_proxy.lock().await;
            manager.mark_probe_finished();
            return;
        }

        let probe_result = async {
            let target = state
                .config
                .openai_upstream_base_url
                .join("v1/models")
                .context("failed to build probe target url")?;
            let client = state
                .http_clients
                .client_for_forward_proxy(candidate.endpoint_url.as_ref())?;
            let started = Instant::now();
            let response = tokio::select! {
                _ = shutdown.cancelled() => {
                    info!(
                        proxy_key_ref = %forward_proxy_log_ref(&candidate.key),
                        "stopping penalized forward proxy probe because shutdown is in progress"
                    );
                    return Ok::<(), anyhow::Error>(());
                }
                response = timeout(
                    state.config.openai_proxy_handshake_timeout,
                    client.get(target).send(),
                ) => {
                    response
                        .map_err(|_| anyhow!("probe timed out"))?
                        .context("probe request failed")?
                }
            };
            let status = response.status();
            if shutdown.is_cancelled() {
                info!(
                    proxy_key_ref = %forward_proxy_log_ref(&candidate.key),
                    "skipping penalized forward proxy probe recording because shutdown is in progress"
                );
                return Ok::<(), anyhow::Error>(());
            }
            // Treat 429 as a probe failure so we don't "recover" a still-rate-limited proxy.
            let success = is_validation_probe_reachable_status(status);
            let latency_ms = Some(elapsed_ms(started));
            record_forward_proxy_attempt(
                state.clone(),
                candidate.clone(),
                success,
                latency_ms,
                if success {
                    None
                } else if status == StatusCode::TOO_MANY_REQUESTS {
                    Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
                } else if status.is_server_error() {
                    Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
                } else {
                    Some(FORWARD_PROXY_FAILURE_SEND_ERROR)
                },
                true,
            )
            .await;
            Ok::<(), anyhow::Error>(())
        }
        .await;

        if let Err(err) = probe_result {
            warn!(
                proxy_key_ref = %forward_proxy_log_ref(&candidate.key),
                proxy_source = candidate.source,
                proxy_label = candidate.display_name,
                proxy_url_ref = %forward_proxy_log_ref_option(candidate.endpoint_url_raw.as_deref()),
                error = %err,
                "penalized forward proxy probe failed"
            );
        }

        let mut manager = state.forward_proxy.lock().await;
        manager.mark_probe_finished();
    });
}

pub(crate) async fn fetch_upstream_models_payload(
    state: Arc<AppState>,
    target_url: Url,
    headers: &HeaderMap,
    upstream_429_max_retries: u8,
) -> Result<Value> {
    let handshake_timeout = state.config.openai_proxy_handshake_timeout;
    let upstream_response = match send_forward_proxy_request_with_429_retry(
        state.clone(),
        Method::GET,
        target_url,
        headers,
        None,
        None,
        handshake_timeout,
        None,
        upstream_429_max_retries,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => {
            record_forward_proxy_attempt(
                state.clone(),
                err.selected_proxy,
                false,
                Some(err.connect_latency_ms),
                Some(err.attempt_failure_kind),
                false,
            )
            .await;
            return Err(anyhow!(err.message));
        }
    };

    let selected_proxy = upstream_response.selected_proxy;
    let latency_ms = Some(upstream_response.connect_latency_ms);
    let attempt_already_recorded = upstream_response.attempt_recorded;
    let upstream_response = upstream_response.response;

    if upstream_response.status() == StatusCode::TOO_MANY_REQUESTS {
        if !attempt_already_recorded {
            record_forward_proxy_attempt(
                state.clone(),
                selected_proxy,
                false,
                latency_ms,
                Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429),
                false,
            )
            .await;
        }
        bail!("upstream /v1/models returned status 429");
    }

    if upstream_response.status().is_server_error() {
        record_forward_proxy_attempt(
            state.clone(),
            selected_proxy,
            false,
            latency_ms,
            Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX),
            false,
        )
        .await;
        bail!(
            "upstream /v1/models returned status {}",
            upstream_response.status()
        );
    }

    let payload_bytes = timeout(handshake_timeout, upstream_response.into_bytes())
        .await
        .map_err(|_| {
            anyhow!(
                "{PROXY_UPSTREAM_HANDSHAKE_TIMEOUT} after {}ms while decoding upstream /v1/models response",
                handshake_timeout.as_millis()
            )
        })?
        .map_err(anyhow::Error::msg)
        .context("failed to read upstream /v1/models response body")?;
    let payload: Value = serde_json::from_slice(&payload_bytes)
        .context("failed to decode upstream /v1/models response as JSON")?;

    payload
        .get("data")
        .and_then(|value| value.as_array())
        .ok_or_else(|| anyhow!("upstream /v1/models payload missing data array"))?;

    record_forward_proxy_attempt(state, selected_proxy, true, latency_ms, None, false).await;
    Ok(payload)
}

pub(crate) fn detect_versions(static_dir: Option<&Path>) -> (String, String) {
    let backend_base = option_env!("APP_EFFECTIVE_VERSION")
        .map(|s| s.to_string())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    let backend = if cfg!(debug_assertions) {
        format!("{}-dev", backend_base)
    } else {
        backend_base
    };

    // Try to get frontend version from a version.json written during build
    let frontend = static_dir
        .and_then(|p| {
            let path = p.join("version.json");
            fs::File::open(&path).ok().and_then(|mut f| {
                let mut s = String::new();
                if f.read_to_string(&mut s).is_ok() {
                    serde_json::from_str::<serde_json::Value>(&s)
                        .ok()
                        .and_then(|v| {
                            v.get("version")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                        })
                } else {
                    None
                }
            })
        })
        .or_else(|| {
            // Fallback to reading the web/package.json in dev setups
            let path = Path::new("web").join("package.json");
            fs::File::open(&path).ok().and_then(|mut f| {
                let mut s = String::new();
                if f.read_to_string(&mut s).is_ok() {
                    serde_json::from_str::<serde_json::Value>(&s)
                        .ok()
                        .and_then(|v| {
                            v.get("version")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                        })
                } else {
                    None
                }
            })
        })
        .unwrap_or_else(|| "unknown".to_string());

    let frontend = if cfg!(debug_assertions) {
        format!("{}-dev", frontend)
    } else {
        frontend
    };

    (backend, frontend)
}

pub(crate) fn ensure_db_directory(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("failed to create database directory: {}", parent.display())
        })?;
    }
    Ok(())
}

pub(crate) fn build_sqlite_connect_options(
    database_url: &str,
    busy_timeout: Duration,
) -> Result<SqliteConnectOptions> {
    let options = SqliteConnectOptions::from_str(database_url)
        .context("invalid sqlite database url")?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(busy_timeout);
    Ok(options)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxySettings {
    #[serde(default)]
    pub(crate) proxy_urls: Vec<String>,
    #[serde(default)]
    pub(crate) subscription_urls: Vec<String>,
    #[serde(default = "default_forward_proxy_subscription_interval_secs")]
    pub(crate) subscription_update_interval_secs: u64,
    #[serde(default = "default_forward_proxy_insert_direct_compat")]
    pub(crate) insert_direct: bool,
}

impl Default for ForwardProxySettings {
    fn default() -> Self {
        Self {
            proxy_urls: Vec::new(),
            subscription_urls: Vec::new(),
            subscription_update_interval_secs: default_forward_proxy_subscription_interval_secs(),
            insert_direct: default_forward_proxy_insert_direct_compat(),
        }
    }
}

impl ForwardProxySettings {
    pub(crate) fn normalized(self) -> Self {
        Self {
            proxy_urls: normalize_proxy_url_entries(self.proxy_urls),
            subscription_urls: normalize_subscription_entries(self.subscription_urls),
            subscription_update_interval_secs: self
                .subscription_update_interval_secs
                .clamp(60, 7 * 24 * 60 * 60),
            insert_direct: self.insert_direct,
        }
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct ForwardProxySettingsRow {
    pub(crate) proxy_urls_json: Option<String>,
    pub(crate) subscription_urls_json: Option<String>,
    pub(crate) subscription_update_interval_secs: Option<i64>,
}

impl From<ForwardProxySettingsRow> for ForwardProxySettings {
    fn from(value: ForwardProxySettingsRow) -> Self {
        let proxy_urls = decode_string_vec_json(value.proxy_urls_json.as_deref());
        let subscription_urls = decode_string_vec_json(value.subscription_urls_json.as_deref());
        let interval = value
            .subscription_update_interval_secs
            .and_then(|v| u64::try_from(v).ok())
            .unwrap_or_else(default_forward_proxy_subscription_interval_secs);
        ForwardProxySettings {
            proxy_urls,
            subscription_urls,
            subscription_update_interval_secs: interval,
            insert_direct: default_forward_proxy_insert_direct_compat(),
        }
        .normalized()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxySettingsUpdateRequest {
    #[serde(default)]
    pub(crate) proxy_urls: Vec<String>,
    #[serde(default)]
    pub(crate) subscription_urls: Vec<String>,
    #[serde(default = "default_forward_proxy_subscription_interval_secs")]
    pub(crate) subscription_update_interval_secs: u64,
    #[serde(default = "default_forward_proxy_insert_direct_compat")]
    pub(crate) insert_direct: bool,
}

impl From<ForwardProxySettingsUpdateRequest> for ForwardProxySettings {
    fn from(value: ForwardProxySettingsUpdateRequest) -> Self {
        ForwardProxySettings {
            proxy_urls: value.proxy_urls,
            subscription_urls: value.subscription_urls,
            subscription_update_interval_secs: value.subscription_update_interval_secs,
            insert_direct: value.insert_direct,
        }
        .normalized()
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ForwardProxyValidationKind {
    ProxyUrl,
    SubscriptionUrl,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyCandidateValidationRequest {
    pub(crate) kind: ForwardProxyValidationKind,
    pub(crate) value: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyCandidateValidationResponse {
    pub(crate) ok: bool,
    pub(crate) message: String,
    pub(crate) normalized_value: Option<String>,
    pub(crate) discovered_nodes: Option<usize>,
    pub(crate) latency_ms: Option<f64>,
}

impl ForwardProxyCandidateValidationResponse {
    pub(crate) fn success(
        message: impl Into<String>,
        normalized_value: Option<String>,
        discovered_nodes: Option<usize>,
        latency_ms: Option<f64>,
    ) -> Self {
        Self {
            ok: true,
            message: message.into(),
            normalized_value,
            discovered_nodes,
            latency_ms,
        }
    }

    pub(crate) fn failed(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: message.into(),
            normalized_value: None,
            discovered_nodes: None,
            latency_ms: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForwardProxyProtocol {
    Direct,
    Http,
    Https,
    Socks5,
    Socks5h,
    Vmess,
    Vless,
    Trojan,
    Shadowsocks,
}
