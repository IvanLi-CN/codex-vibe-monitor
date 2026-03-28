use super::*;
use crate::stats::*;

#[derive(Debug, FromRow)]
pub(crate) struct ForwardProxyAttemptStatsRow {
    pub(crate) proxy_key: String,
    pub(crate) attempts: i64,
    pub(crate) success_count: i64,
    pub(crate) latency_sum_ms: Option<f64>,
}

#[derive(Debug, FromRow)]
pub(crate) struct ForwardProxyHourlyStatsRow {
    pub(crate) proxy_key: String,
    pub(crate) bucket_start_epoch: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
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

fn ceil_hour_epoch(epoch: i64) -> i64 {
    let floor = align_bucket_epoch(epoch, 3_600, 0);
    if floor < epoch { floor + 3_600 } else { floor }
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
}

pub(crate) async fn load_forward_proxy_metadata_history(
    pool: &Pool<Sqlite>,
    proxy_keys: &[String],
) -> Result<HashMap<String, ForwardProxyMetadataHistoryRow>> {
    if proxy_keys.is_empty() {
        return Ok(HashMap::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT proxy_key, display_name, source, endpoint_url \
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

    let rows = query
        .build_query_as::<ForwardProxyMetadataHistoryRow>()
        .fetch_all(pool)
        .await
        .context("failed to load forward_proxy metadata history rows")?;
    Ok(rows
        .into_iter()
        .map(|row| (row.proxy_key.clone(), row))
        .collect())
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

pub(crate) async fn query_forward_proxy_window_stats(
    pool: &Pool<Sqlite>,
    window: &str,
) -> Result<HashMap<String, ForwardProxyAttemptWindowStats>> {
    let rows = sqlx::query_as::<_, ForwardProxyAttemptStatsRow>(
        r#"
        SELECT
            proxy_key,
            COUNT(*) AS attempts,
            SUM(CASE WHEN is_success != 0 THEN 1 ELSE 0 END) AS success_count,
            SUM(CASE WHEN is_success != 0 THEN latency_ms END) AS latency_sum_ms
        FROM forward_proxy_attempts
        WHERE occurred_at >= datetime('now', ?1)
        GROUP BY proxy_key
        "#,
    )
    .bind(window)
    .fetch_all(pool)
    .await
    .with_context(|| format!("failed to query forward proxy attempt stats for {window}"))?;

    let alias_map = load_forward_proxy_key_aliases(pool).await?;
    let mut grouped = HashMap::new();
    let mut latency_totals = HashMap::new();

    for row in rows {
        let proxy_key = alias_map
            .get(&row.proxy_key)
            .cloned()
            .unwrap_or(row.proxy_key.clone());
        let stats = grouped
            .entry(proxy_key.clone())
            .or_insert_with(ForwardProxyAttemptWindowStats::default);
        stats.attempts += row.attempts;
        stats.success_count += row.success_count;
        *latency_totals.entry(proxy_key).or_insert(0.0) += row.latency_sum_ms.unwrap_or(0.0);
    }

    for (proxy_key, stats) in &mut grouped {
        if stats.success_count > 0 {
            stats.avg_latency_ms = latency_totals
                .get(proxy_key)
                .copied()
                .map(|value| value / stats.success_count as f64);
        }
    }

    Ok(grouped)
}

pub(crate) async fn query_forward_proxy_hourly_stats(
    pool: &Pool<Sqlite>,
    range_start_epoch: i64,
    range_end_epoch: i64,
) -> Result<HashMap<String, HashMap<i64, ForwardProxyHourlyStatsPoint>>> {
    let rows = sqlx::query_as::<_, ForwardProxyHourlyStatsRow>(
        r#"
        SELECT
            proxy_key,
            bucket_start_epoch,
            success_count,
            failure_count
        FROM forward_proxy_attempt_hourly
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
            "failed to query forward proxy hourly stats within [{range_start_epoch}, {range_end_epoch})"
        )
    })?;

    let alias_map = load_forward_proxy_key_aliases(pool).await?;
    let mut grouped: HashMap<String, HashMap<i64, ForwardProxyHourlyStatsPoint>> = HashMap::new();
    for row in rows {
        let proxy_key = alias_map
            .get(&row.proxy_key)
            .cloned()
            .unwrap_or(row.proxy_key.clone());
        let point = grouped
            .entry(proxy_key)
            .or_default()
            .entry(row.bucket_start_epoch)
            .or_default();
        point.success_count += row.success_count;
        point.failure_count += row.failure_count;
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
    let (settings, runtime_rows) = {
        let manager = state.forward_proxy.lock().await;
        (
            manager.settings.clone(),
            manager
                .snapshot_runtime()
                .into_iter()
                .filter(|runtime| runtime.proxy_key != FORWARD_PROXY_DIRECT_KEY)
                .collect::<Vec<_>>(),
        )
    };

    let windows = [
        ("-1 minute", 0usize),
        ("-15 minutes", 1usize),
        ("-1 hour", 2usize),
        ("-1 day", 3usize),
        ("-7 days", 4usize),
    ];
    let mut window_maps: Vec<HashMap<String, ForwardProxyAttemptWindowStats>> = Vec::new();
    for (window, _) in &windows {
        window_maps.push(query_forward_proxy_window_stats(&state.pool, window).await?);
    }

    let mut nodes = runtime_rows
        .into_iter()
        .map(|runtime| {
            let stats_for = |index: usize| {
                window_maps[index]
                    .get(&runtime.proxy_key)
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

pub(crate) async fn build_forward_proxy_binding_nodes_response(
    state: &AppState,
    extra_proxy_keys: &[String],
) -> Result<Vec<ForwardProxyBindingNodeResponse>> {
    const BUCKET_SECONDS: i64 = 3600;
    const BUCKET_COUNT: i64 = 24;

    crate::ensure_hourly_rollups_caught_up(state).await?;

    let mut nodes = {
        let manager = state.forward_proxy.lock().await;
        manager.binding_nodes()
    };
    let current_node_keys = nodes
        .iter()
        .map(|node| node.key.clone())
        .collect::<HashSet<_>>();

    let mut seen = nodes
        .iter()
        .map(|node| node.key.clone())
        .collect::<HashSet<_>>();
    let extra_keys = extra_proxy_keys
        .iter()
        .map(|key| key.trim())
        .filter(|key| !key.is_empty())
        .map(ToOwned::to_owned)
        .filter(|key| seen.insert(key.clone()))
        .collect::<Vec<_>>();
    let mut metadata_keys = nodes
        .iter()
        .map(|node| node.key.clone())
        .collect::<Vec<_>>();
    metadata_keys.extend(extra_keys.iter().cloned());
    let metadata_map = load_forward_proxy_metadata_history(&state.pool, &metadata_keys).await?;

    for proxy_key in extra_keys {
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
            penalized: false,
            selectable: false,
            last24h: Vec::new(),
        });
    }

    if nodes.is_empty() {
        return Ok(nodes);
    }

    let now_epoch = Utc::now().timestamp();
    let range_end_epoch = align_bucket_epoch(now_epoch, BUCKET_SECONDS, 0) + BUCKET_SECONDS;
    let range_start_epoch = range_end_epoch - BUCKET_COUNT * BUCKET_SECONDS;
    let hourly_map =
        query_forward_proxy_hourly_stats(&state.pool, range_start_epoch, range_end_epoch).await?;

    for node in &mut nodes {
        let hourly = hourly_map.get(&node.key);
        node.last24h = if current_node_keys.contains(&node.key) || hourly.is_some() {
            build_forward_proxy_hourly_buckets(
                hourly,
                range_start_epoch,
                BUCKET_SECONDS,
                BUCKET_COUNT,
            )?
        } else {
            Vec::new()
        };
    }
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

    let runtime_rows = {
        let manager = state.forward_proxy.lock().await;
        manager
            .snapshot_runtime()
            .into_iter()
            .filter(|runtime| runtime.proxy_key != FORWARD_PROXY_DIRECT_KEY)
            .collect::<Vec<_>>()
    };
    let runtime_proxy_keys = runtime_rows
        .iter()
        .map(|runtime| runtime.proxy_key.clone())
        .collect::<Vec<_>>();

    let windows = [
        ("-1 minute", 0usize),
        ("-15 minutes", 1usize),
        ("-1 hour", 2usize),
        ("-1 day", 3usize),
        ("-7 days", 4usize),
    ];
    let mut window_maps: Vec<HashMap<String, ForwardProxyAttemptWindowStats>> = Vec::new();
    for (window, _) in &windows {
        window_maps.push(query_forward_proxy_window_stats(&state.pool, window).await?);
    }

    let now_epoch = Utc::now().timestamp();
    let range_end_epoch = align_bucket_epoch(now_epoch, BUCKET_SECONDS, 0) + BUCKET_SECONDS;
    let range_start_epoch = range_end_epoch - BUCKET_COUNT * BUCKET_SECONDS;
    let hourly_map =
        query_forward_proxy_hourly_stats(&state.pool, range_start_epoch, range_end_epoch).await?;
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
            let penalized = runtime.is_penalized();
            let runtime_weight = runtime.weight;
            let stats_for = |index: usize, key: &str| {
                window_maps[index]
                    .get(key)
                    .cloned()
                    .map(ForwardProxyWindowStatsResponse::from)
                    .unwrap_or_default()
            };
            let hourly = hourly_map.get(&proxy_key);
            let weight_hourly = weight_hourly_map.get(&proxy_key);
            let mut carry_weight = weight_carry_map
                .get(&proxy_key)
                .copied()
                .unwrap_or(runtime_weight);
            let one_minute = stats_for(0, &proxy_key);
            let fifteen_minutes = stats_for(1, &proxy_key);
            let one_hour = stats_for(2, &proxy_key);
            let one_day = stats_for(3, &proxy_key);
            let seven_days = stats_for(4, &proxy_key);
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

    let runtime_rows = {
        let manager = state.forward_proxy.lock().await;
        manager
            .snapshot_runtime()
            .into_iter()
            .filter(|runtime| runtime.proxy_key != FORWARD_PROXY_DIRECT_KEY)
            .collect::<Vec<_>>()
    };
    let runtime_map = runtime_rows
        .into_iter()
        .map(|runtime| (runtime.proxy_key.clone(), runtime))
        .collect::<HashMap<_, _>>();

    let start_epoch = range_window.start.timestamp();
    let end_epoch = range_window.end.timestamp();
    let query_start_epoch = align_bucket_epoch(start_epoch, BUCKET_SECONDS, 0);
    let query_end_epoch = ceil_hour_epoch(end_epoch);
    let fill_start_epoch = query_start_epoch;
    let fill_end_epoch = query_end_epoch;

    let hourly_map =
        query_forward_proxy_hourly_stats(&state.pool, query_start_epoch, query_end_epoch).await?;
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
        if key != FORWARD_PROXY_DIRECT_KEY && seen.insert(key.clone()) {
            proxy_keys.push(key.clone());
        }
    }
    for key in weight_hourly_map.keys() {
        if key != FORWARD_PROXY_DIRECT_KEY && seen.insert(key.clone()) {
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
            let request_points = hourly_map.get(&proxy_key);
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
                    .unwrap_or_else(|| "archived".to_string()),
                display_name: runtime
                    .map(|item| item.display_name.clone())
                    .or_else(|| metadata.map(|item| item.display_name.clone()))
                    .unwrap_or_else(|| proxy_key.clone()),
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
            validate_subscription_candidate(state.as_ref(), payload.value).await
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
    state: &AppState,
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

    let mut last_error: Option<anyhow::Error> = None;
    let mut best_latency_ms: Option<f64> = None;
    for endpoint in endpoints.iter().take(3) {
        let Some(remaining_timeout) =
            remaining_timeout_budget(validation_timeout, validation_started.elapsed())
        else {
            last_error = Some(timeout_error_for_duration(validation_timeout));
            break;
        };
        if remaining_timeout.is_zero() {
            last_error = Some(timeout_error_for_duration(validation_timeout));
            break;
        }

        match probe_forward_proxy_endpoint(state, endpoint, remaining_timeout, None).await {
            Ok(Some(latency_ms)) => {
                best_latency_ms = Some(latency_ms);
                break;
            }
            Ok(None) => {
                last_error = Some(shutdown_cancelled_forward_proxy_probe());
                break;
            }
            Err(err) => {
                if timeout_budget_exhausted(validation_timeout, validation_started.elapsed()) {
                    last_error = Some(timeout_error_for_duration(validation_timeout));
                    break;
                }
                last_error = Some(err);
            }
        }
    }

    let Some(latency_ms) = best_latency_ms else {
        if let Some(err) = last_error {
            return Err(anyhow!(
                "subscription proxy probe failed: {err}; no entry passed validation"
            ));
        }
        bail!("no subscription proxy entry passed validation");
    };

    Ok(ForwardProxyCandidateValidationResponse::success(
        "subscription validation succeeded",
        Some(normalized_subscription),
        Some(endpoints.len()),
        Some(latency_ms),
    ))
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

pub(crate) async fn select_forward_proxy_for_scope(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
) -> Result<SelectedForwardProxy> {
    let mut manager = state.forward_proxy.lock().await;
    manager.select_proxy_for_scope(scope)
}

pub(crate) async fn record_forward_proxy_scope_result(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
    selected_proxy_key: &str,
    result: ForwardProxyRouteResultKind,
) {
    let mut manager = state.forward_proxy.lock().await;
    manager.record_scope_result(scope, selected_proxy_key, result);
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
    pub(crate) current_proxy_key: Option<String>,
    pub(crate) consecutive_network_failures: u32,
}

#[derive(Debug, Clone)]
pub(crate) enum ForwardProxyRouteScope {
    Automatic,
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
        let mut bound_key_aliases = HashMap::new();
        for endpoint in &self.endpoints {
            let Some(raw_url) = endpoint.raw_url.as_deref() else {
                continue;
            };
            for alias in legacy_bound_proxy_key_aliases(raw_url, endpoint.protocol) {
                if alias != endpoint.key {
                    bound_key_aliases.insert(alias, endpoint.key.clone());
                }
            }
        }
        self.bound_key_aliases = bound_key_aliases;

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
        let mut selectable = self
            .endpoints
            .iter()
            .filter(|endpoint| endpoint.is_bound_selectable())
            .map(|endpoint| endpoint.key.as_str())
            .collect::<HashSet<_>>();
        selectable.insert(FORWARD_PROXY_DIRECT_KEY);
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
            if !selectable.contains(canonical.as_str()) || !seen.insert(canonical.clone()) {
                continue;
            }
            available.push(canonical);
        }
        available.sort();
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
            .and_then(|state| state.current_proxy_key.clone())
            .filter(|key| available_keys.contains(key));
        let selected_key = existing_current.unwrap_or_else(|| {
            self.choose_random_bound_proxy_key(&available_keys, None)
                .expect("available bound proxy keys should not be empty")
        });
        let state = self
            .bound_group_runtime
            .entry(group_name.to_string())
            .or_default();
        state.current_proxy_key = Some(selected_key.clone());
        if selected_key == FORWARD_PROXY_DIRECT_KEY {
            return Ok(SelectedForwardProxy::from_endpoint(
                &ForwardProxyEndpoint::direct(),
            ));
        }
        let endpoint = self
            .endpoints
            .iter()
            .find(|endpoint| endpoint.key == selected_key)
            .ok_or_else(|| anyhow!("selected bound proxy disappeared from runtime"))?;
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

        let mut should_switch = false;
        {
            let state = self
                .bound_group_runtime
                .entry(group_name.clone())
                .or_default();
            state.current_proxy_key = Some(selected_proxy_key.to_string());
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
            && let Some(next_proxy_key) =
                self.choose_random_bound_proxy_key(&available_keys, Some(selected_proxy_key))
        {
            let state = self
                .bound_group_runtime
                .entry(group_name.clone())
                .or_default();
            state.current_proxy_key = Some(next_proxy_key);
            state.consecutive_network_failures = 0;
        }
    }

    pub(crate) fn binding_nodes(&self) -> Vec<ForwardProxyBindingNodeResponse> {
        let mut alias_keys_by_key: HashMap<&str, Vec<String>> = HashMap::new();
        for (alias, canonical) in &self.bound_key_aliases {
            alias_keys_by_key
                .entry(canonical.as_str())
                .or_default()
                .push(alias.clone());
        }
        let mut nodes = self
            .endpoints
            .iter()
            .map(|endpoint| {
                let penalized = self
                    .runtime
                    .get(&endpoint.key)
                    .is_some_and(ForwardProxyRuntimeState::is_penalized);
                let mut alias_keys = alias_keys_by_key
                    .remove(endpoint.key.as_str())
                    .unwrap_or_default();
                alias_keys.sort();
                ForwardProxyBindingNodeResponse {
                    key: endpoint.key.clone(),
                    alias_keys,
                    source: endpoint.source.clone(),
                    display_name: endpoint.display_name.clone(),
                    protocol_label: endpoint.protocol.label().to_string(),
                    penalized,
                    selectable: endpoint.is_bound_selectable(),
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

#[derive(Debug, Clone)]
pub(crate) struct SelectedForwardProxy {
    pub(crate) key: String,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) endpoint_url: Option<Url>,
    pub(crate) endpoint_url_raw: Option<String>,
}

impl SelectedForwardProxy {
    pub(crate) fn from_endpoint(endpoint: &ForwardProxyEndpoint) -> Self {
        Self {
            key: endpoint.key.clone(),
            source: endpoint.source.clone(),
            display_name: endpoint.display_name.clone(),
            endpoint_url: endpoint.endpoint_url.clone(),
            endpoint_url_raw: endpoint.raw_url.clone(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct XrayInstance {
    pub(crate) local_proxy_url: Url,
    pub(crate) config_path: PathBuf,
    pub(crate) child: Child,
}

#[derive(Debug, Default)]
pub(crate) struct XraySupervisor {
    pub(crate) binary: String,
    pub(crate) runtime_dir: PathBuf,
    pub(crate) instances: HashMap<String, XrayInstance>,
}

impl XraySupervisor {
    pub(crate) fn new(binary: String, runtime_dir: PathBuf) -> Self {
        Self {
            binary,
            runtime_dir,
            instances: HashMap::new(),
        }
    }

    pub(crate) async fn sync_endpoints(
        &mut self,
        endpoints: &mut [ForwardProxyEndpoint],
        shutdown: &CancellationToken,
    ) -> Result<()> {
        fs::create_dir_all(&self.runtime_dir).with_context(|| {
            format!(
                "failed to create xray runtime directory: {}",
                self.runtime_dir.display()
            )
        })?;

        let desired_keys = endpoints
            .iter()
            .filter(|endpoint| endpoint.requires_xray())
            .map(|endpoint| endpoint.key.clone())
            .collect::<HashSet<_>>();
        let stale_keys = self
            .instances
            .keys()
            .filter(|key| !desired_keys.contains(*key))
            .cloned()
            .collect::<Vec<_>>();

        for endpoint in endpoints {
            if shutdown.is_cancelled() {
                info!("stopping xray route sync because shutdown is in progress");
                bail!("xray route sync cancelled because shutdown is in progress");
            }
            if !endpoint.requires_xray() {
                continue;
            }
            match self.ensure_instance(endpoint, shutdown).await {
                Ok(route_url) => endpoint.endpoint_url = Some(route_url),
                Err(err) => {
                    endpoint.endpoint_url = None;
                    warn!(
                        proxy_key_ref = %forward_proxy_log_ref(&endpoint.key),
                        proxy_source = endpoint.source,
                        proxy_label = endpoint.display_name,
                        proxy_url_ref = %forward_proxy_log_ref_option(endpoint.raw_url.as_deref()),
                        error = %err,
                        "failed to prepare xray forward proxy route"
                    );
                }
            }
        }

        if shutdown.is_cancelled() {
            info!("skipping stale xray route cleanup because shutdown is in progress");
            bail!("xray route sync cancelled because shutdown is in progress");
        }
        for key in stale_keys {
            if shutdown.is_cancelled() {
                info!("skipping stale xray route cleanup because shutdown is in progress");
                bail!("xray route sync cancelled because shutdown is in progress");
            }
            self.remove_instance(&key).await;
        }

        Ok(())
    }

    pub(crate) async fn shutdown_all(&mut self) {
        let keys = self.instances.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            self.remove_instance(&key).await;
        }
    }

    pub(crate) async fn ensure_instance(
        &mut self,
        endpoint: &ForwardProxyEndpoint,
        shutdown: &CancellationToken,
    ) -> Result<Url> {
        self.ensure_instance_with_ready_timeout(
            endpoint,
            Duration::from_millis(XRAY_PROXY_READY_TIMEOUT_MS),
            shutdown,
        )
        .await
    }

    pub(crate) async fn ensure_instance_with_ready_timeout(
        &mut self,
        endpoint: &ForwardProxyEndpoint,
        ready_timeout: Duration,
        shutdown: &CancellationToken,
    ) -> Result<Url> {
        if let Some(instance) = self.instances.get_mut(&endpoint.key) {
            match instance.child.try_wait() {
                Ok(None) => return Ok(instance.local_proxy_url.clone()),
                Ok(Some(status)) => {
                    warn!(
                        proxy_key_ref = %forward_proxy_log_ref(&endpoint.key),
                        status = %status,
                        "xray proxy process exited unexpectedly; restarting"
                    );
                }
                Err(err) => {
                    warn!(
                        proxy_key_ref = %forward_proxy_log_ref(&endpoint.key),
                        error = %err,
                        "failed to inspect xray proxy process; restarting"
                    );
                }
            }
        }

        self.remove_instance(&endpoint.key).await;
        self.spawn_instance(endpoint, ready_timeout, shutdown).await
    }

    pub(crate) async fn spawn_instance(
        &mut self,
        endpoint: &ForwardProxyEndpoint,
        ready_timeout: Duration,
        shutdown: &CancellationToken,
    ) -> Result<Url> {
        let outbound = build_xray_outbound_for_endpoint(endpoint)?;
        let local_port = pick_unused_local_port().context("failed to allocate xray local port")?;
        fs::create_dir_all(&self.runtime_dir).with_context(|| {
            format!(
                "failed to create xray runtime directory: {}",
                self.runtime_dir.display()
            )
        })?;
        let config_path = self.runtime_dir.join(format!(
            "forward-proxy-{:016x}.json",
            stable_hash_u64(&endpoint.key)
        ));
        let config = build_xray_instance_config(local_port, outbound);
        let serialized =
            serde_json::to_vec_pretty(&config).context("failed to serialize xray config")?;
        fs::write(&config_path, serialized)
            .with_context(|| format!("failed to write xray config: {}", config_path.display()))?;

        let mut child = match Command::new(&self.binary)
            .arg("run")
            .arg("-c")
            .arg(&config_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(err) => {
                let _ = fs::remove_file(&config_path);
                return Err(err)
                    .with_context(|| format!("failed to start xray binary: {}", self.binary));
            }
        };

        if let Err(err) =
            wait_for_xray_proxy_ready(&mut child, local_port, ready_timeout, shutdown).await
        {
            let _ = terminate_child_process(
                &mut child,
                Duration::from_secs(2),
                &forward_proxy_log_ref(&endpoint.key),
            )
            .await;
            let _ = fs::remove_file(&config_path);
            return Err(err);
        }

        let local_proxy_url = Url::parse(&format!("socks5h://127.0.0.1:{local_port}"))
            .context("failed to build local xray socks endpoint")?;
        self.instances.insert(
            endpoint.key.clone(),
            XrayInstance {
                local_proxy_url: local_proxy_url.clone(),
                config_path,
                child,
            },
        );

        Ok(local_proxy_url)
    }

    pub(crate) async fn remove_instance(&mut self, key: &str) {
        if let Some(mut instance) = self.instances.remove(key) {
            let proxy_key_ref = forward_proxy_log_ref(key);
            let _ = terminate_child_process(
                &mut instance.child,
                Duration::from_secs(2),
                &proxy_key_ref,
            )
            .await;
            if let Err(err) = fs::remove_file(&instance.config_path)
                && err.kind() != io::ErrorKind::NotFound
            {
                warn!(
                    proxy_key_ref = %proxy_key_ref,
                    path = %instance.config_path.display(),
                    error = %err,
                    "failed to remove xray config file"
                );
            }
        }
    }
}

pub(crate) fn stable_hash_u64(raw: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    raw.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn forward_proxy_log_ref(raw: &str) -> String {
    format!("fp_{:016x}", stable_hash_u64(raw))
}

pub(crate) fn forward_proxy_log_ref_option(raw: Option<&str>) -> String {
    raw.map(forward_proxy_log_ref)
        .unwrap_or_else(|| "direct".to_string())
}

pub(crate) fn pick_unused_local_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .context("failed to bind local socket for port allocation")?;
    let port = listener
        .local_addr()
        .context("failed to read local address for allocated port")?
        .port();
    Ok(port)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildTerminationOutcome {
    AlreadyExited,
    Graceful,
    Forced,
}

pub(crate) async fn terminate_child_process(
    child: &mut Child,
    grace_period: Duration,
    process_ref: &str,
) -> ChildTerminationOutcome {
    match child.try_wait() {
        Ok(Some(status)) => {
            info!(process_ref, status = %status, "child process already exited before shutdown");
            return ChildTerminationOutcome::AlreadyExited;
        }
        Ok(None) => {}
        Err(err) => {
            warn!(process_ref, error = %err, "failed to poll child process before shutdown");
        }
    }

    #[cfg(unix)]
    {
        if let Some(pid) = child.id() {
            let result = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
            if result == 0 {
                info!(
                    process_ref,
                    pid,
                    grace_ms = grace_period.as_millis() as u64,
                    "sent SIGTERM to child process"
                );
                if grace_period.is_zero() {
                    warn!(
                        process_ref,
                        pid,
                        "grace period is zero; falling back to force kill immediately after SIGTERM"
                    );
                } else {
                    match timeout(grace_period, child.wait()).await {
                        Ok(Ok(status)) => {
                            info!(process_ref, pid, status = %status, "child process exited after SIGTERM");
                            return ChildTerminationOutcome::Graceful;
                        }
                        Ok(Err(err)) => {
                            warn!(process_ref, pid, error = %err, "failed while waiting for child process after SIGTERM");
                        }
                        Err(_) => {
                            warn!(
                                process_ref,
                                pid,
                                grace_ms = grace_period.as_millis() as u64,
                                "child process did not exit after SIGTERM; falling back to force kill"
                            );
                        }
                    }
                }
            } else {
                let err = io::Error::last_os_error();
                warn!(process_ref, pid, error = %err, "failed to send SIGTERM to child process; falling back to force kill");
            }
        }
    }

    if let Err(err) = child.kill().await {
        warn!(process_ref, error = %err, "failed to force kill child process");
    } else {
        info!(
            process_ref,
            grace_ms = grace_period.as_millis() as u64,
            "force killed child process after graceful shutdown fallback"
        );
    }

    match timeout(grace_period, child.wait()).await {
        Ok(Ok(status)) => {
            info!(process_ref, status = %status, "child process exited after force kill");
        }
        Ok(Err(err)) => {
            warn!(process_ref, error = %err, "failed while waiting for force killed child process");
        }
        Err(_) => {
            warn!(
                process_ref,
                grace_ms = grace_period.as_millis() as u64,
                "timed out waiting for force killed child process exit"
            );
        }
    }

    ChildTerminationOutcome::Forced
}

pub(crate) async fn wait_for_xray_proxy_ready(
    child: &mut Child,
    local_port: u16,
    ready_timeout: Duration,
    shutdown: &CancellationToken,
) -> Result<()> {
    let deadline = Instant::now() + ready_timeout;
    loop {
        if shutdown.is_cancelled() {
            bail!("xray startup cancelled because shutdown is in progress");
        }
        if let Some(status) = child
            .try_wait()
            .context("failed to poll xray proxy process status")?
        {
            bail!("xray process exited before ready: {status}");
        }
        let connect_attempt = timeout(
            Duration::from_millis(250),
            TcpStream::connect(("127.0.0.1", local_port)),
        );
        tokio::select! {
            _ = shutdown.cancelled() => {
                bail!("xray startup cancelled because shutdown is in progress");
            }
            result = connect_attempt => {
                if result.is_ok_and(|connection| connection.is_ok()) {
                    return Ok(());
                }
            }
        }
        if Instant::now() >= deadline {
            bail!("xray local socks endpoint was not ready in time");
        }
        tokio::select! {
            _ = shutdown.cancelled() => {
                bail!("xray startup cancelled because shutdown is in progress");
            }
            _ = sleep(Duration::from_millis(100)) => {}
        }
    }
}

pub(crate) fn build_xray_instance_config(local_port: u16, outbound: Value) -> Value {
    json!({
        "log": {
            "loglevel": "warning"
        },
        "inbounds": [
            {
                "tag": "inbound-local-socks",
                "listen": "127.0.0.1",
                "port": local_port,
                "protocol": "socks",
                "settings": {
                    "auth": "noauth",
                    "udp": false
                }
            }
        ],
        "outbounds": [
            outbound,
            {
                "tag": "direct",
                "protocol": "freedom"
            }
        ],
        "routing": {
            "domainStrategy": "AsIs",
            "rules": [
                {
                    "type": "field",
                    "inboundTag": ["inbound-local-socks"],
                    "outboundTag": "proxy"
                }
            ]
        }
    })
}

pub(crate) fn build_xray_outbound_for_endpoint(endpoint: &ForwardProxyEndpoint) -> Result<Value> {
    let raw = endpoint
        .raw_url
        .as_deref()
        .ok_or_else(|| anyhow!("xray endpoint missing share link url"))?;
    match endpoint.protocol {
        ForwardProxyProtocol::Vmess => build_vmess_xray_outbound(raw),
        ForwardProxyProtocol::Vless => build_vless_xray_outbound(raw),
        ForwardProxyProtocol::Trojan => build_trojan_xray_outbound(raw),
        ForwardProxyProtocol::Shadowsocks => build_shadowsocks_xray_outbound(raw),
        _ => bail!("unsupported xray protocol for endpoint"),
    }
}

pub(crate) fn build_vmess_xray_outbound(raw: &str) -> Result<Value> {
    let link = parse_vmess_share_link(raw)?;
    let mut outbound = json!({
        "tag": "proxy",
        "protocol": "vmess",
        "settings": {
            "vnext": [
                {
                    "address": link.address,
                    "port": link.port,
                    "users": [
                        {
                            "id": link.id,
                            "alterId": link.alter_id,
                            "security": link.security
                        }
                    ]
                }
            ]
        }
    });
    if let Some(stream_settings) = build_vmess_stream_settings(&link)
        && let Some(object) = outbound.as_object_mut()
    {
        object.insert("streamSettings".to_string(), stream_settings);
    }
    Ok(outbound)
}

pub(crate) fn build_vmess_stream_settings(link: &VmessShareLink) -> Option<Value> {
    let mut stream = serde_json::Map::new();
    stream.insert("network".to_string(), Value::String(link.network.clone()));
    let mut has_non_default_options = link.network != "tcp";

    let security = link
        .tls_mode
        .as_deref()
        .filter(|value| !value.is_empty() && *value != "none")
        .map(|value| value.to_ascii_lowercase());
    if let Some(security) = security.as_ref() {
        stream.insert("security".to_string(), Value::String(security.clone()));
        has_non_default_options = true;
    }

    match link.network.as_str() {
        "ws" => {
            let mut ws = serde_json::Map::new();
            if let Some(path) = link.path.as_ref().filter(|value| !value.trim().is_empty()) {
                ws.insert("path".to_string(), Value::String(path.clone()));
            }
            if let Some(host) = link.host.as_ref().filter(|value| !value.trim().is_empty()) {
                ws.insert("headers".to_string(), json!({ "Host": host }));
            }
            if !ws.is_empty() {
                stream.insert("wsSettings".to_string(), Value::Object(ws));
                has_non_default_options = true;
            }
        }
        "grpc" => {
            let service_name = link
                .path
                .as_ref()
                .filter(|value| !value.trim().is_empty())
                .cloned()
                .unwrap_or_default();
            stream.insert(
                "grpcSettings".to_string(),
                json!({
                    "serviceName": service_name
                }),
            );
            has_non_default_options = true;
        }
        "httpupgrade" => {
            let mut settings = serde_json::Map::new();
            if let Some(host) = link.host.as_ref().filter(|value| !value.trim().is_empty()) {
                settings.insert("host".to_string(), Value::String(host.clone()));
            }
            if let Some(path) = link.path.as_ref().filter(|value| !value.trim().is_empty()) {
                settings.insert("path".to_string(), Value::String(path.clone()));
            }
            if !settings.is_empty() {
                stream.insert("httpupgradeSettings".to_string(), Value::Object(settings));
                has_non_default_options = true;
            }
        }
        _ => {}
    }

    if let Some(security) = security {
        if security == "tls" {
            let mut tls_settings = serde_json::Map::new();
            if let Some(server_name) = link
                .sni
                .as_ref()
                .or(link.host.as_ref())
                .filter(|value| !value.trim().is_empty())
            {
                tls_settings.insert("serverName".to_string(), Value::String(server_name.clone()));
            }
            if let Some(alpn) = link.alpn.as_ref().filter(|items| !items.is_empty()) {
                tls_settings.insert("alpn".to_string(), json!(alpn));
            }
            if let Some(fingerprint) = link
                .fingerprint
                .as_ref()
                .filter(|value| !value.trim().is_empty())
            {
                tls_settings.insert(
                    "fingerprint".to_string(),
                    Value::String(fingerprint.clone()),
                );
            }
            if !tls_settings.is_empty() {
                stream.insert("tlsSettings".to_string(), Value::Object(tls_settings));
                has_non_default_options = true;
            }
        } else if security == "reality" {
            let mut reality_settings = serde_json::Map::new();
            if let Some(server_name) = link
                .sni
                .as_ref()
                .or(link.host.as_ref())
                .filter(|value| !value.trim().is_empty())
            {
                reality_settings
                    .insert("serverName".to_string(), Value::String(server_name.clone()));
            }
            if let Some(fingerprint) = link
                .fingerprint
                .as_ref()
                .filter(|value| !value.trim().is_empty())
            {
                reality_settings.insert(
                    "fingerprint".to_string(),
                    Value::String(fingerprint.clone()),
                );
            }
            if !reality_settings.is_empty() {
                stream.insert(
                    "realitySettings".to_string(),
                    Value::Object(reality_settings),
                );
                has_non_default_options = true;
            }
        }
    }

    if has_non_default_options {
        Some(Value::Object(stream))
    } else {
        None
    }
}

pub(crate) fn build_vless_xray_outbound(raw: &str) -> Result<Value> {
    let url = Url::parse(raw).context("invalid vless share link")?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("vless host missing"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow!("vless port missing"))?;
    let user_id = url.username();
    if user_id.trim().is_empty() {
        bail!("vless id missing");
    }

    let query = url.query_pairs().into_owned().collect::<HashMap<_, _>>();
    let encryption = query
        .get("encryption")
        .cloned()
        .unwrap_or_else(|| "none".to_string());
    let mut user = serde_json::Map::new();
    user.insert("id".to_string(), Value::String(user_id.to_string()));
    user.insert("encryption".to_string(), Value::String(encryption));
    if let Some(flow) = query.get("flow").filter(|value| !value.trim().is_empty()) {
        user.insert("flow".to_string(), Value::String(flow.clone()));
    }

    let mut outbound = json!({
        "tag": "proxy",
        "protocol": "vless",
        "settings": {
            "vnext": [
                {
                    "address": host,
                    "port": port,
                    "users": [Value::Object(user)]
                }
            ]
        }
    });
    if let Some(stream_settings) = build_stream_settings_from_url(&url, None)
        && let Some(object) = outbound.as_object_mut()
    {
        object.insert("streamSettings".to_string(), stream_settings);
    }
    Ok(outbound)
}

pub(crate) fn build_trojan_xray_outbound(raw: &str) -> Result<Value> {
    let url = Url::parse(raw).context("invalid trojan share link")?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("trojan host missing"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow!("trojan port missing"))?;
    let password = url.username();
    if password.trim().is_empty() {
        bail!("trojan password missing");
    }

    let mut outbound = json!({
        "tag": "proxy",
        "protocol": "trojan",
        "settings": {
            "servers": [
                {
                    "address": host,
                    "port": port,
                    "password": password
                }
            ]
        }
    });
    if let Some(stream_settings) = build_stream_settings_from_url(&url, Some("tls"))
        && let Some(object) = outbound.as_object_mut()
    {
        object.insert("streamSettings".to_string(), stream_settings);
    }
    Ok(outbound)
}

pub(crate) fn build_shadowsocks_xray_outbound(raw: &str) -> Result<Value> {
    let parsed = parse_shadowsocks_share_link(raw)?;
    Ok(json!({
        "tag": "proxy",
        "protocol": "shadowsocks",
        "settings": {
            "servers": [
                {
                    "address": parsed.host,
                    "port": parsed.port,
                    "method": parsed.method,
                    "password": parsed.password
                }
            ]
        }
    }))
}

pub(crate) fn build_stream_settings_from_url(
    url: &Url,
    default_security: Option<&str>,
) -> Option<Value> {
    let query = url.query_pairs().into_owned().collect::<HashMap<_, _>>();
    let network = query
        .get("type")
        .or_else(|| query.get("net"))
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "tcp".to_string());
    let security = query
        .get("security")
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .or_else(|| default_security.map(str::to_string))
        .unwrap_or_else(|| "none".to_string());

    let host = query
        .get("host")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let path = query
        .get("path")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let service_name = query
        .get("serviceName")
        .or_else(|| query.get("service_name"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| path.clone());

    let mut stream = serde_json::Map::new();
    stream.insert("network".to_string(), Value::String(network.clone()));
    let mut has_non_default_options = network != "tcp";
    if security != "none" {
        stream.insert("security".to_string(), Value::String(security.clone()));
        has_non_default_options = true;
    }

    match network.as_str() {
        "ws" => {
            let mut ws = serde_json::Map::new();
            if let Some(path) = path.as_ref() {
                ws.insert("path".to_string(), Value::String(path.clone()));
            }
            if let Some(host) = host.as_ref() {
                ws.insert("headers".to_string(), json!({ "Host": host }));
            }
            if !ws.is_empty() {
                stream.insert("wsSettings".to_string(), Value::Object(ws));
                has_non_default_options = true;
            }
        }
        "grpc" => {
            let service_name = service_name.unwrap_or_default();
            stream.insert(
                "grpcSettings".to_string(),
                json!({
                    "serviceName": service_name,
                    "multiMode": query_flag_true(&query, "multiMode")
                }),
            );
            has_non_default_options = true;
        }
        "httpupgrade" => {
            let mut settings = serde_json::Map::new();
            if let Some(host) = host.as_ref() {
                settings.insert("host".to_string(), Value::String(host.clone()));
            }
            if let Some(path) = path.as_ref() {
                settings.insert("path".to_string(), Value::String(path.clone()));
            }
            if !settings.is_empty() {
                stream.insert("httpupgradeSettings".to_string(), Value::Object(settings));
                has_non_default_options = true;
            }
        }
        _ => {}
    }

    if security == "tls" {
        let mut tls_settings = serde_json::Map::new();
        if let Some(server_name) = query
            .get("sni")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| host.clone())
            .or_else(|| url.host_str().map(str::to_string))
        {
            tls_settings.insert("serverName".to_string(), Value::String(server_name));
        }
        if query_flag_true(&query, "allowInsecure") || query_flag_true(&query, "insecure") {
            tls_settings.insert("allowInsecure".to_string(), Value::Bool(true));
        }
        if let Some(fingerprint) = query
            .get("fp")
            .or_else(|| query.get("fingerprint"))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            tls_settings.insert("fingerprint".to_string(), Value::String(fingerprint));
        }
        if let Some(alpn) = query
            .get("alpn")
            .map(|value| parse_alpn_csv(value))
            .filter(|items| !items.is_empty())
        {
            tls_settings.insert("alpn".to_string(), json!(alpn));
        }
        if !tls_settings.is_empty() {
            stream.insert("tlsSettings".to_string(), Value::Object(tls_settings));
            has_non_default_options = true;
        }
    } else if security == "reality" {
        let mut reality_settings = serde_json::Map::new();
        if let Some(server_name) = query
            .get("sni")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| host.clone())
            .or_else(|| url.host_str().map(str::to_string))
        {
            reality_settings.insert("serverName".to_string(), Value::String(server_name));
        }
        if let Some(fingerprint) = query
            .get("fp")
            .or_else(|| query.get("fingerprint"))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            reality_settings.insert("fingerprint".to_string(), Value::String(fingerprint));
        }
        if let Some(public_key) = query
            .get("pbk")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            reality_settings.insert("publicKey".to_string(), Value::String(public_key));
        }
        if let Some(short_id) = query
            .get("sid")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            reality_settings.insert("shortId".to_string(), Value::String(short_id));
        }
        if let Some(spider_x) = query
            .get("spx")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            reality_settings.insert("spiderX".to_string(), Value::String(spider_x));
        }
        if !reality_settings.is_empty() {
            stream.insert(
                "realitySettings".to_string(),
                Value::Object(reality_settings),
            );
            has_non_default_options = true;
        }
    }

    if has_non_default_options {
        Some(Value::Object(stream))
    } else {
        None
    }
}

pub(crate) fn query_flag_true(query: &HashMap<String, String>, key: &str) -> bool {
    query.get(key).is_some_and(|raw| {
        matches!(
            raw.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ForwardProxyAttemptWindowStats {
    pub(crate) attempts: i64,
    pub(crate) success_count: i64,
    pub(crate) avg_latency_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyWindowStatsResponse {
    pub(crate) attempts: i64,
    pub(crate) success_rate: Option<f64>,
    pub(crate) avg_latency_ms: Option<f64>,
}

impl From<ForwardProxyAttemptWindowStats> for ForwardProxyWindowStatsResponse {
    fn from(value: ForwardProxyAttemptWindowStats) -> Self {
        let success_rate = if value.attempts > 0 {
            Some((value.success_count as f64) / (value.attempts as f64))
        } else {
            None
        };
        Self {
            attempts: value.attempts,
            success_rate,
            avg_latency_ms: value.avg_latency_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyStatsResponse {
    pub(crate) one_minute: ForwardProxyWindowStatsResponse,
    pub(crate) fifteen_minutes: ForwardProxyWindowStatsResponse,
    pub(crate) one_hour: ForwardProxyWindowStatsResponse,
    pub(crate) one_day: ForwardProxyWindowStatsResponse,
    pub(crate) seven_days: ForwardProxyWindowStatsResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyNodeResponse {
    pub(crate) key: String,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) endpoint_url: Option<String>,
    pub(crate) weight: f64,
    pub(crate) penalized: bool,
    pub(crate) stats: ForwardProxyStatsResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyBindingNodeResponse {
    pub(crate) key: String,
    pub(crate) alias_keys: Vec<String>,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) protocol_label: String,
    pub(crate) penalized: bool,
    pub(crate) selectable: bool,
    pub(crate) last24h: Vec<ForwardProxyHourlyBucketResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxySettingsResponse {
    pub(crate) proxy_urls: Vec<String>,
    pub(crate) subscription_urls: Vec<String>,
    pub(crate) subscription_update_interval_secs: u64,
    pub(crate) nodes: Vec<ForwardProxyNodeResponse>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ForwardProxyHourlyStatsPoint {
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct ForwardProxyWeightHourlyStatsPoint {
    pub(crate) sample_count: i64,
    pub(crate) min_weight: f64,
    pub(crate) max_weight: f64,
    pub(crate) avg_weight: f64,
    pub(crate) last_weight: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyHourlyBucketResponse {
    pub(crate) bucket_start: String,
    pub(crate) bucket_end: String,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyWeightHourlyBucketResponse {
    pub(crate) bucket_start: String,
    pub(crate) bucket_end: String,
    pub(crate) sample_count: i64,
    pub(crate) min_weight: f64,
    pub(crate) max_weight: f64,
    pub(crate) avg_weight: f64,
    pub(crate) last_weight: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyLiveNodeResponse {
    pub(crate) key: String,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) endpoint_url: Option<String>,
    pub(crate) weight: f64,
    pub(crate) penalized: bool,
    pub(crate) stats: ForwardProxyStatsResponse,
    pub(crate) last24h: Vec<ForwardProxyHourlyBucketResponse>,
    pub(crate) weight24h: Vec<ForwardProxyWeightHourlyBucketResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyLiveStatsResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) bucket_seconds: i64,
    pub(crate) nodes: Vec<ForwardProxyLiveNodeResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyTimeseriesNodeResponse {
    pub(crate) key: String,
    pub(crate) source: String,
    pub(crate) display_name: String,
    pub(crate) endpoint_url: Option<String>,
    pub(crate) weight: f64,
    pub(crate) penalized: bool,
    pub(crate) buckets: Vec<ForwardProxyHourlyBucketResponse>,
    pub(crate) weight_buckets: Vec<ForwardProxyWeightHourlyBucketResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyTimeseriesResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) bucket_seconds: i64,
    pub(crate) effective_bucket: String,
    pub(crate) available_buckets: Vec<String>,
    pub(crate) nodes: Vec<ForwardProxyTimeseriesNodeResponse>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manager_with_manual_proxy() -> ForwardProxyManager {
        ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec!["http://jp-edge-01:8080".to_string()],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        )
    }

    #[test]
    fn binding_nodes_include_selectable_direct_with_protocol_label() {
        let manager = manager_with_manual_proxy();

        assert!(!manager.runtime.contains_key(FORWARD_PROXY_DIRECT_KEY));

        let direct = manager
            .binding_nodes()
            .into_iter()
            .find(|node| node.key == FORWARD_PROXY_DIRECT_KEY)
            .expect("missing direct binding node");

        assert_eq!(direct.display_name, FORWARD_PROXY_DIRECT_LABEL);
        assert_eq!(direct.protocol_label, "DIRECT");
        assert!(direct.selectable);
        assert!(!direct.penalized);
    }

    #[test]
    fn binding_nodes_include_legacy_vless_aliases() {
        let proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=tcp#东京节点";
        let normalized_proxy_url =
            normalize_share_link_scheme(proxy_url, "vless").expect("normalize vless url");
        let legacy_alias = {
            let parsed = Url::parse(&normalized_proxy_url).expect("parse normalized vless url");
            stable_forward_proxy_key(&canonical_share_link_identity(&parsed))
        };
        let canonical_key = normalize_single_proxy_key(proxy_url).expect("canonical vless key");
        assert_ne!(legacy_alias, canonical_key);

        let manager = ForwardProxyManager::new(
            ForwardProxySettings {
                proxy_urls: vec![proxy_url.to_string()],
                ..ForwardProxySettings::default()
            },
            Vec::new(),
        );

        let node = manager
            .binding_nodes()
            .into_iter()
            .find(|candidate| candidate.key == canonical_key)
            .expect("vless binding node should be present");

        assert!(node.alias_keys.contains(&legacy_alias));
    }

    #[test]
    fn automatic_selection_does_not_use_direct() {
        let mut manager = ForwardProxyManager::new(ForwardProxySettings::default(), Vec::new());

        assert!(manager.select_auto_proxy().is_none());
    }

    #[test]
    fn bound_group_network_failures_can_switch_from_direct_to_proxy() {
        let mut manager = manager_with_manual_proxy();
        let proxy_key = manager
            .endpoints
            .iter()
            .find(|endpoint| endpoint.key != FORWARD_PROXY_DIRECT_KEY)
            .map(|endpoint| endpoint.key.clone())
            .expect("missing non-direct endpoint");
        let scope = ForwardProxyRouteScope::BoundGroup {
            group_name: "latam".to_string(),
            bound_proxy_keys: vec![FORWARD_PROXY_DIRECT_KEY.to_string(), proxy_key.clone()],
        };
        manager.bound_group_runtime.insert(
            "latam".to_string(),
            BoundForwardProxyGroupState {
                current_proxy_key: Some(FORWARD_PROXY_DIRECT_KEY.to_string()),
                consecutive_network_failures: 0,
            },
        );

        manager.record_scope_result(
            &scope,
            FORWARD_PROXY_DIRECT_KEY,
            ForwardProxyRouteResultKind::NetworkFailure,
        );
        manager.record_scope_result(
            &scope,
            FORWARD_PROXY_DIRECT_KEY,
            ForwardProxyRouteResultKind::NetworkFailure,
        );
        manager.record_scope_result(
            &scope,
            FORWARD_PROXY_DIRECT_KEY,
            ForwardProxyRouteResultKind::NetworkFailure,
        );

        let group_state = manager
            .bound_group_runtime
            .get("latam")
            .expect("missing bound group state after failures");
        assert_eq!(
            group_state.current_proxy_key.as_deref(),
            Some(proxy_key.as_str())
        );
        assert_eq!(group_state.consecutive_network_failures, 0);
    }
}
