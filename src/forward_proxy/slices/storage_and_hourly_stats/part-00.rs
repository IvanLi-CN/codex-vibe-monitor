use super::*;

pub(crate) const FORWARD_PROXY_EGRESS_IP_PROVIDER: &str = "ipify";
pub(crate) const FORWARD_PROXY_EGRESS_IP_ENDPOINT: &str = "https://api.ipify.org?format=json";
pub(crate) const FORWARD_PROXY_EGRESS_IP_REFRESH_INTERVAL_SECS: i64 = 600;
pub(crate) const FORWARD_PROXY_EGRESS_IP_TIMEOUT_SECS: u64 = 5;
pub(crate) const FORWARD_PROXY_MANUAL_LATENCY_TEST_ROUNDS: usize = 5;
pub(crate) const FORWARD_PROXY_MANUAL_LATENCY_ROUND_TIMEOUT_SECS: u64 = 5;
pub(crate) const FORWARD_PROXY_MANUAL_LATENCY_SINGLE_TIMEOUT_SECS: u64 = 15;
pub(crate) const FORWARD_PROXY_MANUAL_LATENCY_TARGET_COUNT: usize = 3;
pub(crate) const FORWARD_PROXY_LATENCY_TARGET_EGRESS_IP: &str = "egressIp";
pub(crate) const FORWARD_PROXY_LATENCY_TARGET_OAUTH_UPSTREAM: &str = "oauthUpstream";
pub(crate) const FORWARD_PROXY_LATENCY_TARGET_CODEX_RESPONSES: &str = "codexResponses";

#[derive(Debug, FromRow)]
pub(crate) struct PoolUpstreamBindingWindowStatsRow {
    proxy_binding_key_snapshot: String,
    attempts: i64,
    success_count: i64,
    latency_sum_ms: Option<f64>,
    latency_sample_count: i64,
}

