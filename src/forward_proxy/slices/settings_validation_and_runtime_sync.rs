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

    let mut subscription_proxy_urls = Vec::new();
    let mut fetched_any_subscription = false;
    for subscription_url in &subscription_urls {
        if state.shutdown.is_cancelled() {
            info!("stopping forward proxy subscription refresh because shutdown is in progress");
            return Ok(());
        }
        let fetch_result = tokio::select! {
            _ = state.shutdown.cancelled() => {
                info!("stopping forward proxy subscription refresh because shutdown is in progress");
                return Ok(());
            }
            result = fetch_subscription_proxy_urls(
                &state.http_clients.shared,
                subscription_url,
                state.config.request_timeout,
            ) => result,
        };
        match fetch_result {
            Ok(urls) => {
                fetched_any_subscription = true;
                subscription_proxy_urls.extend(urls);
            }
            Err(err) => {
                warn!(
                    subscription_url,
                    error = %err,
                    "failed to fetch forward proxy subscription"
                );
            }
        }
    }

    if !subscription_urls.is_empty() && !fetched_any_subscription {
        bail!("all forward proxy subscriptions failed to refresh");
    }
    if state.shutdown.is_cancelled() {
        info!("stopping forward proxy subscription refresh because shutdown is in progress");
        return Ok(());
    }

    let _refresh_guard = state.forward_proxy_subscription_refresh_lock.lock().await;
    let added_subscription_endpoints = {
        let mut manager = state.forward_proxy.lock().await;
        if state.shutdown.is_cancelled() {
            info!(
                "stopping forward proxy subscription refresh before applying refreshed endpoints because shutdown is in progress"
            );
            return Ok(());
        }
        if manager.settings.subscription_urls != subscription_urls {
            debug!("skip stale forward proxy subscription refresh after settings changed");
            return Ok(());
        }
        let mut known_subscription_keys = snapshot_active_forward_proxy_endpoints(&manager)
            .into_iter()
            .filter(|endpoint| endpoint.source == FORWARD_PROXY_SOURCE_SUBSCRIPTION)
            .map(|endpoint| endpoint.key)
            .collect::<HashSet<_>>();
        if let Some(override_keys) = &known_subscription_keys_override {
            known_subscription_keys.extend(override_keys.iter().cloned());
        }
        manager.apply_subscription_urls(subscription_proxy_urls);
        let after = snapshot_active_forward_proxy_endpoints(&manager);
        after
            .into_iter()
            .filter(|endpoint| endpoint.source == FORWARD_PROXY_SOURCE_SUBSCRIPTION)
            .filter(|endpoint| !known_subscription_keys.contains(&endpoint.key))
            .collect::<Vec<_>>()
    };
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

async fn canonicalize_forward_proxy_route_scope(
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
    let (updated_runtime, probe_candidate, attempt_update) = {
        let mut manager = state.forward_proxy.lock().await;
        let runtime_active = manager
            .endpoints
            .iter()
            .any(|endpoint| endpoint.key == selected_proxy.key);
        let weight_before = if runtime_active {
            manager
                .runtime
                .get(&selected_proxy.key)
                .map(|runtime| runtime.weight)
        } else {
            None
        };
        manager.record_attempt(&selected_proxy.key, success, latency_ms, is_probe);
        let updated_runtime = if runtime_active {
            manager.runtime.get(&selected_proxy.key).cloned()
        } else {
            None
        };
        let weight_after = updated_runtime.as_ref().map(|runtime| runtime.weight);
        let weight_delta = match (weight_before, weight_after) {
            (Some(before), Some(after)) if before.is_finite() && after.is_finite() => {
                Some(after - before)
            }
            _ => None,
        };
        let probe_candidate = if is_probe
            // A 429 already tells us to back off; probing immediately just adds more traffic and
            // ignores upstream Retry-After guidance.
            || failure_kind == Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
        {
            None
        } else {
            manager.mark_probe_started()
        };
        (
            updated_runtime,
            probe_candidate,
            ForwardProxyAttemptUpdate {
                weight_before,
                weight_after,
                weight_delta,
            },
        )
    };

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
        warn!(
            proxy_key_ref = %forward_proxy_log_ref(&selected_proxy.key),
            error = %err,
            "failed to persist forward proxy attempt"
        );
    }

    if let Some(runtime) = updated_runtime {
        let sample_epoch_us = Utc::now().timestamp_micros();
        let bucket_start_epoch = align_bucket_epoch(sample_epoch_us.div_euclid(1_000_000), 3600, 0);
        if let Err(err) = persist_forward_proxy_runtime_state(&state.pool, &runtime).await {
            warn!(
                proxy_key_ref = %forward_proxy_log_ref(&runtime.proxy_key),
                error = %err,
                "failed to persist forward proxy runtime state"
            );
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
            warn!(
                proxy_key_ref = %forward_proxy_log_ref(&runtime.proxy_key),
                error = %err,
                "failed to persist forward proxy weight bucket"
            );
        }
    }

    if let Some(candidate) = probe_candidate {
        spawn_penalized_forward_proxy_probe(state, candidate);
    }

    attempt_update
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

impl ForwardProxyProtocol {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Direct => "DIRECT",
            Self::Http => "HTTP",
            Self::Https => "HTTPS",
            Self::Socks5 => "SOCKS5",
            Self::Socks5h => "SOCKS5H",
            Self::Vmess => "VMESS",
            Self::Vless => "VLESS",
            Self::Trojan => "TROJAN",
            Self::Shadowsocks => "SS",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ForwardProxyEndpoint {
    pub(crate) key: String,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) protocol: ForwardProxyProtocol,
    pub(crate) endpoint_url: Option<Url>,
    pub(crate) raw_url: Option<String>,
}

