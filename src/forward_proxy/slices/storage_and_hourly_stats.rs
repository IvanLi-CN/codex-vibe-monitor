use crate::stats::*;
use crate::*;

const FORWARD_PROXY_EGRESS_IP_PROVIDER: &str = "ipify";
const FORWARD_PROXY_EGRESS_IP_ENDPOINT: &str = "https://api.ipify.org?format=json";
const FORWARD_PROXY_EGRESS_IP_REFRESH_INTERVAL_SECS: i64 = 600;
const FORWARD_PROXY_EGRESS_IP_TIMEOUT_SECS: u64 = 5;

#[derive(Debug, FromRow)]
struct PoolUpstreamBindingWindowStatsRow {
    proxy_binding_key_snapshot: String,
    attempts: i64,
    success_count: i64,
    latency_sum_ms: Option<f64>,
    latency_sample_count: i64,
}

#[derive(Debug, FromRow)]
struct PoolUpstreamBindingHourlyStatsRow {
    proxy_binding_key_snapshot: String,
    bucket_start_epoch: i64,
    success_count: i64,
    failure_count: i64,
}

#[derive(Debug, Clone, FromRow)]
struct PendingPoolUpstreamBindingAttemptRow {
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

const POOL_UPSTREAM_BINDING_BUCKET_START_EPOCH_SQL: &str = r#"
    ((CASE
        WHEN instr(occurred_at, 'T') > 0
            THEN CAST(strftime('%s', occurred_at) AS INTEGER)
        ELSE CAST(strftime('%s', occurred_at || '+08:00') AS INTEGER)
    END) / 3600) * 3600
"#;

const POOL_UPSTREAM_BINDING_SUCCESS_LATENCY_SQL: &str =
    "COALESCE(first_byte_latency_ms, connect_latency_ms, stream_latency_ms)";
const POOL_UPSTREAM_BINDING_HOURLY_BUCKET_SECONDS: i64 = 3600;
fn ceil_hour_epoch(epoch: i64) -> i64 {
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

fn forward_proxy_egress_ip_is_fresh(checked_at: Option<&str>) -> bool {
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

async fn persist_forward_proxy_egress_ip_result(
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
    .bind(selected_proxy.endpoint_url_raw.as_deref().or_else(|| {
        selected_proxy
            .endpoint_url
            .as_ref()
            .map(|url| url.as_str())
    }))
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

async fn fetch_forward_proxy_egress_ip(
    client: &Client,
    request_timeout: Duration,
) -> Result<String> {
    let response = timeout(request_timeout, client.get(FORWARD_PROXY_EGRESS_IP_ENDPOINT).send())
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
    let existing = load_forward_proxy_metadata_history(&state.pool, std::slice::from_ref(&selected_proxy.key))
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
    Ok(load_forward_proxy_metadata_history(&state.pool, std::slice::from_ref(&selected_proxy.key))
        .await?
        .remove(&selected_proxy.key)
        .and_then(|row| row.egress_ip))
}

fn is_missing_forward_proxy_metadata_history_table(err: &sqlx::Error) -> bool {
    let sqlx::Error::Database(db_err) = err else {
        return false;
    };
    let message = db_err.message().to_ascii_lowercase();
    message.contains("no such table") && message.contains("forward_proxy_metadata_history")
}

fn register_forward_proxy_storage_aliases(alias_map: &mut HashMap<String, String>, raw: &str) {
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

async fn load_pool_upstream_binding_key_canonical_map(
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

fn resolve_pool_upstream_binding_target_key(
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

fn record_pool_upstream_binding_window_stats(
    grouped: &mut HashMap<String, ForwardProxyAttemptWindowStats>,
    latency_totals: &mut HashMap<String, f64>,
    latency_samples: &mut HashMap<String, i64>,
    proxy_key: String,
    attempts: i64,
    success_count: i64,
    latency_sum_ms: Option<f64>,
    latency_sample_count: i64,
) {
    let stats = grouped
        .entry(proxy_key.clone())
        .or_insert_with(ForwardProxyAttemptWindowStats::default);
    stats.attempts += attempts;
    stats.success_count += success_count;
    *latency_totals.entry(proxy_key.clone()).or_insert(0.0) += latency_sum_ms.unwrap_or(0.0);
    *latency_samples.entry(proxy_key).or_insert(0) += latency_sample_count;
}

fn record_pool_upstream_binding_hourly_stats(
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

fn ensure_owner_facing_direct_runtime_row(
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

fn owner_facing_pool_upstream_pending_archive_temp_path(archive_path: &Path) -> PathBuf {
    PathBuf::from(format!(
        "{}.owner-facing-node-health.{}.sqlite",
        archive_path.display(),
        retention_temp_suffix()
    ))
}

async fn load_pending_pool_upstream_node_health_archive_file_paths(
    pool: &Pool<Sqlite>,
    start_at: &str,
    end_at: &str,
) -> Result<Vec<String>> {
    Ok(load_pending_pool_upstream_node_health_archive_files(pool, Some(start_at), Some(end_at))
        .await?
        .into_iter()
        .map(|row| row.file_path)
        .collect())
}

async fn inflate_pending_pool_upstream_node_health_archive_to_temp(
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

async fn load_pending_pool_upstream_binding_attempt_rows_from_archive_file(
    archive_file_path: &str,
    start_at: &str,
    end_at: &str,
) -> Result<Option<Vec<PendingPoolUpstreamBindingAttemptRow>>> {
    let archive_path = PathBuf::from(archive_file_path);
    if !archive_path.exists() {
        warn!(
            file_path = archive_file_path,
            "skipping missing pending pool upstream node health archive while serving owner-facing node health"
        );
        return Ok(None);
    }

    let temp_path = owner_facing_pool_upstream_pending_archive_temp_path(&archive_path);
    let temp_cleanup = TempSqliteCleanup(temp_path.clone());
    let query_result = async {
        inflate_pending_pool_upstream_node_health_archive_to_temp(&archive_path, &temp_path)
            .await?;
        let mut conn = SqliteConnection::connect(&sqlite_url_for_path(&temp_path))
            .await
            .with_context(|| format!("failed to open archive batch {}", archive_path.display()))?;
        ensure_pool_upstream_request_attempts_archive_schema_in_place(&mut conn).await?;
        let attempts_sql = format!(
            r#"
            SELECT
                proxy_binding_key_snapshot,
                occurred_at,
                {bucket_start_epoch_sql} AS bucket_start_epoch,
                CASE WHEN status = '{success}' THEN 1 ELSE 0 END AS is_success,
                CASE WHEN status = '{success}' THEN {latency_sql} ELSE NULL END AS latency_ms
            FROM pool_upstream_request_attempts
            WHERE proxy_binding_key_snapshot IS NOT NULL
              AND finished_at IS NOT NULL
              AND status != '{budget_exhausted}'
              AND occurred_at >= ?1
              AND occurred_at < ?2
            "#,
            bucket_start_epoch_sql = POOL_UPSTREAM_BINDING_BUCKET_START_EPOCH_SQL,
            success = POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
            budget_exhausted = POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
            latency_sql = POOL_UPSTREAM_BINDING_SUCCESS_LATENCY_SQL,
        );
        let rows = sqlx::query_as::<_, PendingPoolUpstreamBindingAttemptRow>(&attempts_sql)
            .bind(start_at)
            .bind(end_at)
            .fetch_all(&mut conn)
            .await
            .with_context(|| {
                format!(
                    "failed to load pending archive node health attempts from {} within [{start_at}, {end_at})",
                    archive_path.display()
                )
            })?;
        let _ = conn.close().await;
        Ok::<_, anyhow::Error>(rows)
    }
    .await;
    drop(temp_cleanup);

    match query_result {
        Ok(rows) => Ok(Some(rows)),
        Err(err) => {
            warn!(
                file_path = archive_file_path,
                error = %err,
                "skipping unreadable pending pool upstream node health archive while serving owner-facing node health"
            );
            Ok(None)
        }
    }
}

async fn load_pending_pool_upstream_binding_attempt_rows(
    pending_archive_file_paths: &[String],
    start_at: &str,
    end_at: &str,
) -> Result<Vec<PendingPoolUpstreamBindingAttemptRow>> {
    let mut rows = Vec::new();
    for archive_file_path in pending_archive_file_paths {
        let Some(mut archive_rows) =
            load_pending_pool_upstream_binding_attempt_rows_from_archive_file(
                archive_file_path,
                start_at,
                end_at,
            )
            .await?
        else {
            continue;
        };
        rows.append(&mut archive_rows);
    }
    Ok(rows)
}

fn aggregate_pending_pool_upstream_binding_window_stats(
    pending_archive_rows: &[PendingPoolUpstreamBindingAttemptRow],
    start_at: &str,
    end_at: &str,
) -> Vec<PoolUpstreamBindingWindowStatsRow> {
    let mut grouped = HashMap::<String, PoolUpstreamBindingWindowStatsRow>::new();
    for row in pending_archive_rows {
        if row.occurred_at.as_str() < start_at || row.occurred_at.as_str() >= end_at {
            continue;
        }
        let entry = grouped
            .entry(row.proxy_binding_key_snapshot.clone())
            .or_insert_with(|| PoolUpstreamBindingWindowStatsRow {
                proxy_binding_key_snapshot: row.proxy_binding_key_snapshot.clone(),
                attempts: 0,
                success_count: 0,
                latency_sum_ms: None,
                latency_sample_count: 0,
            });
        entry.attempts += 1;
        entry.success_count += row.is_success;
        if let Some(latency_ms) = row.latency_ms {
            entry.latency_sum_ms = Some(entry.latency_sum_ms.unwrap_or_default() + latency_ms);
            entry.latency_sample_count += 1;
        }
    }
    grouped.into_values().collect()
}

async fn query_pending_pool_upstream_binding_window_stats(
    pending_archive_file_paths: &[String],
    start_at: &str,
    end_at: &str,
) -> Result<Vec<PoolUpstreamBindingWindowStatsRow>> {
    let rows =
        load_pending_pool_upstream_binding_attempt_rows(pending_archive_file_paths, start_at, end_at)
            .await?;
    Ok(aggregate_pending_pool_upstream_binding_window_stats(
        &rows, start_at, end_at,
    ))
}

fn aggregate_pending_pool_upstream_binding_hourly_stats(
    pending_archive_rows: &[PendingPoolUpstreamBindingAttemptRow],
    range_start_epoch: i64,
    range_end_epoch: i64,
) -> Vec<PoolUpstreamBindingHourlyStatsRow> {
    let mut grouped = HashMap::<(String, i64), PoolUpstreamBindingHourlyStatsRow>::new();
    for row in pending_archive_rows {
        if !(range_start_epoch..range_end_epoch).contains(&row.bucket_start_epoch) {
            continue;
        }
        let entry = grouped
            .entry((
                row.proxy_binding_key_snapshot.clone(),
                row.bucket_start_epoch,
            ))
            .or_insert_with(|| PoolUpstreamBindingHourlyStatsRow {
                proxy_binding_key_snapshot: row.proxy_binding_key_snapshot.clone(),
                bucket_start_epoch: row.bucket_start_epoch,
                success_count: 0,
                failure_count: 0,
            });
        entry.success_count += row.is_success;
        entry.failure_count += if row.is_success == 0 { 1 } else { 0 };
    }
    grouped.into_values().collect()
}

async fn query_pending_pool_upstream_binding_hourly_stats(
    pending_archive_file_paths: &[String],
    range_start_at: &str,
    range_end_at: &str,
    range_start_epoch: i64,
    range_end_epoch: i64,
) -> Result<Vec<PoolUpstreamBindingHourlyStatsRow>> {
    let rows = load_pending_pool_upstream_binding_attempt_rows(
        pending_archive_file_paths,
        range_start_at,
        range_end_at,
    )
    .await?;
    Ok(aggregate_pending_pool_upstream_binding_hourly_stats(
        &rows,
        range_start_epoch,
        range_end_epoch,
    ))
}

async fn query_materialized_pool_upstream_binding_hourly_stats(
    pool: &Pool<Sqlite>,
    range_start_epoch: i64,
    range_end_epoch: i64,
) -> Result<Vec<PoolUpstreamBindingHourlyStatsRow>> {
    sqlx::query_as::<_, PoolUpstreamBindingHourlyStatsRow>(
        r#"
        SELECT
            hourly.proxy_binding_key_snapshot,
            hourly.bucket_start_epoch,
            SUM(hourly.success_count) AS success_count,
            SUM(hourly.failure_count) AS failure_count
        FROM pool_upstream_node_health_hourly_archive AS hourly
        WHERE hourly.bucket_start_epoch >= ?1
          AND hourly.bucket_start_epoch < ?2
          AND (
                hourly.archive_batch_id IS NULL
                OR NOT EXISTS (
                    SELECT 1
                    FROM archive_batches AS batches
                    WHERE batches.dataset = 'pool_upstream_request_attempts'
                      AND batches.status = ?3
                      AND batches.id = hourly.archive_batch_id
                )
          )
        GROUP BY hourly.proxy_binding_key_snapshot, hourly.bucket_start_epoch
        "#,
    )
    .bind(range_start_epoch)
    .bind(range_end_epoch)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(pool)
    .await
    .with_context(|| {
        format!(
            "failed to query materialized pool upstream hourly stats within [{range_start_epoch}, {range_end_epoch})"
        )
    })
}

async fn query_pool_upstream_binding_window_stats(
    state: &AppState,
    start_at: &str,
    end_at: &str,
    target_keys: Option<&HashSet<String>>,
    pending_archive_file_paths: &[String],
    pending_archive_rows: Option<&[PendingPoolUpstreamBindingAttemptRow]>,
) -> Result<HashMap<String, ForwardProxyAttemptWindowStats>> {
    let window_sql = format!(
        r#"
            SELECT
                proxy_binding_key_snapshot,
                COUNT(*) AS attempts,
                SUM(CASE WHEN status = '{success}' THEN 1 ELSE 0 END) AS success_count,
                SUM(CASE WHEN status = '{success}' THEN {latency_sql} END) AS latency_sum_ms,
                SUM(CASE
                        WHEN status = '{success}' AND {latency_sql} IS NOT NULL THEN 1
                        ELSE 0
                    END) AS latency_sample_count
            FROM pool_upstream_request_attempts
            WHERE proxy_binding_key_snapshot IS NOT NULL
              AND finished_at IS NOT NULL
          AND status != '{budget_exhausted}'
          AND occurred_at >= ?1
          AND occurred_at < ?2
        GROUP BY proxy_binding_key_snapshot
        "#,
        success = POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        budget_exhausted = POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
        latency_sql = POOL_UPSTREAM_BINDING_SUCCESS_LATENCY_SQL,
    );
    let mut rows = sqlx::query_as::<_, PoolUpstreamBindingWindowStatsRow>(&window_sql)
        .bind(start_at)
        .bind(end_at)
        .fetch_all(&state.pool)
        .await
        .with_context(|| {
            format!(
                "failed to query pool upstream binding window stats within [{start_at}, {end_at})"
            )
        })?;
    let mut archive_query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            proxy_binding_key_snapshot,
            COUNT(*) AS attempts,
            SUM(is_success) AS success_count,
            SUM(CASE WHEN is_success != 0 THEN latency_ms END) AS latency_sum_ms,
            SUM(CASE
                    WHEN is_success != 0 AND latency_ms IS NOT NULL THEN 1
                    ELSE 0
                END) AS latency_sample_count
        FROM pool_upstream_node_health_archive
        WHERE occurred_at >= "#,
    );
    archive_query.push_bind(start_at);
    archive_query.push(" AND occurred_at < ");
    archive_query.push_bind(end_at);
    if !pending_archive_file_paths.is_empty() {
        archive_query.push(" AND archive_file_path NOT IN (");
        {
            let mut separated = archive_query.separated(", ");
            for file_path in pending_archive_file_paths {
                separated.push_bind(file_path);
            }
        }
        archive_query.push(")");
    }
    archive_query.push(" GROUP BY proxy_binding_key_snapshot");
    rows.extend(
        archive_query
            .build_query_as::<PoolUpstreamBindingWindowStatsRow>()
            .fetch_all(&state.pool)
            .await
            .with_context(|| {
                format!(
                    "failed to query cached pool upstream archive window stats within [{start_at}, {end_at})"
                )
            })?,
    );
    if let Some(pending_archive_rows) = pending_archive_rows {
        rows.extend(aggregate_pending_pool_upstream_binding_window_stats(
            pending_archive_rows,
            start_at,
            end_at,
        ));
    } else {
        rows.extend(
            query_pending_pool_upstream_binding_window_stats(
                pending_archive_file_paths,
                start_at,
                end_at,
            )
            .await?,
        );
    }

    let raw_keys = rows
        .iter()
        .map(|row| row.proxy_binding_key_snapshot.clone())
        .collect::<Vec<_>>();
    let canonical_map = load_pool_upstream_binding_key_canonical_map(state, &raw_keys).await?;
    let mut grouped = HashMap::new();
    let mut latency_totals = HashMap::new();
    let mut latency_samples = HashMap::new();
    for row in rows {
        let Some(proxy_key) = resolve_pool_upstream_binding_target_key(
            &row.proxy_binding_key_snapshot,
            &canonical_map,
            target_keys,
        ) else {
            continue;
        };
        record_pool_upstream_binding_window_stats(
            &mut grouped,
            &mut latency_totals,
            &mut latency_samples,
            proxy_key,
            row.attempts,
            row.success_count,
            row.latency_sum_ms,
            row.latency_sample_count,
        );
    }

    for (proxy_key, stats) in &mut grouped {
        let latency_sample_count = latency_samples.get(proxy_key).copied().unwrap_or_default();
        if stats.success_count > 0 && latency_sample_count == stats.success_count {
            stats.avg_latency_ms = latency_totals
                .get(proxy_key)
                .copied()
                .map(|value| value / latency_sample_count as f64);
        }
    }

    Ok(grouped)
}

async fn query_pool_upstream_binding_hourly_stats(
    state: &AppState,
    range_start_epoch: i64,
    range_end_epoch: i64,
    target_keys: Option<&HashSet<String>>,
    pending_archive_file_paths: &[String],
    pending_archive_rows: Option<&[PendingPoolUpstreamBindingAttemptRow]>,
) -> Result<HashMap<String, HashMap<i64, ForwardProxyHourlyStatsPoint>>> {
    let range_start = Utc
        .timestamp_opt(range_start_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid pool upstream binding bucket range start epoch"))?;
    let range_end = Utc
        .timestamp_opt(range_end_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid pool upstream binding bucket range end epoch"))?;
    let range_start_at = db_occurred_at_lower_bound(range_start);
    let range_end_at = db_occurred_at_lower_bound(range_end);
    let hourly_sql = format!(
        r#"
        SELECT
            proxy_binding_key_snapshot,
            bucket_start_epoch,
            SUM(CASE WHEN status = '{success}' THEN 1 ELSE 0 END) AS success_count,
            SUM(CASE WHEN status != '{success}' THEN 1 ELSE 0 END) AS failure_count
        FROM (
            SELECT
                proxy_binding_key_snapshot,
                status,
                {bucket_start_epoch_sql} AS bucket_start_epoch
            FROM pool_upstream_request_attempts
            WHERE proxy_binding_key_snapshot IS NOT NULL
              AND finished_at IS NOT NULL
              AND status != '{budget_exhausted}'
              AND occurred_at >= ?1
              AND occurred_at < ?2
        )
        WHERE bucket_start_epoch >= ?3
          AND bucket_start_epoch < ?4
        GROUP BY proxy_binding_key_snapshot, bucket_start_epoch
        "#,
        bucket_start_epoch_sql = POOL_UPSTREAM_BINDING_BUCKET_START_EPOCH_SQL,
        success = POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        budget_exhausted = POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
    );
    let mut rows = sqlx::query_as::<_, PoolUpstreamBindingHourlyStatsRow>(&hourly_sql)
        .bind(&range_start_at)
        .bind(&range_end_at)
        .bind(range_start_epoch)
        .bind(range_end_epoch)
        .fetch_all(&state.pool)
        .await
        .with_context(|| {
            format!(
                "failed to query pool upstream binding hourly stats within [{range_start_epoch}, {range_end_epoch})"
            )
        })?;
    let mut archive_query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            proxy_binding_key_snapshot,
            bucket_start_epoch,
            SUM(is_success) AS success_count,
            SUM(CASE WHEN is_success = 0 THEN 1 ELSE 0 END) AS failure_count
        FROM (
            SELECT
                proxy_binding_key_snapshot,
                is_success,
                "#,
    );
    archive_query.push(POOL_UPSTREAM_BINDING_BUCKET_START_EPOCH_SQL);
    archive_query.push(
        r#" AS bucket_start_epoch
            FROM pool_upstream_node_health_archive
            WHERE occurred_at >= "#,
    );
    archive_query.push_bind(&range_start_at);
    archive_query.push(" AND occurred_at < ");
    archive_query.push_bind(&range_end_at);
    if !pending_archive_file_paths.is_empty() {
        archive_query.push(" AND archive_file_path NOT IN (");
        {
            let mut separated = archive_query.separated(", ");
            for file_path in pending_archive_file_paths {
                separated.push_bind(file_path);
            }
        }
        archive_query.push(")");
    }
    archive_query.push(
        r#"
        )
        WHERE bucket_start_epoch >= "#,
    );
    archive_query.push_bind(range_start_epoch);
    archive_query.push(" AND bucket_start_epoch < ");
    archive_query.push_bind(range_end_epoch);
    archive_query.push(" GROUP BY proxy_binding_key_snapshot, bucket_start_epoch");
    rows.extend(
        archive_query
            .build_query_as::<PoolUpstreamBindingHourlyStatsRow>()
            .fetch_all(&state.pool)
            .await
            .with_context(|| {
                format!(
                    "failed to query cached pool upstream archive hourly stats within [{range_start_epoch}, {range_end_epoch})"
                )
            })?,
    );
    if let Some(pending_archive_rows) = pending_archive_rows {
        rows.extend(aggregate_pending_pool_upstream_binding_hourly_stats(
            pending_archive_rows,
            range_start_epoch,
            range_end_epoch,
        ));
    } else {
        rows.extend(
            query_pending_pool_upstream_binding_hourly_stats(
                pending_archive_file_paths,
                &range_start_at,
                &range_end_at,
                range_start_epoch,
                range_end_epoch,
            )
            .await?,
        );
    }
    rows.extend(
        query_materialized_pool_upstream_binding_hourly_stats(
            &state.pool,
            range_start_epoch,
            range_end_epoch,
        )
        .await?,
    );

    let raw_keys = rows
        .iter()
        .map(|row| row.proxy_binding_key_snapshot.clone())
        .collect::<Vec<_>>();
    let canonical_map = load_pool_upstream_binding_key_canonical_map(state, &raw_keys).await?;
    let mut grouped = HashMap::new();
    for row in rows {
        let Some(proxy_key) = resolve_pool_upstream_binding_target_key(
            &row.proxy_binding_key_snapshot,
            &canonical_map,
            target_keys,
        ) else {
            continue;
        };
        if !(range_start_epoch..range_end_epoch).contains(&row.bucket_start_epoch) {
            continue;
        }
        record_pool_upstream_binding_hourly_stats(
            &mut grouped,
            proxy_key,
            row.bucket_start_epoch,
            row.success_count,
            row.failure_count,
        );
    }

    Ok(grouped)
}

pub(crate) async fn query_forward_proxy_weight_hourly_stats(
    pool: &Pool<Sqlite>,
    range_start_epoch: i64,
    range_end_epoch: i64,
) -> Result<HashMap<String, HashMap<i64, ForwardProxyWeightHourlyStatsPoint>>> {
    let rows = sqlx::query_as::<_, ForwardProxyWeightHourlyStatsRow>(
        r#"
        SELECT
            proxy_key,
            bucket_start_epoch,
            sample_count,
            min_weight,
            max_weight,
            avg_weight,
            last_weight,
            last_sample_epoch_us
        FROM forward_proxy_weight_hourly
        WHERE bucket_start_epoch >= ?1
          AND bucket_start_epoch < ?2
        "#,
    )
    .bind(range_start_epoch)
    .bind(range_end_epoch)
    .fetch_all(pool)
    .await
    .with_context(|| {
        format!(
            "failed to query forward proxy weight stats within [{range_start_epoch}, {range_end_epoch})"
        )
    })?;

    let alias_map = load_forward_proxy_key_aliases(pool).await?;
    let mut grouped: HashMap<String, HashMap<i64, ForwardProxyWeightHourlyStatsPoint>> =
        HashMap::new();
    let mut latest_sample_epochs: HashMap<(String, i64), i64> = HashMap::new();

    for row in rows {
        let proxy_key = alias_map
            .get(&row.proxy_key)
            .cloned()
            .unwrap_or(row.proxy_key.clone());
        let key = (proxy_key.clone(), row.bucket_start_epoch);
        let point = grouped
            .entry(proxy_key.clone())
            .or_default()
            .entry(row.bucket_start_epoch)
            .or_insert_with(|| ForwardProxyWeightHourlyStatsPoint {
                sample_count: 0,
                min_weight: row.min_weight,
                max_weight: row.max_weight,
                avg_weight: 0.0,
                last_weight: row.last_weight,
            });
        let combined_sample_count = point.sample_count + row.sample_count;
        point.avg_weight = if combined_sample_count > 0 {
            ((point.avg_weight * point.sample_count as f64)
                + (row.avg_weight * row.sample_count as f64))
                / combined_sample_count as f64
        } else {
            row.avg_weight
        };
        point.sample_count = combined_sample_count;
        point.min_weight = point.min_weight.min(row.min_weight);
        point.max_weight = point.max_weight.max(row.max_weight);

        let current_latest = latest_sample_epochs.get(&key).copied().unwrap_or(i64::MIN);
        if row.last_sample_epoch_us >= current_latest {
            point.last_weight = row.last_weight;
            latest_sample_epochs.insert(key, row.last_sample_epoch_us);
        }
    }

    Ok(grouped)
}

pub(crate) async fn query_forward_proxy_weight_last_before(
    pool: &Pool<Sqlite>,
    range_start_epoch: i64,
    proxy_keys: &[String],
) -> Result<HashMap<String, f64>> {
    if proxy_keys.is_empty() {
        return Ok(HashMap::new());
    }

    let mut builder = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT latest.proxy_key, latest.last_weight, latest.last_sample_epoch_us
        FROM forward_proxy_weight_hourly AS latest
        INNER JOIN (
            SELECT proxy_key, MAX(bucket_start_epoch) AS bucket_start_epoch
            FROM forward_proxy_weight_hourly
            WHERE bucket_start_epoch < "#,
    );
    builder.push_bind(range_start_epoch);
    builder.push(" AND proxy_key IN (");
    {
        let mut separated = builder.separated(", ");
        for key in proxy_keys {
            separated.push_bind(key);
        }
    }
    builder.push(
        r#")
            GROUP BY proxy_key
        ) AS prior
            ON latest.proxy_key = prior.proxy_key
           AND latest.bucket_start_epoch = prior.bucket_start_epoch
        "#,
    );

    let rows = builder
        .build_query_as::<ForwardProxyWeightLastBeforeRangeRow>()
        .fetch_all(pool)
        .await
        .with_context(|| {
            format!("failed to query forward proxy weight carry values before {range_start_epoch}")
        })?;

    let alias_map = load_forward_proxy_key_aliases(pool).await?;
    let mut grouped = HashMap::new();
    let mut latest_sample_epochs = HashMap::new();
    for row in rows {
        let proxy_key = alias_map
            .get(&row.proxy_key)
            .cloned()
            .unwrap_or(row.proxy_key.clone());
        let current_latest = latest_sample_epochs
            .get(&proxy_key)
            .copied()
            .unwrap_or(i64::MIN);
        if row.last_sample_epoch_us >= current_latest {
            grouped.insert(proxy_key.clone(), row.last_weight);
            latest_sample_epochs.insert(proxy_key, row.last_sample_epoch_us);
        }
    }
    Ok(grouped)
}

pub(crate) async fn build_forward_proxy_settings_response(
    state: &AppState,
) -> Result<ForwardProxySettingsResponse> {
    let now_utc = Utc::now();
    let window_end_at = db_occurred_at_lower_bound(now_utc + ChronoDuration::seconds(1));
    let widest_window_start_at = db_occurred_at_lower_bound(now_utc - ChronoDuration::days(7));
    let pending_archive_file_paths = load_pending_pool_upstream_node_health_archive_file_paths(
        &state.pool,
        &widest_window_start_at,
        &window_end_at,
    )
    .await?;

    let (settings, runtime_rows, runtime_health_key_by_proxy_key) = {
        let manager = state.forward_proxy.lock().await;
        let mut runtime_rows = manager
            .snapshot_runtime()
            .into_iter()
            .collect::<Vec<_>>();
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
        (
            manager.settings.clone(),
            runtime_rows,
            runtime_health_key_by_proxy_key,
        )
    };
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

    let mut nodes = runtime_rows
        .into_iter()
        .map(|runtime| {
            let health_key = runtime_health_key_by_proxy_key
                .get(&runtime.proxy_key)
                .unwrap_or(&runtime.proxy_key);
            let stats_for = |index: usize| {
                window_maps[index]
                    .get(health_key)
                    .cloned()
                    .map(ForwardProxyWindowStatsResponse::from)
                    .unwrap_or_default()
            };
            ForwardProxyNodeResponse {
                key: runtime.proxy_key.clone(),
                source: runtime.source.clone(),
                display_name: runtime.display_name.clone(),
                endpoint_url: runtime.endpoint_url.clone(),
                weight: runtime.weight,
                penalized: runtime.is_penalized(),
                stats: ForwardProxyStatsResponse {
                    one_minute: stats_for(0),
                    fifteen_minutes: stats_for(1),
                    one_hour: stats_for(2),
                    one_day: stats_for(3),
                    seven_days: stats_for(4),
                },
            }
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|lhs, rhs| lhs.display_name.cmp(&rhs.display_name));

    Ok(ForwardProxySettingsResponse {
        proxy_urls: settings.proxy_urls,
        subscription_urls: settings.subscription_urls,
        subscription_update_interval_secs: settings.subscription_update_interval_secs,
        nodes,
    })
}

async fn build_forward_proxy_binding_node_catalog(
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

fn apply_forward_proxy_binding_hourly_buckets(
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

fn build_forward_proxy_hourly_buckets(
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
        let mut runtime_rows = manager
            .snapshot_runtime()
            .into_iter()
            .collect::<Vec<_>>();
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
        let mut runtime_rows = manager
            .snapshot_runtime()
            .into_iter()
            .collect::<Vec<_>>();
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

pub(crate) async fn post_forward_proxy_candidate_validation(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ForwardProxyCandidateValidationRequest>,
) -> Result<Json<ForwardProxyCandidateValidationResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }

    let result = match payload.kind {
        ForwardProxyValidationKind::ProxyUrl => {
            validate_single_forward_proxy_candidate(state.as_ref(), payload.value).await
        }
        ForwardProxyValidationKind::SubscriptionUrl => {
            validate_subscription_candidate(state.clone(), payload.value).await
        }
    };

    let response = match result {
        Ok(response) => response,
        Err(err) => {
            warn!(error = %err, "forward proxy candidate validation failed");
            ForwardProxyCandidateValidationResponse::failed(err.to_string())
        }
    };

    Ok(Json(response))
}

pub(crate) async fn validate_single_forward_proxy_candidate(
    state: &AppState,
    value: String,
) -> Result<ForwardProxyCandidateValidationResponse> {
    let parsed = parse_forward_proxy_entry(value.trim())
        .ok_or_else(|| anyhow!("unsupported proxy url or unsupported scheme"))?;
    let endpoint = ForwardProxyEndpoint {
        key: format!(
            "__validate_proxy__{:016x}",
            stable_hash_u64(&parsed.normalized)
        ),
        source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
        display_name: parsed.display_name,
        protocol: parsed.protocol,
        endpoint_url: parsed.endpoint_url,
        raw_url: Some(parsed.normalized.clone()),
    };
    let latency_ms = probe_forward_proxy_endpoint(
        state,
        &endpoint,
        forward_proxy_validation_timeout(ForwardProxyValidationKind::ProxyUrl),
        None,
    )
    .await?
    .expect("validation probes should not be cancelled without a shutdown token");
    Ok(ForwardProxyCandidateValidationResponse::success(
        "proxy validation succeeded",
        Some(parsed.normalized),
        Some(1),
        Some(latency_ms),
    ))
}

pub(crate) async fn validate_subscription_candidate(
    state: Arc<AppState>,
    value: String,
) -> Result<ForwardProxyCandidateValidationResponse> {
    let validation_timeout =
        forward_proxy_validation_timeout(ForwardProxyValidationKind::SubscriptionUrl);
    let validation_started = Instant::now();
    let normalized_subscription = normalize_subscription_entries(vec![value])
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("subscription url must be a valid http/https url"))?;
    let urls = fetch_subscription_proxy_urls_with_validation_budget(
        &state.http_clients.shared,
        &normalized_subscription,
        validation_timeout,
        validation_started,
    )
    .await
    .context("failed to fetch or decode subscription payload")?;
    if urls.is_empty() {
        bail!("subscription resolved zero proxy entries");
    }
    let endpoints = normalize_proxy_endpoints_from_urls(&urls, FORWARD_PROXY_SOURCE_SUBSCRIPTION);
    if endpoints.is_empty() {
        bail!("subscription contains no supported proxy entries");
    }

    let discovered_nodes = endpoints.len();
    let latency_ms = validate_subscription_endpoints_concurrently(
        state,
        endpoints,
        validation_timeout,
        validation_started,
    )
    .await?;

    Ok(ForwardProxyCandidateValidationResponse::success(
        "subscription validation succeeded",
        Some(normalized_subscription),
        Some(discovered_nodes),
        Some(latency_ms),
    ))
}

pub(crate) async fn validate_subscription_endpoints_concurrently(
    state: Arc<AppState>,
    endpoints: Vec<ForwardProxyEndpoint>,
    validation_timeout: Duration,
    validation_started: Instant,
) -> Result<f64> {
    let endpoint_count = endpoints.len();
    let concurrency = FORWARD_PROXY_SUBSCRIPTION_PROBE_CONCURRENCY.max(1);
    let attempts = FORWARD_PROXY_SUBSCRIPTION_PROBE_ATTEMPTS.max(1);
    let attempt_timeout =
        Duration::from_secs(FORWARD_PROXY_SUBSCRIPTION_PROBE_ATTEMPT_TIMEOUT_SECS.max(1));
    let cancellation = CancellationToken::new();
    let _cancel_on_drop = ProbeCancellationGuard(cancellation.clone());
    let semaphore = Arc::new(Semaphore::new(concurrency));
    let (tx, mut rx) = mpsc::channel::<Result<f64, String>>(endpoint_count.max(1));

    for endpoint in endpoints {
        let state = state.clone();
        let semaphore = semaphore.clone();
        let cancellation = cancellation.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let _permit = match semaphore.acquire_owned().await {
                Ok(permit) => permit,
                Err(err) => {
                    let _ = tx
                        .send(Err(format!(
                            "subscription validation concurrency limiter closed: {err}"
                        )))
                        .await;
                    return;
                }
            };
            if cancellation.is_cancelled() {
                return;
            }
            let result = probe_subscription_endpoint_with_retries(
                state.as_ref(),
                &endpoint,
                attempts,
                attempt_timeout,
                validation_timeout,
                validation_started,
                &cancellation,
            )
            .await;
            match result {
                Ok(latency_ms) => {
                    cancellation.cancel();
                    let _ = tx.send(Ok(latency_ms)).await;
                }
                Err(err) if cancellation.is_cancelled() => {
                    let _ = tx.send(Err(format!("{err:#}"))).await;
                }
                Err(err) => {
                    let _ = tx.send(Err(format!("{err:#}"))).await;
                }
            }
        });
    }
    drop(tx);

    let mut completed = 0usize;
    let mut last_error: Option<String> = None;
    loop {
        let Some(remaining_timeout) =
            remaining_timeout_budget(validation_timeout, validation_started.elapsed())
        else {
            cancellation.cancel();
            return Err(timeout_error_for_duration(validation_timeout));
        };
        if remaining_timeout.is_zero() {
            cancellation.cancel();
            return Err(timeout_error_for_duration(validation_timeout));
        }

        let result = match timeout(remaining_timeout, rx.recv()).await {
            Ok(Some(result)) => result,
            Ok(None) => break,
            Err(_) => {
                cancellation.cancel();
                return Err(timeout_error_for_duration(validation_timeout));
            }
        };

        completed += 1;
        match result {
            Ok(latency_ms) => {
                cancellation.cancel();
                return Ok(latency_ms);
            }
            Err(err) => last_error = Some(err),
        }
        if completed >= endpoint_count {
            break;
        }
    }

    let mut message = format!(
        "subscription validation scanned {endpoint_count} proxy entries with concurrency {concurrency}, {attempts} attempts per entry, and {}s per attempt; no entry passed validation",
        timeout_seconds_for_message(attempt_timeout)
    );
    if let Some(err) = last_error {
        message.push_str(&format!("; last error: {err}"));
    }
    bail!(message)
}

struct ProbeCancellationGuard(CancellationToken);

impl Drop for ProbeCancellationGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

async fn probe_subscription_endpoint_with_retries(
    state: &AppState,
    endpoint: &ForwardProxyEndpoint,
    attempts: usize,
    attempt_timeout: Duration,
    validation_timeout: Duration,
    validation_started: Instant,
    cancellation: &CancellationToken,
) -> Result<f64> {
    let mut last_error: Option<anyhow::Error> = None;
    for attempt in 1..=attempts {
        if cancellation.is_cancelled() {
            return Err(shutdown_cancelled_forward_proxy_probe());
        }
        let Some(remaining_timeout) =
            remaining_timeout_budget(validation_timeout, validation_started.elapsed())
        else {
            return Err(timeout_error_for_duration(validation_timeout));
        };
        if remaining_timeout.is_zero() {
            return Err(timeout_error_for_duration(validation_timeout));
        }

        let probe_result = tokio::select! {
            _ = cancellation.cancelled() => {
                return Err(shutdown_cancelled_forward_proxy_probe());
            }
            _ = tokio::time::sleep(remaining_timeout) => {
                return Err(timeout_error_for_duration(validation_timeout));
            }
            result = probe_forward_proxy_endpoint(state, endpoint, attempt_timeout, Some(cancellation)) => {
                result
            }
        };

        match probe_result {
            Ok(Some(latency_ms)) => return Ok(latency_ms),
            Ok(None) => return Err(shutdown_cancelled_forward_proxy_probe()),
            Err(err) => {
                last_error = Some(err.context(format!(
                    "attempt {attempt}/{attempts} failed for {}",
                    endpoint.display_name
                )));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("subscription proxy probe did not run")))
}

fn shutdown_cancelled_forward_proxy_probe() -> anyhow::Error {
    anyhow!("forward proxy probe cancelled because shutdown is in progress")
}

pub(crate) async fn probe_forward_proxy_endpoint(
    state: &AppState,
    endpoint: &ForwardProxyEndpoint,
    validation_timeout: Duration,
    shutdown: Option<&CancellationToken>,
) -> Result<Option<f64>> {
    if shutdown.is_some_and(CancellationToken::is_cancelled) {
        return Ok(None);
    }

    let probe_target = state
        .config
        .openai_upstream_base_url
        .join("v1/models")
        .context("failed to build validation probe target")?;
    let started = Instant::now();
    let (endpoint_url, temporary_xray_key) = match resolve_forward_proxy_probe_endpoint_url(
        state,
        endpoint,
        validation_timeout,
        shutdown,
    )
    .await
    {
        Ok(result) => result,
        Err(_err) if shutdown.is_some_and(CancellationToken::is_cancelled) => {
            return Ok(None);
        }
        Err(err) => return Err(err),
    };

    let probe_result = async {
        let send_timeout = remaining_timeout_budget(validation_timeout, started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| timeout_error_for_duration(validation_timeout))?;
        let client = state
            .http_clients
            .client_for_forward_proxy(endpoint_url.as_ref())?;
        let response = match shutdown {
            Some(shutdown) => {
                tokio::select! {
                    _ = shutdown.cancelled() => {
                        return Ok(None);
                    }
                    response = timeout(send_timeout, client.get(probe_target).send()) => {
                        response
                            .map_err(|_| timeout_error_for_duration(validation_timeout))?
                            .context("validation request failed")?
                    }
                }
            }
            None => timeout(send_timeout, client.get(probe_target).send())
                .await
                .map_err(|_| timeout_error_for_duration(validation_timeout))?
                .context("validation request failed")?,
        };
        let status = response.status();
        // Validation only needs to prove the route is reachable; auth/404 still count as reachable.
        if !is_validation_probe_reachable_status(status) {
            bail!("validation probe returned status {}", status);
        }
        if shutdown.is_some_and(CancellationToken::is_cancelled) {
            return Ok(None);
        }
        Ok(Some(elapsed_ms(started)))
    }
    .await;

    if let Some(temp_key) = temporary_xray_key {
        let mut supervisor = state.xray_supervisor.lock().await;
        supervisor.remove_instance(&temp_key).await;
    }

    probe_result
}

pub(crate) fn is_validation_probe_reachable_status(status: StatusCode) -> bool {
    status.is_success()
        || status == StatusCode::UNAUTHORIZED
        || status == StatusCode::FORBIDDEN
        || status == StatusCode::NOT_FOUND
}

pub(crate) fn forward_proxy_validation_timeout(kind: ForwardProxyValidationKind) -> Duration {
    match kind {
        ForwardProxyValidationKind::ProxyUrl => {
            Duration::from_secs(FORWARD_PROXY_VALIDATION_TIMEOUT_SECS)
        }
        ForwardProxyValidationKind::SubscriptionUrl => {
            Duration::from_secs(FORWARD_PROXY_SUBSCRIPTION_VALIDATION_TIMEOUT_SECS)
        }
    }
}

pub(crate) fn remaining_timeout_budget(
    total_timeout: Duration,
    elapsed: Duration,
) -> Option<Duration> {
    total_timeout.checked_sub(elapsed)
}

pub(crate) fn timeout_budget_exhausted(total_timeout: Duration, elapsed: Duration) -> bool {
    match remaining_timeout_budget(total_timeout, elapsed) {
        Some(remaining) => remaining.is_zero(),
        None => true,
    }
}

pub(crate) fn timeout_error_for_duration(timeout: Duration) -> anyhow::Error {
    anyhow!(
        "validation request timed out after {}s",
        timeout_seconds_for_message(timeout)
    )
}

pub(crate) fn timeout_seconds_for_message(timeout: Duration) -> u64 {
    let secs = timeout.as_secs();
    if timeout.subsec_nanos() > 0 {
        secs.saturating_add(1).max(1)
    } else {
        secs.max(1)
    }
}

pub(crate) async fn resolve_forward_proxy_probe_endpoint_url(
    state: &AppState,
    endpoint: &ForwardProxyEndpoint,
    validation_timeout: Duration,
    shutdown: Option<&CancellationToken>,
) -> Result<(Option<Url>, Option<String>)> {
    if shutdown.is_some_and(CancellationToken::is_cancelled) {
        return Err(shutdown_cancelled_forward_proxy_probe());
    }
    if !endpoint.requires_xray() {
        return Ok((endpoint.endpoint_url.clone(), None));
    }
    let raw_url = endpoint
        .raw_url
        .as_deref()
        .ok_or_else(|| anyhow!("xray proxy validation requires raw proxy url"))?;
    let temporary_key = format!(
        "__validate_xray__{:016x}_{}",
        stable_hash_u64(raw_url),
        Utc::now().timestamp_millis()
    );
    let probe_endpoint = ForwardProxyEndpoint {
        key: temporary_key.clone(),
        source: endpoint.source.clone(),
        display_name: endpoint.display_name.clone(),
        protocol: endpoint.protocol,
        endpoint_url: None,
        raw_url: Some(raw_url.to_string()),
    };
    let validation_shutdown = shutdown.cloned().unwrap_or_else(CancellationToken::new);
    let route_url = {
        let mut supervisor = state.xray_supervisor.lock().await;
        supervisor
            .ensure_instance_with_ready_timeout(
                &probe_endpoint,
                validation_timeout,
                &validation_shutdown,
            )
            .await?
    };
    Ok((Some(route_url), Some(temporary_key)))
}

pub(crate) fn snapshot_active_forward_proxy_endpoints(
    manager: &ForwardProxyManager,
) -> Vec<ForwardProxyEndpoint> {
    manager
        .endpoints
        .iter()
        .filter(|endpoint| endpoint.protocol != ForwardProxyProtocol::Direct)
        .filter(|endpoint| endpoint.endpoint_url.is_some() || endpoint.requires_xray())
        .cloned()
        .collect()
}

pub(crate) fn compute_added_forward_proxy_endpoints(
    before: &[ForwardProxyEndpoint],
    after: &[ForwardProxyEndpoint],
) -> Vec<ForwardProxyEndpoint> {
    let known = before
        .iter()
        .map(|endpoint| endpoint.key.as_str())
        .collect::<HashSet<_>>();
    after
        .iter()
        .filter(|endpoint| !known.contains(endpoint.key.as_str()))
        .cloned()
        .collect()
}

pub(crate) fn snapshot_known_subscription_proxy_keys(
    manager: &ForwardProxyManager,
) -> HashSet<String> {
    manager
        .runtime
        .values()
        .filter(|entry| entry.source == FORWARD_PROXY_SOURCE_SUBSCRIPTION)
        .map(|entry| entry.proxy_key.clone())
        .collect()
}

pub(crate) fn classify_bootstrap_forward_proxy_probe_failure(err: &anyhow::Error) -> &'static str {
    let message = err.to_string().to_ascii_lowercase();
    if message.contains("timed out") || message.contains("timeout") {
        return FORWARD_PROXY_FAILURE_HANDSHAKE_TIMEOUT;
    }
    if message.contains("validation probe returned status 5") {
        return FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX;
    }
    FORWARD_PROXY_FAILURE_SEND_ERROR
}

pub(crate) fn spawn_forward_proxy_bootstrap_probe_round(
    state: Arc<AppState>,
    added_endpoints: Vec<ForwardProxyEndpoint>,
    trigger: &'static str,
) {
    if added_endpoints.is_empty() || state.shutdown.is_cancelled() {
        return;
    }
    tokio::spawn(async move {
        let shutdown = state.shutdown.clone();
        let validation_timeout =
            forward_proxy_validation_timeout(ForwardProxyValidationKind::ProxyUrl);
        info!(
            trigger,
            added_count = added_endpoints.len(),
            timeout_secs = validation_timeout.as_secs(),
            "forward proxy bootstrap probe round started"
        );
        for endpoint in added_endpoints {
            if shutdown.is_cancelled() {
                info!(
                    trigger,
                    "forward proxy bootstrap probe round stopped by shutdown"
                );
                break;
            }
            let selected_proxy = SelectedForwardProxy::from_endpoint(&endpoint);
            let started = Instant::now();
            let probe_result = probe_forward_proxy_endpoint(
                state.as_ref(),
                &endpoint,
                validation_timeout,
                Some(&shutdown),
            )
            .await;
            match probe_result {
                Ok(Some(latency_ms)) => {
                    if shutdown.is_cancelled() {
                        info!(
                            trigger,
                            proxy_key_ref = %forward_proxy_log_ref(&endpoint.key),
                            "forward proxy bootstrap probe round stopped before recording a completed probe because shutdown is in progress"
                        );
                        break;
                    }
                    record_forward_proxy_attempt(
                        state.clone(),
                        selected_proxy,
                        true,
                        Some(latency_ms),
                        None,
                        true,
                    )
                    .await;
                }
                Ok(None) => {
                    info!(
                        trigger,
                        proxy_key_ref = %forward_proxy_log_ref(&endpoint.key),
                        "forward proxy bootstrap probe round stopped by shutdown during an in-flight probe"
                    );
                    break;
                }
                Err(err) => {
                    let failure_kind = classify_bootstrap_forward_proxy_probe_failure(&err);
                    warn!(
                        trigger,
                        proxy_key_ref = %forward_proxy_log_ref(&endpoint.key),
                        proxy_source = endpoint.source,
                        proxy_label = endpoint.display_name,
                        proxy_url_ref = %forward_proxy_log_ref_option(endpoint.raw_url.as_deref()),
                        failure_kind,
                        error = %err,
                        "forward proxy bootstrap probe failed"
                    );
                    record_forward_proxy_attempt(
                        state.clone(),
                        selected_proxy,
                        false,
                        Some(elapsed_ms(started)),
                        Some(failure_kind),
                        true,
                    )
                    .await;
                }
            }
        }
        info!(trigger, "forward proxy bootstrap probe round finished");
    });
}