#[derive(Debug, FromRow)]
pub(crate) struct PoolUpstreamBindingHourlyStatsRow {
    proxy_binding_key_snapshot: String,
    bucket_start_epoch: i64,
    success_count: i64,
    failure_count: i64,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct PendingPoolUpstreamBindingAttemptRow {
    proxy_binding_key_snapshot: String,
    occurred_at: String,
    bucket_start_epoch: i64,
    is_success: i64,
    latency_ms: Option<f64>,
}

#[derive(Debug, FromRow)]
pub(crate) struct ForwardProxyWeightHourlyStatsRow {
    pub(crate) proxy_key: String,
    pub(crate) bucket_start_epoch: i64,
    pub(crate) sample_count: i64,
    pub(crate) min_weight: f64,
    pub(crate) max_weight: f64,
    pub(crate) avg_weight: f64,
    pub(crate) last_weight: f64,
    pub(crate) last_sample_epoch_us: i64,
}

#[derive(Debug, FromRow)]
pub(crate) struct ForwardProxyWeightLastBeforeRangeRow {
    pub(crate) proxy_key: String,
    pub(crate) last_weight: f64,
    pub(crate) last_sample_epoch_us: i64,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct ForwardProxyKeyAliasRow {
    pub(crate) proxy_key: String,
    pub(crate) endpoint_url: Option<String>,
}

pub(crate) const POOL_UPSTREAM_BINDING_BUCKET_START_EPOCH_SQL: &str = r#"
    ((CASE
        WHEN instr(occurred_at, 'T') > 0
            THEN CAST(strftime('%s', occurred_at) AS INTEGER)
        ELSE CAST(strftime('%s', occurred_at || '+08:00') AS INTEGER)
    END) / 3600) * 3600
"#;

pub(crate) const POOL_UPSTREAM_BINDING_SUCCESS_LATENCY_SQL: &str =
    "COALESCE(first_byte_latency_ms, connect_latency_ms, stream_latency_ms)";
pub(crate) const POOL_UPSTREAM_BINDING_HOURLY_BUCKET_SECONDS: i64 = 3600;
pub(crate) fn ceil_hour_epoch(epoch: i64) -> i64 {
    let floor = align_bucket_epoch(epoch, POOL_UPSTREAM_BINDING_HOURLY_BUCKET_SECONDS, 0);
    if floor < epoch {
        floor + POOL_UPSTREAM_BINDING_HOURLY_BUCKET_SECONDS
    } else {
        floor
    }
}

pub(crate) async fn load_forward_proxy_settings(
    pool: &Pool<Sqlite>,
) -> Result<ForwardProxySettings> {
    let row = sqlx::query_as::<_, ForwardProxySettingsRow>(
        r#"
        SELECT
            proxy_urls_json,
            subscription_urls_json,
            subscription_update_interval_secs
        FROM forward_proxy_settings
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(FORWARD_PROXY_SETTINGS_SINGLETON_ID)
    .fetch_optional(pool)
    .await
    .context("failed to load forward_proxy_settings row")?;

    Ok(row
        .map(Into::into)
        .unwrap_or_else(ForwardProxySettings::default))
}

pub(crate) async fn save_forward_proxy_settings(
    pool: &Pool<Sqlite>,
    settings: ForwardProxySettings,
) -> Result<()> {
    let normalized = settings.normalized();
    let proxy_urls_json = serde_json::to_string(&normalized.proxy_urls)
        .context("failed to serialize forward proxy urls")?;
    let subscription_urls_json = serde_json::to_string(&normalized.subscription_urls)
        .context("failed to serialize forward proxy subscription urls")?;

    sqlx::query(
        r#"
        UPDATE forward_proxy_settings
        SET
            proxy_urls_json = ?1,
            subscription_urls_json = ?2,
            subscription_update_interval_secs = ?3,
            updated_at = datetime('now')
        WHERE id = ?4
        "#,
    )
    .bind(proxy_urls_json)
    .bind(subscription_urls_json)
    .bind(normalized.subscription_update_interval_secs as i64)
    .bind(FORWARD_PROXY_SETTINGS_SINGLETON_ID)
    .execute(pool)
    .await
    .context("failed to persist forward_proxy_settings row")?;

    Ok(())
}

pub(crate) async fn load_forward_proxy_runtime_states(
    pool: &Pool<Sqlite>,
) -> Result<Vec<ForwardProxyRuntimeState>> {
    let rows = sqlx::query_as::<_, ForwardProxyRuntimeRow>(
        r#"
        SELECT
            proxy_key,
            display_name,
            source,
            endpoint_url,
            weight,
            success_ema,
            latency_ema_ms,
            consecutive_failures
        FROM forward_proxy_runtime
        ORDER BY updated_at DESC
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to load forward_proxy_runtime rows")?;
    let alias_map = load_forward_proxy_key_aliases(pool).await?;

    let mut runtime = HashMap::new();
    for row in rows {
        let mut state: ForwardProxyRuntimeState = row.into();
        let canonical_proxy_key =
            canonical_forward_proxy_storage_key(&state.proxy_key, state.endpoint_url.as_deref());
        state.proxy_key = alias_map
            .get(&state.proxy_key)
            .or_else(|| alias_map.get(&canonical_proxy_key))
            .cloned()
            .unwrap_or(canonical_proxy_key);
        runtime.entry(state.proxy_key.clone()).or_insert(state);
    }
    Ok(runtime.into_values().collect())
}

pub(crate) async fn persist_forward_proxy_runtime_state(
    pool: &Pool<Sqlite>,
    state: &ForwardProxyRuntimeState,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO forward_proxy_runtime (
            proxy_key,
            display_name,
            source,
            endpoint_url,
            weight,
            success_ema,
            latency_ema_ms,
            consecutive_failures,
            is_penalized,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))
        ON CONFLICT(proxy_key) DO UPDATE SET
            display_name = excluded.display_name,
            source = excluded.source,
            endpoint_url = excluded.endpoint_url,
            weight = excluded.weight,
            success_ema = excluded.success_ema,
            latency_ema_ms = excluded.latency_ema_ms,
            consecutive_failures = excluded.consecutive_failures,
            is_penalized = excluded.is_penalized,
            updated_at = datetime('now')
        "#,
    )
    .bind(&state.proxy_key)
    .bind(&state.display_name)
    .bind(&state.source)
    .bind(&state.endpoint_url)
    .bind(state.weight)
    .bind(state.success_ema)
    .bind(state.latency_ema_ms)
    .bind(i64::from(state.consecutive_failures))
    .bind(state.is_penalized() as i64)
    .execute(pool)
    .await
    .with_context(|| {
        format!(
            "failed to persist forward_proxy_runtime row {}",
            state.proxy_key
        )
    })?;

    sqlx::query(
        r#"
        INSERT INTO forward_proxy_metadata_history (
            proxy_key,
            display_name,
            source,
            endpoint_url,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, datetime('now'))
        ON CONFLICT(proxy_key) DO UPDATE SET
            display_name = excluded.display_name,
            source = excluded.source,
            endpoint_url = excluded.endpoint_url,
            updated_at = datetime('now')
        "#,
    )
    .bind(&state.proxy_key)
    .bind(&state.display_name)
    .bind(&state.source)
    .bind(&state.endpoint_url)
    .execute(pool)
    .await
    .with_context(|| {
        format!(
            "failed to persist forward_proxy_metadata_history row {}",
            state.proxy_key
        )
    })?;
    Ok(())
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct ForwardProxyMetadataHistoryRow {
    pub(crate) proxy_key: String,
    pub(crate) display_name: String,
    pub(crate) source: String,
    pub(crate) endpoint_url: Option<String>,
    pub(crate) egress_ip: Option<String>,
    pub(crate) egress_ip_provider: Option<String>,
    pub(crate) egress_ip_checked_at: Option<String>,
    pub(crate) egress_ip_error: Option<String>,
    pub(crate) egress_ip_error_at: Option<String>,
}

pub(crate) async fn load_forward_proxy_metadata_history(
    pool: &Pool<Sqlite>,
    proxy_keys: &[String],
) -> Result<HashMap<String, ForwardProxyMetadataHistoryRow>> {
    if proxy_keys.is_empty() {
        return Ok(HashMap::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT proxy_key, display_name, source, endpoint_url, \
            egress_ip, egress_ip_provider, egress_ip_checked_at, egress_ip_error, egress_ip_error_at \
         FROM forward_proxy_metadata_history \
         WHERE proxy_key IN (",
    );
    {
        let mut separated = query.separated(", ");
        for key in proxy_keys {
            separated.push_bind(key);
        }
    }
    query.push(")");

    let rows = match query
        .build_query_as::<ForwardProxyMetadataHistoryRow>()
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(err) if is_missing_forward_proxy_metadata_history_table(&err) => {
            return Ok(HashMap::new());
        }
        Err(err) => {
            return Err(err).context("failed to load forward_proxy metadata history rows");
        }
    };
    Ok(rows
        .into_iter()
        .map(|row| (row.proxy_key.clone(), row))
        .collect())
}

pub(crate) fn forward_proxy_egress_ip_is_fresh(checked_at: Option<&str>) -> bool {
    checked_at
        .and_then(|raw| {
            DateTime::parse_from_rfc3339(raw)
                .ok()
                .map(|value| value.with_timezone(&Utc))
        })
        .is_some_and(|checked_at| {
            (Utc::now() - checked_at).num_seconds() < FORWARD_PROXY_EGRESS_IP_REFRESH_INTERVAL_SECS
        })
}

pub(crate) async fn persist_forward_proxy_egress_ip_result(
    pool: &Pool<Sqlite>,
    selected_proxy: &SelectedForwardProxy,
    egress_ip: Option<&str>,
    error: Option<&str>,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO forward_proxy_metadata_history (
            proxy_key,
            display_name,
            source,
            endpoint_url,
            egress_ip,
            egress_ip_provider,
            egress_ip_checked_at,
            egress_ip_error,
            egress_ip_error_at,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?7)
        ON CONFLICT(proxy_key) DO UPDATE SET
            display_name = excluded.display_name,
            source = excluded.source,
            endpoint_url = excluded.endpoint_url,
            egress_ip = COALESCE(excluded.egress_ip, forward_proxy_metadata_history.egress_ip),
            egress_ip_provider = excluded.egress_ip_provider,
            egress_ip_checked_at = excluded.egress_ip_checked_at,
            egress_ip_error = excluded.egress_ip_error,
            egress_ip_error_at = excluded.egress_ip_error_at,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(&selected_proxy.key)
    .bind(&selected_proxy.display_name)
    .bind(&selected_proxy.source)
    .bind(
        selected_proxy
            .endpoint_url_raw
            .as_deref()
            .or_else(|| selected_proxy.endpoint_url.as_ref().map(|url| url.as_str())),
    )
    .bind(egress_ip)
    .bind(FORWARD_PROXY_EGRESS_IP_PROVIDER)
    .bind(&now_iso)
    .bind(error)
    .bind(error.map(|_| now_iso.as_str()))
    .execute(pool)
    .await
    .with_context(|| {
        format!(
            "failed to persist forward proxy egress IP metadata for {}",
            selected_proxy.key
        )
    })?;
    Ok(())
}

pub(crate) async fn fetch_forward_proxy_egress_ip(
    client: &Client,
    request_timeout: Duration,
) -> Result<String> {
    let response = timeout(
        request_timeout,
        client.get(FORWARD_PROXY_EGRESS_IP_ENDPOINT).send(),
    )
    .await
    .map_err(|_| anyhow!("egress IP metadata request timed out"))?
    .context("failed to request egress IP metadata")?;
    if !response.status().is_success() {
        bail!("egress IP metadata endpoint returned {}", response.status());
    }
    let body = timeout(request_timeout, response.text())
        .await
        .map_err(|_| anyhow!("egress IP metadata body read timed out"))?
        .context("failed to read egress IP metadata body")?;
    let value: Value = serde_json::from_str(&body).context("failed to decode egress IP JSON")?;
    let ip = value
        .get("ip")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("egress IP metadata response did not include ip"))?;
    ip.parse::<std::net::IpAddr>()
        .with_context(|| format!("invalid egress IP metadata value: {ip}"))?;
    Ok(ip.to_string())
}

pub(crate) async fn refresh_forward_proxy_egress_ip_if_stale(
    state: &AppState,
    selected_proxy: &SelectedForwardProxy,
) -> Result<Option<String>> {
    let existing =
        load_forward_proxy_metadata_history(&state.pool, std::slice::from_ref(&selected_proxy.key))
            .await?
            .remove(&selected_proxy.key);
    if let Some(row) = existing.as_ref()
        && forward_proxy_egress_ip_is_fresh(row.egress_ip_checked_at.as_deref())
    {
        return Ok(row.egress_ip.clone());
    }

    let client = state
        .http_clients
        .client_for_forward_proxy(selected_proxy.endpoint_url.as_ref())
        .context("failed to initialize egress IP metadata client")?;
    let result = fetch_forward_proxy_egress_ip(
        &client,
        Duration::from_secs(FORWARD_PROXY_EGRESS_IP_TIMEOUT_SECS),
    )
    .await;
    match result {
        Ok(egress_ip) => {
            persist_forward_proxy_egress_ip_result(
                &state.pool,
                selected_proxy,
                Some(&egress_ip),
                None,
            )
            .await?;
            Ok(Some(egress_ip))
        }
        Err(err) => {
            let previous_ip = existing.and_then(|row| row.egress_ip);
            persist_forward_proxy_egress_ip_result(
                &state.pool,
                selected_proxy,
                None,
                Some(&err.to_string()),
            )
            .await?;
            Ok(previous_ip)
        }
    }
}

pub(crate) async fn load_forward_proxy_egress_ip_snapshot(
    state: &AppState,
    selected_proxy: &SelectedForwardProxy,
) -> Result<Option<String>> {
    Ok(
        load_forward_proxy_metadata_history(&state.pool, std::slice::from_ref(&selected_proxy.key))
            .await?
            .remove(&selected_proxy.key)
            .and_then(|row| row.egress_ip),
    )
}

pub(crate) fn is_missing_forward_proxy_metadata_history_table(err: &sqlx::Error) -> bool {
    let sqlx::Error::Database(db_err) = err else {
        return false;
    };
    let message = db_err.message().to_ascii_lowercase();
    message.contains("no such table") && message.contains("forward_proxy_metadata_history")
}

pub(crate) fn register_forward_proxy_storage_aliases(
    alias_map: &mut HashMap<String, String>,
    raw: &str,
) {
    let Some((canonical, aliases)) = forward_proxy_storage_aliases(raw) else {
        return;
    };
    for alias in aliases {
        alias_map.entry(alias).or_insert_with(|| canonical.clone());
    }
}

pub(crate) fn canonical_forward_proxy_storage_key(
    proxy_key: &str,
    endpoint_url: Option<&str>,
) -> String {
    endpoint_url
        .and_then(normalize_single_proxy_key)
        .or_else(|| normalize_bound_proxy_key(proxy_key))
        .unwrap_or_else(|| proxy_key.to_string())
}

pub(crate) async fn load_forward_proxy_key_aliases(
    pool: &Pool<Sqlite>,
) -> Result<HashMap<String, String>> {
    let settings = load_forward_proxy_settings(pool).await?;
    let rows = sqlx::query_as::<_, ForwardProxyKeyAliasRow>(
        r#"
        SELECT proxy_key, endpoint_url
        FROM forward_proxy_metadata_history
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to load forward_proxy key aliases")?;

    let mut alias_map = HashMap::new();
    for raw in settings.proxy_urls {
        register_forward_proxy_storage_aliases(&mut alias_map, &raw);
    }
    for row in rows {
        let canonical =
            canonical_forward_proxy_storage_key(&row.proxy_key, row.endpoint_url.as_deref());
        if canonical != row.proxy_key {
            alias_map
                .entry(row.proxy_key.clone())
                .or_insert(canonical.clone());
        }
        if let Some(raw) = row.endpoint_url.as_deref() {
            register_forward_proxy_storage_aliases(&mut alias_map, raw);
        }
    }
    Ok(alias_map)
}

pub(crate) async fn delete_forward_proxy_runtime_rows_not_in(
    pool: &Pool<Sqlite>,
    active_keys: &[String],
) -> Result<()> {
    if active_keys.is_empty() {
        sqlx::query("DELETE FROM forward_proxy_runtime")
            .execute(pool)
            .await
            .context("failed to clear forward_proxy_runtime rows")?;
        return Ok(());
    }
    let mut builder =
        QueryBuilder::<Sqlite>::new("DELETE FROM forward_proxy_runtime WHERE proxy_key NOT IN (");
    {
        let mut separated = builder.separated(", ");
        for key in active_keys {
            separated.push_bind(key);
        }
    }
    builder.push(")");
    builder
        .build()
        .execute(pool)
        .await
        .context("failed to prune forward_proxy_runtime rows")?;
    Ok(())
}

pub(crate) async fn insert_forward_proxy_attempt(
    pool: &Pool<Sqlite>,
    proxy_key: &str,
    success: bool,
    latency_ms: Option<f64>,
    failure_kind: Option<&str>,
    is_probe: bool,
) -> Result<()> {
    let occurred_at = format_naive(Utc::now().naive_utc());
    let mut tx = pool.begin().await?;
    let insert = sqlx::query(
        r#"
        INSERT INTO forward_proxy_attempts (
            proxy_key,
            occurred_at,
            is_success,
            latency_ms,
            failure_kind,
            is_probe
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind(proxy_key)
    .bind(&occurred_at)
    .bind(success as i64)
    .bind(latency_ms)
    .bind(failure_kind)
    .bind(is_probe as i64)
    .execute(tx.as_mut())
    .await
    .with_context(|| format!("failed to insert forward proxy attempt for {proxy_key}"))?;
    let inserted_id = insert.last_insert_rowid();
    upsert_forward_proxy_attempt_hourly_rollups_tx(
        tx.as_mut(),
        &[ForwardProxyAttemptHourlySourceRecord {
            id: inserted_id,
            proxy_key: proxy_key.to_string(),
            occurred_at,
            is_success: success as i64,
            latency_ms,
        }],
    )
    .await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
        inserted_id,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn upsert_forward_proxy_weight_hourly_bucket(
    pool: &Pool<Sqlite>,
    proxy_key: &str,
    bucket_start_epoch: i64,
    weight: f64,
    sample_epoch_us: i64,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO forward_proxy_weight_hourly (
            proxy_key,
            bucket_start_epoch,
            sample_count,
            min_weight,
            max_weight,
            avg_weight,
            last_weight,
            last_sample_epoch_us,
            updated_at
        )
        VALUES (?1, ?2, 1, ?3, ?3, ?3, ?3, ?4, datetime('now'))
        ON CONFLICT(proxy_key, bucket_start_epoch) DO UPDATE SET
            sample_count = forward_proxy_weight_hourly.sample_count + 1,
            min_weight = MIN(forward_proxy_weight_hourly.min_weight, excluded.min_weight),
            max_weight = MAX(forward_proxy_weight_hourly.max_weight, excluded.max_weight),
            avg_weight = (
                (forward_proxy_weight_hourly.avg_weight * forward_proxy_weight_hourly.sample_count)
                + excluded.avg_weight
            ) / (forward_proxy_weight_hourly.sample_count + 1),
            last_weight = CASE
                WHEN excluded.last_sample_epoch_us >= forward_proxy_weight_hourly.last_sample_epoch_us
                    THEN excluded.last_weight
                ELSE forward_proxy_weight_hourly.last_weight
            END,
            last_sample_epoch_us = MAX(
                forward_proxy_weight_hourly.last_sample_epoch_us,
                excluded.last_sample_epoch_us
            ),
            updated_at = datetime('now')
        "#,
    )
    .bind(proxy_key)
    .bind(bucket_start_epoch)
    .bind(weight)
    .bind(sample_epoch_us)
    .execute(pool)
    .await
    .with_context(|| {
        format!(
            "failed to upsert forward proxy weight bucket for {proxy_key} at {bucket_start_epoch}"
        )
    })?;
    Ok(())
}

pub(crate) async fn load_pool_upstream_binding_key_canonical_map(
    state: &AppState,
    raw_keys: &[String],
) -> Result<HashMap<String, String>> {
    if raw_keys.is_empty() {
        return Ok(HashMap::new());
    }
    let metadata_map = load_forward_proxy_metadata_history(&state.pool, raw_keys).await?;
    let manager = state.forward_proxy.lock().await;
    Ok(raw_keys
        .iter()
        .map(|raw_key| {
            let canonical = manager
                .canonicalize_bound_proxy_key(raw_key, metadata_map.get(raw_key))
                .unwrap_or_else(|| raw_key.clone());
            (raw_key.clone(), canonical)
        })
        .collect())
}

pub(crate) fn resolve_pool_upstream_binding_target_key(
    raw_key: &str,
    canonical_map: &HashMap<String, String>,
    target_keys: Option<&HashSet<String>>,
) -> Option<String> {
    let canonical = canonical_map
        .get(raw_key)
        .cloned()
        .unwrap_or_else(|| raw_key.to_string());
    match target_keys {
        Some(keys) if keys.contains(&canonical) => Some(canonical),
        Some(keys) if keys.contains(raw_key) => Some(raw_key.to_string()),
        Some(_) => None,
        None => Some(canonical),
    }
}

pub(crate) fn record_pool_upstream_binding_window_stats(
    grouped: &mut HashMap<String, ForwardProxyAttemptWindowStats>,
    latency_totals: &mut HashMap<String, f64>,
    latency_samples: &mut HashMap<String, i64>,
    proxy_key: String,
    attempts: i64,
    success_count: i64,
    latency_sum_ms: Option<f64>,
    latency_sample_count: i64,
) {
    let stats = grouped.entry(proxy_key.clone()).or_default();
    stats.attempts += attempts;
    stats.success_count += success_count;
    *latency_totals.entry(proxy_key.clone()).or_insert(0.0) += latency_sum_ms.unwrap_or(0.0);
    *latency_samples.entry(proxy_key).or_insert(0) += latency_sample_count;
}

pub(crate) fn record_pool_upstream_binding_hourly_stats(
    grouped: &mut HashMap<String, HashMap<i64, ForwardProxyHourlyStatsPoint>>,
    proxy_key: String,
    bucket_start_epoch: i64,
    success_count: i64,
    failure_count: i64,
) {
    let point = grouped
        .entry(proxy_key)
        .or_default()
        .entry(bucket_start_epoch)
        .or_default();
    point.success_count += success_count;
    point.failure_count += failure_count;
}

pub(crate) fn ensure_owner_facing_direct_runtime_row(
    runtime_rows: &mut Vec<ForwardProxyRuntimeState>,
    algo: ForwardProxyAlgo,
    insert_direct: bool,
) {
    if !insert_direct
        || runtime_rows
            .iter()
            .any(|runtime| runtime.proxy_key == FORWARD_PROXY_DIRECT_KEY)
    {
        return;
    }

    runtime_rows.push(ForwardProxyRuntimeState {
        proxy_key: FORWARD_PROXY_DIRECT_KEY.to_string(),
        display_name: FORWARD_PROXY_DIRECT_LABEL.to_string(),
        source: FORWARD_PROXY_SOURCE_DIRECT.to_string(),
        endpoint_url: None,
        weight: match algo {
            ForwardProxyAlgo::V1 => 1.0,
            ForwardProxyAlgo::V2 => FORWARD_PROXY_V2_DIRECT_INITIAL_WEIGHT,
        },
        success_ema: 0.65,
        latency_ema_ms: None,
        consecutive_failures: 0,
    });
}

pub(crate) fn owner_facing_pool_upstream_pending_archive_temp_path(archive_path: &Path) -> PathBuf {
    PathBuf::from(format!(
        "{}.owner-facing-node-health.{}.sqlite",
        archive_path.display(),
        retention_temp_suffix()
    ))
}

pub(crate) async fn load_pending_pool_upstream_node_health_archive_file_paths(
    pool: &Pool<Sqlite>,
    start_at: &str,
    end_at: &str,
) -> Result<Vec<String>> {
    Ok(
        load_pending_pool_upstream_node_health_archive_files(pool, Some(start_at), Some(end_at))
            .await?
            .into_iter()
            .map(|row| row.file_path)
            .collect(),
    )
}

pub(crate) async fn inflate_pending_pool_upstream_node_health_archive_to_temp(
    archive_path: &Path,
    temp_path: &Path,
) -> Result<()> {
    let archive_path = archive_path.to_path_buf();
    let temp_path = temp_path.to_path_buf();
    tokio::task::spawn_blocking(move || inflate_gzip_sqlite_file(&archive_path, &temp_path))
        .await
        .context("pending pool upstream node health archive inflate task panicked")??;
    Ok(())
}