impl ForwardProxyEndpoint {
    pub(crate) fn direct() -> Self {
        Self {
            key: FORWARD_PROXY_DIRECT_KEY.to_string(),
            source: FORWARD_PROXY_SOURCE_DIRECT.to_string(),
            display_name: FORWARD_PROXY_DIRECT_LABEL.to_string(),
            protocol: ForwardProxyProtocol::Direct,
            endpoint_url: None,
            raw_url: None,
        }
    }

    pub(crate) fn is_selectable(&self) -> bool {
        self.endpoint_url.is_some()
    }

    pub(crate) fn is_bound_selectable(&self) -> bool {
        self.endpoint_url.is_some() || matches!(self.protocol, ForwardProxyProtocol::Direct)
    }

    pub(crate) fn requires_xray(&self) -> bool {
        matches!(
            self.protocol,
            ForwardProxyProtocol::Vmess
                | ForwardProxyProtocol::Vless
                | ForwardProxyProtocol::Trojan
                | ForwardProxyProtocol::Shadowsocks
        )
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ForwardProxyRuntimeState {
    pub(crate) proxy_key: String,
    pub(crate) display_name: String,
    pub(crate) source: String,
    pub(crate) endpoint_url: Option<String>,
    pub(crate) weight: f64,
    pub(crate) success_ema: f64,
    pub(crate) latency_ema_ms: Option<f64>,
    pub(crate) consecutive_failures: u32,
}

impl ForwardProxyRuntimeState {
    pub(crate) fn default_for_endpoint(
        endpoint: &ForwardProxyEndpoint,
        algo: ForwardProxyAlgo,
    ) -> Self {
        Self {
            proxy_key: endpoint.key.clone(),
            display_name: endpoint.display_name.clone(),
            source: endpoint.source.clone(),
            endpoint_url: endpoint.raw_url.clone(),
            weight: if endpoint.key == FORWARD_PROXY_DIRECT_KEY {
                match algo {
                    ForwardProxyAlgo::V1 => 1.0,
                    ForwardProxyAlgo::V2 => FORWARD_PROXY_V2_DIRECT_INITIAL_WEIGHT,
                }
            } else {
                0.8
            },
            success_ema: 0.65,
            latency_ema_ms: None,
            consecutive_failures: 0,
        }
    }

    pub(crate) fn is_penalized(&self) -> bool {
        self.weight <= 0.0
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct ForwardProxyRuntimeRow {
    pub(crate) proxy_key: String,
    pub(crate) display_name: String,
    pub(crate) source: String,
    pub(crate) endpoint_url: Option<String>,
    pub(crate) weight: f64,
    pub(crate) success_ema: f64,
    pub(crate) latency_ema_ms: Option<f64>,
    pub(crate) consecutive_failures: i64,
}

impl From<ForwardProxyRuntimeRow> for ForwardProxyRuntimeState {
    fn from(value: ForwardProxyRuntimeRow) -> Self {
        Self {
            proxy_key: value.proxy_key,
            display_name: value.display_name,
            source: value.source,
            endpoint_url: value.endpoint_url,
            weight: value.weight,
            success_ema: value.success_ema.clamp(0.0, 1.0),
            latency_ema_ms: value.latency_ema_ms,
            consecutive_failures: value.consecutive_failures.max(0) as u32,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ForwardProxyManager {
    pub(crate) algo: ForwardProxyAlgo,
    pub(crate) settings: ForwardProxySettings,
    pub(crate) endpoints: Vec<ForwardProxyEndpoint>,
    pub(crate) runtime: HashMap<String, ForwardProxyRuntimeState>,
    pub(crate) bound_key_aliases: HashMap<String, String>,
    pub(crate) bound_key_endpoint_keys: HashMap<String, String>,
    pub(crate) bound_key_by_endpoint_key: HashMap<String, String>,
    pub(crate) bound_node_descriptors: Vec<ForwardProxyBindingNodeDescriptor>,
    pub(crate) bound_group_runtime: HashMap<String, BoundForwardProxyGroupState>,
    pub(crate) selection_counter: u64,
    pub(crate) requests_since_probe: u64,
    pub(crate) probe_in_flight: bool,
    pub(crate) last_probe_at: DateTime<Utc>,
    pub(crate) last_subscription_refresh_at: Option<DateTime<Utc>>,
}

const BOUND_FORWARD_PROXY_SWITCH_FAILURE_THRESHOLD: u32 = 3;

#[derive(Debug, Clone, Default)]
pub(crate) struct BoundForwardProxyGroupState {
    pub(crate) current_binding_key: Option<String>,
    pub(crate) consecutive_network_failures: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct ForwardProxyBindingNodeDescriptor {
    pub(crate) key: String,
    pub(crate) endpoint_key: String,
    pub(crate) alias_keys: Vec<String>,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) protocol_label: String,
}

#[derive(Debug, Clone)]
pub(crate) enum ForwardProxyRouteScope {
    Automatic,
    PinnedProxyKey(String),
    BoundGroup {
        group_name: String,
        bound_proxy_keys: Vec<String>,
    },
}

impl ForwardProxyRouteScope {
    pub(crate) fn from_group_binding(
        group_name: Option<&str>,
        bound_proxy_keys: Vec<String>,
    ) -> Self {
        let normalized_group_name = group_name
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let normalized_bound_proxy_keys = bound_proxy_keys
            .into_iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(|value| normalize_bound_proxy_key(&value).unwrap_or(value))
            .collect::<Vec<_>>();
        match (
            normalized_group_name,
            normalized_bound_proxy_keys.is_empty(),
        ) {
            (Some(group_name), false) => Self::BoundGroup {
                group_name,
                bound_proxy_keys: normalized_bound_proxy_keys,
            },
            _ => Self::Automatic,
        }
    }

    pub(crate) fn pinned(proxy_key: impl Into<String>) -> Self {
        Self::PinnedProxyKey(proxy_key.into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForwardProxyRouteResultKind {
    CompletedRequest,
    NetworkFailure,
}

impl ForwardProxyManager {
    #[cfg(test)]
    pub(crate) fn new(
        settings: ForwardProxySettings,
        runtime_rows: Vec<ForwardProxyRuntimeState>,
    ) -> Self {
        Self::with_algo(settings, runtime_rows, ForwardProxyAlgo::V1)
    }

    pub(crate) fn with_algo(
        settings: ForwardProxySettings,
        runtime_rows: Vec<ForwardProxyRuntimeState>,
        algo: ForwardProxyAlgo,
    ) -> Self {
        let runtime = runtime_rows
            .into_iter()
            .map(|mut entry| {
                Self::normalize_runtime_for_algo(&mut entry, algo);
                (entry.proxy_key.clone(), entry)
            })
            .collect::<HashMap<_, _>>();
        let mut manager = Self {
            algo,
            settings,
            endpoints: Vec::new(),
            runtime,
            bound_key_aliases: HashMap::new(),
            bound_key_endpoint_keys: HashMap::new(),
            bound_key_by_endpoint_key: HashMap::new(),
            bound_node_descriptors: Vec::new(),
            bound_group_runtime: HashMap::new(),
            selection_counter: 0,
            requests_since_probe: 0,
            probe_in_flight: false,
            last_probe_at: Utc::now() - ChronoDuration::seconds(algo.probe_interval_secs()),
            last_subscription_refresh_at: None,
        };
        manager.rebuild_endpoints(Vec::new());
        manager
    }

    pub(crate) fn normalize_runtime_for_algo(
        runtime: &mut ForwardProxyRuntimeState,
        algo: ForwardProxyAlgo,
    ) {
        runtime.success_ema = runtime.success_ema.clamp(0.0, 1.0);
        if runtime
            .latency_ema_ms
            .is_some_and(|value| !value.is_finite() || value < 0.0)
        {
            runtime.latency_ema_ms = None;
        }
        if !runtime.weight.is_finite() {
            runtime.weight = 0.0;
        }
        runtime.weight = match algo {
            ForwardProxyAlgo::V1 => runtime
                .weight
                .clamp(FORWARD_PROXY_WEIGHT_MIN, FORWARD_PROXY_WEIGHT_MAX),
            ForwardProxyAlgo::V2 => runtime
                .weight
                .clamp(FORWARD_PROXY_V2_WEIGHT_MIN, FORWARD_PROXY_V2_WEIGHT_MAX),
        };
    }

    pub(crate) fn apply_settings(&mut self, settings: ForwardProxySettings) {
        self.settings = settings;
        self.rebuild_endpoints(Vec::new());
    }

    pub(crate) fn apply_subscription_urls(&mut self, proxy_urls: Vec<String>) {
        let normalized_urls = normalize_proxy_url_entries(proxy_urls);
        let subscription_endpoints = normalize_proxy_endpoints_from_urls(
            &normalized_urls,
            FORWARD_PROXY_SOURCE_SUBSCRIPTION,
        );
        self.rebuild_endpoints(subscription_endpoints);
        self.last_subscription_refresh_at = Some(Utc::now());
    }

    pub(crate) fn rebuild_endpoints(&mut self, subscription_endpoints: Vec<ForwardProxyEndpoint>) {
        let mut merged = Vec::new();
        let manual = normalize_proxy_endpoints_from_urls(
            &self.settings.proxy_urls,
            FORWARD_PROXY_SOURCE_MANUAL,
        );
        let mut seen = HashSet::new();
        for endpoint in manual.into_iter().chain(subscription_endpoints.into_iter()) {
            if seen.insert(endpoint.key.clone()) {
                merged.push(endpoint);
            }
        }
        self.endpoints = merged;
        self.rebuild_bound_key_registry();

        let endpoint_snapshots = self.endpoints.clone();
        for endpoint in &endpoint_snapshots {
            self.migrate_runtime_aliases_to_endpoint(endpoint);
        }

        let algo = self.algo;
        for endpoint in &self.endpoints {
            match self.runtime.entry(endpoint.key.clone()) {
                std::collections::hash_map::Entry::Occupied(mut occupied) => {
                    let runtime = occupied.get_mut();
                    runtime.display_name = endpoint.display_name.clone();
                    runtime.source = endpoint.source.clone();
                    runtime.endpoint_url = endpoint.raw_url.clone();
                }
                std::collections::hash_map::Entry::Vacant(vacant) => {
                    vacant.insert(ForwardProxyRuntimeState::default_for_endpoint(
                        endpoint, algo,
                    ));
                }
            }
        }
        self.ensure_non_zero_weight();
    }

    fn binding_parts_for_endpoint(
        endpoint: &ForwardProxyEndpoint,
    ) -> Option<ForwardProxyBindingParts> {
        endpoint.raw_url.as_deref().and_then(|raw_url| {
            forward_proxy_binding_parts_from_raw(raw_url, Some(&endpoint.display_name))
        })
    }

    fn rebuild_bound_key_registry(&mut self) {
        let mut name_counts = HashMap::<String, usize>::new();
        let mut name_protocol_counts = HashMap::<(String, String), usize>::new();
        let mut endpoint_parts = self
            .endpoints
            .iter()
            .filter_map(|endpoint| {
                Self::binding_parts_for_endpoint(endpoint).map(|parts| (endpoint.clone(), parts))
            })
            .collect::<Vec<_>>();
        endpoint_parts.sort_by(|lhs, rhs| lhs.0.key.cmp(&rhs.0.key));

        for (_, parts) in &endpoint_parts {
            *name_counts.entry(parts.display_name.clone()).or_default() += 1;
            *name_protocol_counts
                .entry((parts.display_name.clone(), parts.protocol_key.clone()))
                .or_default() += 1;
        }

        let mut bound_key_aliases = HashMap::new();
        let mut bound_key_endpoint_keys = HashMap::new();
        let mut bound_key_by_endpoint_key = HashMap::new();
        let mut descriptor_aliases = HashMap::<String, HashSet<String>>::new();
        let mut bound_node_descriptors = Vec::new();

        for (endpoint, parts) in endpoint_parts {
            let candidate_keys = forward_proxy_binding_key_candidates(&parts);
            let binding_key = if name_counts
                .get(&parts.display_name)
                .copied()
                .unwrap_or_default()
                <= 1
            {
                candidate_keys[0].clone()
            } else if name_protocol_counts
                .get(&(parts.display_name.clone(), parts.protocol_key.clone()))
                .copied()
                .unwrap_or_default()
                <= 1
            {
                candidate_keys[1].clone()
            } else {
                candidate_keys[2].clone()
            };

            let primary_endpoint = bound_key_endpoint_keys
                .entry(binding_key.clone())
                .or_insert_with(|| endpoint.key.clone())
                .clone();
            bound_key_by_endpoint_key.insert(endpoint.key.clone(), binding_key.clone());

            let aliases = descriptor_aliases.entry(binding_key.clone()).or_default();
            for candidate_key in candidate_keys {
                if candidate_key != binding_key {
                    bound_key_aliases
                        .entry(candidate_key.clone())
                        .or_insert_with(|| binding_key.clone());
                    aliases.insert(candidate_key);
                }
            }
            bound_key_aliases
                .entry(endpoint.key.clone())
                .or_insert_with(|| binding_key.clone());
            aliases.insert(endpoint.key.clone());

            if let Some(raw_url) = endpoint.raw_url.as_deref() {
                if let Some((canonical_key, storage_aliases)) =
                    forward_proxy_storage_aliases(raw_url)
                {
                    if canonical_key != binding_key {
                        bound_key_aliases
                            .entry(canonical_key.clone())
                            .or_insert_with(|| binding_key.clone());
                        aliases.insert(canonical_key);
                    }
                    for alias in storage_aliases {
                        if alias != binding_key {
                            bound_key_aliases
                                .entry(alias.clone())
                                .or_insert_with(|| binding_key.clone());
                            aliases.insert(alias);
                        }
                    }
                }
                for alias in legacy_bound_proxy_key_aliases(raw_url, endpoint.protocol) {
                    if alias != binding_key {
                        bound_key_aliases
                            .entry(alias.clone())
                            .or_insert_with(|| binding_key.clone());
                        aliases.insert(alias);
                    }
                }
            }

            if primary_endpoint == endpoint.key {
                bound_node_descriptors.push(ForwardProxyBindingNodeDescriptor {
                    key: binding_key,
                    endpoint_key: endpoint.key.clone(),
                    alias_keys: Vec::new(),
                    source: endpoint.source.clone(),
                    display_name: endpoint.display_name.clone(),
                    protocol_label: endpoint.protocol.label().to_string(),
                });
            }
        }

        for descriptor in &mut bound_node_descriptors {
            let mut alias_keys = descriptor_aliases
                .remove(&descriptor.key)
                .unwrap_or_default()
                .into_iter()
                .filter(|alias| alias != &descriptor.key)
                .collect::<Vec<_>>();
            alias_keys.sort();
            alias_keys.dedup();
            descriptor.alias_keys = alias_keys;
        }
        bound_node_descriptors.sort_by(|lhs, rhs| lhs.display_name.cmp(&rhs.display_name));

        self.bound_key_aliases = bound_key_aliases;
        self.bound_key_endpoint_keys = bound_key_endpoint_keys;
        self.bound_key_by_endpoint_key = bound_key_by_endpoint_key;
        self.bound_node_descriptors = bound_node_descriptors;
    }

    fn resolve_current_bound_proxy_key_from_parts(
        &self,
        parts: &ForwardProxyBindingParts,
    ) -> Option<String> {
        for candidate in forward_proxy_binding_key_candidates(parts) {
            if self.bound_key_endpoint_keys.contains_key(&candidate) {
                return Some(candidate);
            }
            if let Some(canonical) = self.bound_key_aliases.get(&candidate) {
                return Some(canonical.clone());
            }
        }
        None
    }

    fn resolve_current_bound_proxy_key(&self, proxy_key: &str) -> Option<String> {
        let normalized = normalize_bound_proxy_key(proxy_key)?;
        if normalized == FORWARD_PROXY_DIRECT_KEY {
            return Some(normalized);
        }
        if self.bound_key_endpoint_keys.contains_key(&normalized) {
            return Some(normalized);
        }
        self.bound_key_aliases.get(&normalized).cloned()
    }

    fn resolve_bound_proxy_key_from_history_row(
        &self,
        row: &ForwardProxyMetadataHistoryRow,
    ) -> Option<String> {
        let raw = row.endpoint_url.as_deref()?;
        let parts = forward_proxy_binding_parts_from_raw(raw, Some(&row.display_name))?;
        self.resolve_current_bound_proxy_key_from_parts(&parts)
    }

    pub(crate) fn resolve_current_or_historical_bound_proxy_key(
        &self,
        proxy_key: &str,
        history_row: Option<&ForwardProxyMetadataHistoryRow>,
    ) -> Option<String> {
        self.resolve_current_bound_proxy_key(proxy_key).or_else(|| {
            history_row.and_then(|row| self.resolve_bound_proxy_key_from_history_row(row))
        })
    }

    pub(crate) fn canonicalize_bound_proxy_key(
        &self,
        proxy_key: &str,
        history_row: Option<&ForwardProxyMetadataHistoryRow>,
    ) -> Option<String> {
        let normalized = normalize_bound_proxy_key(proxy_key)?;
        self.resolve_current_or_historical_bound_proxy_key(&normalized, history_row)
            .or(Some(normalized))
    }

    pub(crate) fn current_bound_group_binding_key(
        &self,
        group_name: &str,
        bound_proxy_keys: &[String],
    ) -> Option<String> {
        let available_keys = self.selectable_bound_proxy_keys_in_order(bound_proxy_keys);
        if available_keys.is_empty() {
            return None;
        }
        self.bound_group_runtime
            .get(group_name)
            .and_then(|state| state.current_binding_key.as_deref())
            .and_then(|key| self.resolve_current_bound_proxy_key(key).or_else(|| normalize_bound_proxy_key(key)))
            .filter(|key| available_keys.contains(key))
    }

    fn migrate_runtime_aliases_to_endpoint(&mut self, endpoint: &ForwardProxyEndpoint) {
        let Some(raw_url) = endpoint.raw_url.as_deref() else {
            return;
        };
        let Some((canonical_key, aliases)) = forward_proxy_storage_aliases(raw_url) else {
            return;
        };
        if canonical_key != endpoint.key {
            return;
        }

        if self.runtime.contains_key(&endpoint.key) {
            for alias in aliases {
                self.runtime.remove(&alias);
            }
            return;
        }

        let mut migrated = None;
        for alias in &aliases {
            if let Some(runtime) = self.runtime.remove(alias) {
                migrated = Some(runtime);
                break;
            }
        }
        for alias in aliases {
            self.runtime.remove(&alias);
        }
        if let Some(mut runtime) = migrated {
            runtime.proxy_key = endpoint.key.clone();
            runtime.endpoint_url = Some(raw_url.to_string());
            self.runtime.insert(endpoint.key.clone(), runtime);
        }
    }

    pub(crate) fn ensure_non_zero_weight(&mut self) {
        let minimum = match self.algo {
            ForwardProxyAlgo::V1 => 1,
            ForwardProxyAlgo::V2 => FORWARD_PROXY_V2_MIN_POSITIVE_CANDIDATES,
        };
        self.ensure_min_positive_candidates(minimum, self.algo.probe_recovery_weight());
    }

    pub(crate) fn selectable_endpoint_keys(&self) -> HashSet<&str> {
        self.endpoints
            .iter()
            .filter(|endpoint| endpoint.is_selectable())
            .map(|endpoint| endpoint.key.as_str())
            .collect::<HashSet<_>>()
    }

    pub(crate) fn selectable_proxy_keys(&self) -> HashSet<String> {
        let mut keys = self
            .bound_node_descriptors
            .iter()
            .filter_map(|descriptor| {
                self.endpoints
                    .iter()
                    .find(|endpoint| endpoint.key == descriptor.endpoint_key)
                    .filter(|endpoint| endpoint.is_bound_selectable())
                    .map(|_| descriptor.key.clone())
            })
            .collect::<HashSet<_>>();
        keys.insert(FORWARD_PROXY_DIRECT_KEY.to_string());
        keys
    }

    pub(crate) fn ensure_min_positive_candidates(&mut self, minimum: usize, recovery_weight: f64) {
        if minimum == 0 {
            return;
        }

        let selectable_keys = self.selectable_endpoint_keys();
        let active_keys = if selectable_keys.is_empty() {
            self.endpoints
                .iter()
                .map(|endpoint| endpoint.key.as_str())
                .collect::<HashSet<_>>()
        } else {
            selectable_keys
        };
        let mut positive_count = self
            .runtime
            .values()
            .filter(|entry| {
                active_keys.contains(entry.proxy_key.as_str())
                    && entry.weight > 0.0
                    && entry.weight.is_finite()
            })
            .count();
        if positive_count >= minimum {
            return;
        }

        let mut candidates = self
            .runtime
            .values()
            .filter(|entry| active_keys.contains(entry.proxy_key.as_str()))
            .map(|entry| (entry.proxy_key.clone(), entry.weight))
            .collect::<Vec<_>>();
        candidates.sort_by(|lhs, rhs| rhs.1.total_cmp(&lhs.1));

        for (proxy_key, _) in candidates {
            if positive_count >= minimum {
                break;
            }
            if let Some(entry) = self.runtime.get_mut(&proxy_key)
                && !(entry.weight > 0.0 && entry.weight.is_finite())
            {
                entry.weight = recovery_weight;
                if self.algo == ForwardProxyAlgo::V2 {
                    entry.consecutive_failures = 0;
                }
                positive_count += 1;
            }
        }
    }

    pub(crate) fn snapshot_runtime(&self) -> Vec<ForwardProxyRuntimeState> {
        self.endpoints
            .iter()
            .filter_map(|endpoint| self.runtime.get(&endpoint.key).cloned())
            .collect()
    }

    fn next_random_index(&mut self, upper_bound: usize) -> usize {
        debug_assert!(upper_bound > 0);
        self.selection_counter = self.selection_counter.wrapping_add(1);
        let random = deterministic_unit_f64(self.selection_counter);
        ((random * upper_bound as f64).floor() as usize).min(upper_bound.saturating_sub(1))
    }

    fn selectable_bound_proxy_keys(&self, bound_proxy_keys: &[String]) -> Vec<String> {
        let selectable = self.selectable_proxy_keys();
        let mut seen = HashSet::new();
        let mut available = Vec::new();
        for key in bound_proxy_keys {
            let normalized = key.trim();
            if normalized.is_empty() {
                continue;
            }
            let canonical = self
                .resolve_current_bound_proxy_key(normalized)
                .unwrap_or_else(|| normalized.to_string());
            if !selectable.contains(&canonical) || !seen.insert(canonical.clone()) {
                continue;
            }
            available.push(canonical);
        }
        available.sort();
        available
    }

    pub(crate) fn selectable_bound_proxy_keys_in_order(
        &self,
        bound_proxy_keys: &[String],
    ) -> Vec<String> {
        let selectable = self.selectable_proxy_keys();
        let mut seen = HashSet::new();
        let mut available = Vec::new();
        for key in bound_proxy_keys {
            let normalized = key.trim();
            if normalized.is_empty() {
                continue;
            }
            let canonical = normalize_bound_proxy_key(normalized)
                .map(|value| self.bound_key_aliases.get(&value).cloned().unwrap_or(value))
                .unwrap_or_else(|| normalized.to_string());
            if !selectable.contains(&canonical) || !seen.insert(canonical.clone()) {
                continue;
            }
            available.push(canonical);
        }
        available
    }

    pub(crate) fn has_selectable_bound_proxy_keys(&self, bound_proxy_keys: &[String]) -> bool {
        !self
            .selectable_bound_proxy_keys(bound_proxy_keys)
            .is_empty()
    }

    fn choose_random_bound_proxy_key(
        &mut self,
        available_keys: &[String],
        exclude_key: Option<&str>,
    ) -> Option<String> {
        let candidates = available_keys
            .iter()
            .filter(|candidate| Some(candidate.as_str()) != exclude_key)
            .cloned()
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return None;
        }
        Some(candidates[self.next_random_index(candidates.len())].clone())
    }

    pub(crate) fn select_auto_proxy(&mut self) -> Option<SelectedForwardProxy> {
        self.selection_counter = self.selection_counter.wrapping_add(1);
        self.requests_since_probe = self.requests_since_probe.saturating_add(1);
        self.ensure_non_zero_weight();

        let mut candidates = Vec::new();
        let mut total_weight = 0.0f64;
        for endpoint in &self.endpoints {
            if !endpoint.is_selectable() {
                continue;
            }
            if let Some(runtime) = self.runtime.get(&endpoint.key)
                && runtime.weight > 0.0
                && runtime.weight.is_finite()
            {
                let effective_weight = if self.algo == ForwardProxyAlgo::V2 {
                    let success_factor = runtime.success_ema.clamp(0.0, 1.0).powi(8).max(0.01);
                    runtime.weight.powi(2) * success_factor
                } else {
                    runtime.weight
                };
                total_weight += effective_weight;
                candidates.push((endpoint, effective_weight));
            }
        }

        if self.algo == ForwardProxyAlgo::V2 && candidates.len() > 3 {
            candidates.sort_by(|lhs, rhs| rhs.1.total_cmp(&lhs.1));
            candidates.truncate(3);
            total_weight = candidates.iter().map(|(_, weight)| *weight).sum::<f64>();
        }

        if candidates.is_empty() {
            return None;
        }

        let seed = self.selection_counter;
        let random = deterministic_unit_f64(seed);
        let mut threshold = random * total_weight;
        let mut last_candidate: Option<&ForwardProxyEndpoint> = None;
        for (endpoint, weight) in candidates {
            last_candidate = Some(endpoint);
            if threshold <= weight {
                return Some(SelectedForwardProxy::from_endpoint(endpoint));
            }
            threshold -= weight;
        }
        last_candidate.map(SelectedForwardProxy::from_endpoint)
    }

    fn select_bound_group_proxy(
        &mut self,
        group_name: &str,
        bound_proxy_keys: &[String],
    ) -> Result<SelectedForwardProxy> {
        let available_keys = self.selectable_bound_proxy_keys(bound_proxy_keys);
        if available_keys.is_empty() {
            self.bound_group_runtime.remove(group_name);
            bail!("bound forward proxy group has no selectable nodes");
        }
        let existing_current = self
            .bound_group_runtime
            .get(group_name)
            .and_then(|state| state.current_binding_key.clone())
            .filter(|key| available_keys.contains(key));
        let selected_binding_key = existing_current.unwrap_or_else(|| {
            self.choose_random_bound_proxy_key(&available_keys, None)
                .expect("available bound proxy keys should not be empty")
        });
        let state = self
            .bound_group_runtime
            .entry(group_name.to_string())
            .or_default();
        state.current_binding_key = Some(selected_binding_key.clone());
        if selected_binding_key == FORWARD_PROXY_DIRECT_KEY {
            return Ok(SelectedForwardProxy::from_endpoint(
                &ForwardProxyEndpoint::direct(),
            ));
        }
        let endpoint_key = self
            .bound_key_endpoint_keys
            .get(&selected_binding_key)
            .ok_or_else(|| anyhow!("selected bound proxy disappeared from runtime"))?;
        let endpoint = self
            .endpoints
            .iter()
            .find(|endpoint| endpoint.key == *endpoint_key)
            .ok_or_else(|| anyhow!("selected bound proxy disappeared from runtime"))?;
        Ok(SelectedForwardProxy::from_endpoint(endpoint))
    }

    fn select_pinned_proxy_key(&self, proxy_key: &str) -> Result<SelectedForwardProxy> {
        let normalized_proxy_key = proxy_key.trim();
        if normalized_proxy_key.is_empty() {
            bail!("pinned forward proxy key is empty");
        }
        if normalized_proxy_key == FORWARD_PROXY_DIRECT_KEY {
            return Ok(SelectedForwardProxy::from_endpoint(
                &ForwardProxyEndpoint::direct(),
            ));
        }
        let canonical_proxy_key = self
            .resolve_current_bound_proxy_key(normalized_proxy_key)
            .unwrap_or_else(|| normalized_proxy_key.to_string());
        let endpoint_key = self
            .bound_key_endpoint_keys
            .get(&canonical_proxy_key)
            .cloned()
            .unwrap_or(canonical_proxy_key);
        let endpoint = self
            .endpoints
            .iter()
            .find(|endpoint| endpoint.key == endpoint_key)
            .ok_or_else(|| anyhow!("pinned forward proxy key is no longer available"))?;
        if !endpoint.is_bound_selectable() {
            bail!("pinned forward proxy key is no longer available");
        }
        Ok(SelectedForwardProxy::from_endpoint(endpoint))
    }

    pub(crate) fn select_proxy_for_scope(
        &mut self,
        scope: &ForwardProxyRouteScope,
    ) -> Result<SelectedForwardProxy> {
        match scope {
            ForwardProxyRouteScope::Automatic => {
                if let Some(selected) = self.select_auto_proxy() {
                    Ok(selected)
                } else {
                    #[cfg(test)]
                    {
                        Ok(SelectedForwardProxy::from_endpoint(
                            &ForwardProxyEndpoint::direct(),
                        ))
                    }
                    #[cfg(not(test))]
                    {
                        Err(anyhow!("no selectable forward proxy nodes configured"))
                    }
                }
            }
            ForwardProxyRouteScope::PinnedProxyKey(proxy_key) => {
                self.select_pinned_proxy_key(proxy_key)
            }
            ForwardProxyRouteScope::BoundGroup {
                group_name,
                bound_proxy_keys,
            } => self.select_bound_group_proxy(group_name, bound_proxy_keys),
        }
    }

    pub(crate) fn record_scope_result(
        &mut self,
        scope: &ForwardProxyRouteScope,
        selected_proxy_key: &str,
        result: ForwardProxyRouteResultKind,
    ) {
        let ForwardProxyRouteScope::BoundGroup {
            group_name,
            bound_proxy_keys,
        } = scope
        else {
            return;
        };
        let available_keys = self.selectable_bound_proxy_keys(bound_proxy_keys);
        if available_keys.is_empty() {
            self.bound_group_runtime.remove(group_name);
            return;
        }

        let selected_binding_key = self
            .resolve_current_bound_proxy_key(selected_proxy_key)
            .unwrap_or_else(|| selected_proxy_key.to_string());
        let mut should_switch = false;
        {
            let state = self
                .bound_group_runtime
                .entry(group_name.clone())
                .or_default();
            state.current_binding_key = Some(selected_binding_key.clone());
            match result {
                ForwardProxyRouteResultKind::CompletedRequest => {
                    state.consecutive_network_failures = 0;
                }
                ForwardProxyRouteResultKind::NetworkFailure => {
                    state.consecutive_network_failures =
                        state.consecutive_network_failures.saturating_add(1);
                    should_switch = state.consecutive_network_failures
                        >= BOUND_FORWARD_PROXY_SWITCH_FAILURE_THRESHOLD;
                }
            }
        }

        if should_switch
            && let Some(next_binding_key) = self
                .choose_random_bound_proxy_key(&available_keys, Some(selected_binding_key.as_str()))
        {
            let state = self
                .bound_group_runtime
                .entry(group_name.clone())
                .or_default();
            state.current_binding_key = Some(next_binding_key);
            state.consecutive_network_failures = 0;
        }
    }

    pub(crate) fn binding_nodes(&self) -> Vec<ForwardProxyBindingNodeResponse> {
        let mut nodes = self
            .bound_node_descriptors
            .iter()
            .map(|descriptor| {
                let penalized = self
                    .runtime
                    .get(&descriptor.endpoint_key)
                    .is_some_and(ForwardProxyRuntimeState::is_penalized);
                ForwardProxyBindingNodeResponse {
                    key: descriptor.key.clone(),
                    alias_keys: descriptor.alias_keys.clone(),
                    source: descriptor.source.clone(),
                    display_name: descriptor.display_name.clone(),
                    protocol_label: descriptor.protocol_label.clone(),
                    penalized,
                    selectable: self
                        .endpoints
                        .iter()
                        .find(|endpoint| endpoint.key == descriptor.endpoint_key)
                        .is_some_and(ForwardProxyEndpoint::is_bound_selectable),
                    last24h: Vec::new(),
                }
            })
            .collect::<Vec<_>>();
        nodes.push(ForwardProxyBindingNodeResponse {
            key: FORWARD_PROXY_DIRECT_KEY.to_string(),
            alias_keys: Vec::new(),
            source: FORWARD_PROXY_SOURCE_DIRECT.to_string(),
            display_name: FORWARD_PROXY_DIRECT_LABEL.to_string(),
            protocol_label: ForwardProxyProtocol::Direct.label().to_string(),
            penalized: false,
            selectable: true,
            last24h: Vec::new(),
        });
        nodes.sort_by(|lhs, rhs| lhs.display_name.cmp(&rhs.display_name));
        nodes
    }

    pub(crate) fn record_attempt(
        &mut self,
        proxy_key: &str,
        success: bool,
        latency_ms: Option<f64>,
        is_probe: bool,
    ) {
        if !self
            .endpoints
            .iter()
            .any(|endpoint| endpoint.key == proxy_key)
        {
            return;
        }
        let Some(runtime) = self.runtime.get_mut(proxy_key) else {
            return;
        };

        Self::update_runtime_ema(runtime, success, latency_ms);
        match self.algo {
            ForwardProxyAlgo::V1 => Self::record_attempt_v1(runtime, success, is_probe),
            ForwardProxyAlgo::V2 => Self::record_attempt_v2(runtime, success, is_probe),
        }
        self.ensure_non_zero_weight();
    }

    pub(crate) fn update_runtime_ema(
        runtime: &mut ForwardProxyRuntimeState,
        success: bool,
        latency_ms: Option<f64>,
    ) {
        runtime.success_ema = runtime.success_ema * 0.9 + if success { 0.1 } else { 0.0 };
        if let Some(latency_ms) = latency_ms.filter(|value| value.is_finite() && *value >= 0.0) {
            runtime.latency_ema_ms = Some(match runtime.latency_ema_ms {
                Some(previous) => previous * 0.8 + latency_ms * 0.2,
                None => latency_ms,
            });
        }
    }

    pub(crate) fn record_attempt_v1(
        runtime: &mut ForwardProxyRuntimeState,
        success: bool,
        is_probe: bool,
    ) {
        if success {
            runtime.consecutive_failures = 0;
            let latency_penalty = runtime
                .latency_ema_ms
                .map(|value| (value / 2500.0).min(0.6))
                .unwrap_or(0.0);
            runtime.weight += FORWARD_PROXY_WEIGHT_SUCCESS_BONUS - latency_penalty;
            if is_probe && runtime.weight <= 0.0 {
                runtime.weight = FORWARD_PROXY_PROBE_RECOVERY_WEIGHT;
            }
        } else {
            runtime.consecutive_failures = runtime.consecutive_failures.saturating_add(1);
            let failure_penalty = FORWARD_PROXY_WEIGHT_FAILURE_PENALTY_BASE
                + f64::from(runtime.consecutive_failures.saturating_sub(1))
                    * FORWARD_PROXY_WEIGHT_FAILURE_PENALTY_STEP;
            runtime.weight -= failure_penalty;
        }

        runtime.weight = runtime
            .weight
            .clamp(FORWARD_PROXY_WEIGHT_MIN, FORWARD_PROXY_WEIGHT_MAX);

        if success && runtime.weight < FORWARD_PROXY_WEIGHT_RECOVERY {
            runtime.weight = runtime.weight.max(FORWARD_PROXY_WEIGHT_RECOVERY * 0.5);
        }
    }

    pub(crate) fn record_attempt_v2(
        runtime: &mut ForwardProxyRuntimeState,
        success: bool,
        is_probe: bool,
    ) {
        if success {
            runtime.consecutive_failures = 0;
            let latency_penalty = runtime
                .latency_ema_ms
                .map(|value| {
                    (value / FORWARD_PROXY_V2_WEIGHT_SUCCESS_LATENCY_DIVISOR)
                        .min(FORWARD_PROXY_V2_WEIGHT_SUCCESS_LATENCY_CAP)
                })
                .unwrap_or(0.0);
            let success_gain = (FORWARD_PROXY_V2_WEIGHT_SUCCESS_BASE - latency_penalty)
                .max(FORWARD_PROXY_V2_WEIGHT_SUCCESS_MIN_GAIN);
            runtime.weight += success_gain;
            if is_probe && runtime.weight <= 0.0 {
                runtime.weight = FORWARD_PROXY_V2_PROBE_RECOVERY_WEIGHT;
            }
        } else {
            runtime.consecutive_failures = runtime.consecutive_failures.saturating_add(1);
            let failure_penalty = (FORWARD_PROXY_V2_WEIGHT_FAILURE_BASE
                + f64::from(runtime.consecutive_failures) * FORWARD_PROXY_V2_WEIGHT_FAILURE_STEP)
                .min(FORWARD_PROXY_V2_WEIGHT_FAILURE_MAX);
            runtime.weight -= failure_penalty;
        }

        runtime.weight = runtime
            .weight
            .clamp(FORWARD_PROXY_V2_WEIGHT_MIN, FORWARD_PROXY_V2_WEIGHT_MAX);

        if success && runtime.weight < FORWARD_PROXY_V2_WEIGHT_RECOVERY_FLOOR {
            runtime.weight = FORWARD_PROXY_V2_WEIGHT_RECOVERY_FLOOR;
        }
    }

    pub(crate) fn should_probe_penalized_proxy(&self) -> bool {
        let selectable_keys = self.selectable_endpoint_keys();
        if selectable_keys.is_empty() {
            return false;
        }
        let has_penalized = self.runtime.values().any(|entry| {
            selectable_keys.contains(entry.proxy_key.as_str()) && entry.is_penalized()
        });
        if !has_penalized || self.probe_in_flight {
            return false;
        }
        self.requests_since_probe >= self.algo.probe_every_requests()
            || (Utc::now() - self.last_probe_at).num_seconds() >= self.algo.probe_interval_secs()
    }

    pub(crate) fn mark_probe_started(&mut self) -> Option<SelectedForwardProxy> {
        if !self.should_probe_penalized_proxy() {
            return None;
        }
        let selectable_keys = self.selectable_endpoint_keys();
        let selected = self
            .runtime
            .values()
            .filter(|entry| {
                entry.is_penalized() && selectable_keys.contains(entry.proxy_key.as_str())
            })
            .max_by(|lhs, rhs| lhs.weight.total_cmp(&rhs.weight))
            .and_then(|entry| {
                self.endpoints
                    .iter()
                    .find(|item| item.key == entry.proxy_key)
            })
            .cloned()?;
        self.probe_in_flight = true;
        self.requests_since_probe = 0;
        self.last_probe_at = Utc::now();
        Some(SelectedForwardProxy::from_endpoint(&selected))
    }

    pub(crate) fn mark_probe_finished(&mut self) {
        self.probe_in_flight = false;
        self.last_probe_at = Utc::now();
    }
}

