include!("storage_and_hourly_stats/part_01.rs");
include!("storage_and_hourly_stats/part_05.rs");
pub(crate) async fn load_pending_pool_upstream_binding_attempt_rows(
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

pub(crate) fn aggregate_pending_pool_upstream_binding_window_stats(
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

pub(crate) async fn query_pending_pool_upstream_binding_window_stats(
    pending_archive_file_paths: &[String],
    start_at: &str,
    end_at: &str,
) -> Result<Vec<PoolUpstreamBindingWindowStatsRow>> {
    let rows = load_pending_pool_upstream_binding_attempt_rows(
        pending_archive_file_paths,
        start_at,
        end_at,
    )
    .await?;
    Ok(aggregate_pending_pool_upstream_binding_window_stats(
        &rows, start_at, end_at,
    ))
}

pub(crate) fn aggregate_pending_pool_upstream_binding_hourly_stats(
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

pub(crate) async fn query_pending_pool_upstream_binding_hourly_stats(
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

pub(crate) async fn query_materialized_pool_upstream_binding_hourly_stats(
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

pub(crate) async fn query_pool_upstream_binding_window_stats(
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
    Ok(aggregate_pool_upstream_binding_window_rows(
        rows,
        &canonical_map,
        target_keys,
    ))
}

fn aggregate_pool_upstream_binding_window_rows(
    rows: Vec<PoolUpstreamBindingWindowStatsRow>,
    canonical_map: &HashMap<String, String>,
    target_keys: Option<&HashSet<String>>,
) -> HashMap<String, ForwardProxyAttemptWindowStats> {
    let mut grouped = HashMap::new();
    let mut latency_totals = HashMap::new();
    let mut latency_samples = HashMap::new();
    for row in rows {
        let Some(proxy_key) = resolve_pool_upstream_binding_target_key(
            &row.proxy_binding_key_snapshot,
            canonical_map,
            target_keys,
        ) else {
            continue;
        };
        record_pool_upstream_binding_window_stats(
            &mut grouped,
            &mut latency_totals,
            &mut latency_samples,
            &proxy_key,
            &row,
        );
    }
    for (proxy_key, stats) in &mut grouped {
        let sample_count = latency_samples.get(proxy_key).copied().unwrap_or_default();
        if stats.success_count > 0 && sample_count == stats.success_count {
            stats.avg_latency_ms = latency_totals
                .get(proxy_key)
                .copied()
                .map(|value| value / sample_count as f64);
        }
    }
    grouped
}

pub(crate) async fn query_pool_upstream_binding_hourly_stats(
    state: &AppState,
    range_start_epoch: i64,
    range_end_epoch: i64,
    target_keys: Option<&HashSet<String>>,
    pending_archive_file_paths: &[String],
    pending_archive_rows: Option<&[PendingPoolUpstreamBindingAttemptRow]>,
) -> Result<HashMap<String, HashMap<i64, ForwardProxyHourlyStatsPoint>>> {
    let (range_start_at, range_end_at) =
        pool_binding_range_bounds(range_start_epoch, range_end_epoch)?;
    let mut rows = query_live_pool_binding_hourly_rows(
        state,
        &range_start_at,
        &range_end_at,
        range_start_epoch,
        range_end_epoch,
    )
    .await?;
    rows.extend(
        query_archived_pool_binding_hourly_rows(
            state,
            pending_archive_file_paths,
            &range_start_at,
            &range_end_at,
            range_start_epoch,
            range_end_epoch,
        )
        .await?,
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
    Ok(aggregate_pool_upstream_binding_hourly_rows(
        rows,
        &canonical_map,
        target_keys,
        range_start_epoch,
        range_end_epoch,
    ))
}

fn pool_binding_range_bounds(
    range_start_epoch: i64,
    range_end_epoch: i64,
) -> Result<(String, String)> {
    let start = Utc
        .timestamp_opt(range_start_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid pool upstream binding bucket range start epoch"))?;
    let end = Utc
        .timestamp_opt(range_end_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid pool upstream binding bucket range end epoch"))?;
    Ok((
        db_occurred_at_lower_bound(start),
        db_occurred_at_lower_bound(end),
    ))
}

async fn query_live_pool_binding_hourly_rows(
    state: &AppState,
    range_start_at: &str,
    range_end_at: &str,
    range_start_epoch: i64,
    range_end_epoch: i64,
) -> Result<Vec<PoolUpstreamBindingHourlyStatsRow>> {
    let hourly_sql = format!(
        r#"
        SELECT proxy_binding_key_snapshot, bucket_start_epoch,
            SUM(CASE WHEN status = '{success}' THEN 1 ELSE 0 END) AS success_count,
            SUM(CASE WHEN status != '{success}' THEN 1 ELSE 0 END) AS failure_count
        FROM (
            SELECT proxy_binding_key_snapshot, status,
                {bucket_start_epoch_sql} AS bucket_start_epoch
            FROM pool_upstream_request_attempts
            WHERE proxy_binding_key_snapshot IS NOT NULL
              AND finished_at IS NOT NULL
              AND status != '{budget_exhausted}'
              AND occurred_at >= ?1 AND occurred_at < ?2
        )
        WHERE bucket_start_epoch >= ?3 AND bucket_start_epoch < ?4
        GROUP BY proxy_binding_key_snapshot, bucket_start_epoch
        "#,
        bucket_start_epoch_sql = POOL_UPSTREAM_BINDING_BUCKET_START_EPOCH_SQL,
        success = POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        budget_exhausted = POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
    );
    sqlx::query_as::<_, PoolUpstreamBindingHourlyStatsRow>(&hourly_sql)
        .bind(range_start_at)
        .bind(range_end_at)
        .bind(range_start_epoch)
        .bind(range_end_epoch)
        .fetch_all(&state.pool)
        .await
        .with_context(|| {
            format!(
                "failed to query pool upstream binding hourly stats within [{range_start_epoch}, {range_end_epoch})"
            )
        })
}

async fn query_archived_pool_binding_hourly_rows(
    state: &AppState,
    pending_archive_file_paths: &[String],
    range_start_at: &str,
    range_end_at: &str,
    range_start_epoch: i64,
    range_end_epoch: i64,
) -> Result<Vec<PoolUpstreamBindingHourlyStatsRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT proxy_binding_key_snapshot, bucket_start_epoch,
            SUM(is_success) AS success_count,
            SUM(CASE WHEN is_success = 0 THEN 1 ELSE 0 END) AS failure_count
        FROM (
            SELECT proxy_binding_key_snapshot, is_success, "#,
    );
    query.push(POOL_UPSTREAM_BINDING_BUCKET_START_EPOCH_SQL);
    query.push(
        " AS bucket_start_epoch FROM pool_upstream_node_health_archive WHERE occurred_at >= ",
    );
    query.push_bind(range_start_at);
    query.push(" AND occurred_at < ");
    query.push_bind(range_end_at);
    if !pending_archive_file_paths.is_empty() {
        query.push(" AND archive_file_path NOT IN (");
        let mut separated = query.separated(", ");
        for path in pending_archive_file_paths {
            separated.push_bind(path);
        }
        query.push(")");
    }
    query.push(") WHERE bucket_start_epoch >= ");
    query.push_bind(range_start_epoch);
    query.push(" AND bucket_start_epoch < ");
    query.push_bind(range_end_epoch);
    query.push(" GROUP BY proxy_binding_key_snapshot, bucket_start_epoch");
    query
        .build_query_as::<PoolUpstreamBindingHourlyStatsRow>()
        .fetch_all(&state.pool)
        .await
        .with_context(|| {
            format!(
                "failed to query cached pool upstream archive hourly stats within [{range_start_epoch}, {range_end_epoch})"
            )
        })
}

fn aggregate_pool_upstream_binding_hourly_rows(
    rows: Vec<PoolUpstreamBindingHourlyStatsRow>,
    canonical_map: &HashMap<String, String>,
    target_keys: Option<&HashSet<String>>,
    range_start_epoch: i64,
    range_end_epoch: i64,
) -> HashMap<String, HashMap<i64, ForwardProxyHourlyStatsPoint>> {
    let mut grouped = HashMap::new();
    for row in rows {
        let Some(proxy_key) = resolve_pool_upstream_binding_target_key(
            &row.proxy_binding_key_snapshot,
            canonical_map,
            target_keys,
        ) else {
            continue;
        };
        if (range_start_epoch..range_end_epoch).contains(&row.bucket_start_epoch) {
            record_pool_upstream_binding_hourly_stats(
                &mut grouped,
                proxy_key,
                row.bucket_start_epoch,
                row.success_count,
                row.failure_count,
            );
        }
    }
    grouped
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

    let (settings, runtime_rows, runtime_health_key_by_proxy_key) =
        load_forward_proxy_runtime_response_data(state).await;
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

async fn load_forward_proxy_runtime_response_data(
    state: &AppState,
) -> (
    ForwardProxySettings,
    Vec<ForwardProxyRuntimeState>,
    HashMap<String, String>,
) {
    let manager = state.forward_proxy.lock().await;
    let mut runtime_rows = manager.snapshot_runtime().into_iter().collect::<Vec<_>>();
    ensure_owner_facing_direct_runtime_row(
        &mut runtime_rows,
        manager.algo,
        manager.settings.insert_direct,
    );
    let runtime_health_keys = runtime_rows
        .iter()
        .map(|runtime| {
            let health_key = manager
                .canonicalize_bound_proxy_key(&runtime.proxy_key, None)
                .unwrap_or_else(|| runtime.proxy_key.clone());
            (runtime.proxy_key.clone(), health_key)
        })
        .collect();
    (manager.settings.clone(), runtime_rows, runtime_health_keys)
}

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyLatencyTestNodeProgress {
    pub(crate) key: String,
    pub(crate) display_name: String,
    pub(crate) round: usize,
    pub(crate) total_rounds: usize,
    pub(crate) completed_rounds: usize,
    pub(crate) success_count: usize,
    pub(crate) attempt_count: usize,
    pub(crate) average_latency_ms: Option<u64>,
    pub(crate) egress_ip: ForwardProxyLatencyProbeTargetResult,
    pub(crate) oauth_upstream: ForwardProxyLatencyProbeTargetResult,
    pub(crate) codex_responses: ForwardProxyLatencyProbeTargetResult,
    pub(crate) all_targets_ok: bool,
    pub(crate) failed_targets: Vec<&'static str>,
    pub(crate) done: bool,
    pub(crate) timed_out: bool,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForwardProxyLatencyTestStreamEvent {
    kind: &'static str,
    node: ForwardProxyLatencyTestNodeProgress,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct ForwardProxyLatencyAccumulator {
    pub(crate) total_latency_ms: f64,
    pub(crate) success_count: usize,
    pub(crate) completed_rounds: usize,
    pub(crate) egress_ip_failures: usize,
    pub(crate) oauth_upstream_failures: usize,
    pub(crate) codex_responses_failures: usize,
    pub(crate) last_egress_ip: Option<ForwardProxyLatencyProbeTargetResult>,
    pub(crate) last_oauth_upstream: Option<ForwardProxyLatencyProbeTargetResult>,
    pub(crate) last_codex_responses: Option<ForwardProxyLatencyProbeTargetResult>,
}

impl ForwardProxyLatencyAccumulator {
    pub(crate) fn record_round(
        &mut self,
        egress_ip: &ForwardProxyLatencyProbeTargetResult,
        oauth_upstream: &ForwardProxyLatencyProbeTargetResult,
        codex_responses: &ForwardProxyLatencyProbeTargetResult,
    ) {
        self.completed_rounds += 1;
        for latency_ms in [
            egress_ip.success_latency(),
            oauth_upstream.success_latency(),
            codex_responses.success_latency(),
        ]
        .into_iter()
        .flatten()
        {
            self.total_latency_ms += latency_ms;
            self.success_count += 1;
        }
        if !egress_ip.ok {
            self.egress_ip_failures += 1;
        }
        if !oauth_upstream.ok {
            self.oauth_upstream_failures += 1;
        }
        if !codex_responses.ok {
            self.codex_responses_failures += 1;
        }
        preserve_forward_proxy_latency_target_result(
            &mut self.last_egress_ip,
            self.egress_ip_failures,
            egress_ip,
        );
        preserve_forward_proxy_latency_target_result(
            &mut self.last_oauth_upstream,
            self.oauth_upstream_failures,
            oauth_upstream,
        );
        preserve_forward_proxy_latency_target_result(
            &mut self.last_codex_responses,
            self.codex_responses_failures,
            codex_responses,
        );
    }

    pub(crate) fn average_latency_ms(&self) -> Option<u64> {
        if self.success_count == 0 {
            return None;
        }
        Some((self.total_latency_ms / self.success_count as f64).round() as u64)
    }

    pub(crate) fn all_targets_ok(&self) -> bool {
        self.completed_rounds > 0
            && self.egress_ip_failures == 0
            && self.oauth_upstream_failures == 0
            && self.codex_responses_failures == 0
    }

    pub(crate) fn failed_targets(&self) -> Vec<&'static str> {
        let mut targets = Vec::new();
        if self.egress_ip_failures > 0 {
            targets.push(FORWARD_PROXY_LATENCY_TARGET_EGRESS_IP);
        }
        if self.oauth_upstream_failures > 0 {
            targets.push(FORWARD_PROXY_LATENCY_TARGET_OAUTH_UPSTREAM);
        }
        if self.codex_responses_failures > 0 {
            targets.push(FORWARD_PROXY_LATENCY_TARGET_CODEX_RESPONSES);
        }
        targets
    }
}

pub(crate) fn preserve_forward_proxy_latency_target_result(
    slot: &mut Option<ForwardProxyLatencyProbeTargetResult>,
    failure_count: usize,
    result: &ForwardProxyLatencyProbeTargetResult,
) {
    if !result.ok || failure_count == 0 || slot.is_none() {
        *slot = Some(result.clone());
    }
}

pub(crate) fn accumulated_forward_proxy_latency_target_results(
    accumulator: &ForwardProxyLatencyAccumulator,
    egress_ip: &ForwardProxyLatencyProbeTargetResult,
    oauth_upstream: &ForwardProxyLatencyProbeTargetResult,
    codex_responses: &ForwardProxyLatencyProbeTargetResult,
) -> (
    ForwardProxyLatencyProbeTargetResult,
    ForwardProxyLatencyProbeTargetResult,
    ForwardProxyLatencyProbeTargetResult,
) {
    (
        accumulator
            .last_egress_ip
            .clone()
            .unwrap_or_else(|| egress_ip.clone()),
        accumulator
            .last_oauth_upstream
            .clone()
            .unwrap_or_else(|| oauth_upstream.clone()),
        accumulator
            .last_codex_responses
            .clone()
            .unwrap_or_else(|| codex_responses.clone()),
    )
}

pub(crate) fn forward_proxy_manual_latency_round_timeout() -> Duration {
    Duration::from_secs(FORWARD_PROXY_MANUAL_LATENCY_ROUND_TIMEOUT_SECS)
}

pub(crate) fn forward_proxy_manual_latency_single_timeout() -> Duration {
    Duration::from_secs(FORWARD_PROXY_MANUAL_LATENCY_SINGLE_TIMEOUT_SECS)
}

pub(crate) fn forward_proxy_manual_latency_round_count() -> usize {
    FORWARD_PROXY_MANUAL_LATENCY_TEST_ROUNDS
}

pub(crate) fn forward_proxy_latency_breadth_first_schedule(
    node_count: usize,
    total_rounds: usize,
) -> Vec<(usize, usize)> {
    (1..=total_rounds)
        .flat_map(|round| (0..node_count).map(move |node_index| (round, node_index)))
        .collect()
}

pub(crate) fn forward_proxy_latency_test_event(
    event_name: &'static str,
    payload: &ForwardProxyLatencyTestStreamEvent,
) -> Option<Event> {
    match Event::default().event(event_name).json_data(payload) {
        Ok(event) => Some(event),
        Err(err) => {
            warn!(?err, "failed to serialize forward proxy latency test event");
            None
        }
    }
}

pub(crate) fn resolve_forward_proxy_endpoint_for_test(
    manager: &ForwardProxyManager,
    proxy_key: &str,
) -> Option<ForwardProxyEndpoint> {
    if proxy_key == FORWARD_PROXY_DIRECT_KEY {
        return Some(ForwardProxyEndpoint::direct());
    }
    manager
        .endpoints
        .iter()
        .find(|endpoint| endpoint.key == proxy_key)
        .cloned()
        .or_else(|| {
            manager
                .runtime
                .get(proxy_key)
                .and_then(|runtime| runtime.endpoint_url.as_deref())
                .and_then(parse_forward_proxy_entry)
                .map(|parsed| ForwardProxyEndpoint {
                    key: proxy_key.to_string(),
                    source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
                    display_name: parsed.display_name,
                    protocol: parsed.protocol,
                    endpoint_url: parsed.endpoint_url,
                    raw_url: Some(parsed.normalized),
                })
        })
}
pub(crate) async fn load_forward_proxy_endpoints_for_latency_test(
    state: &AppState,
    proxy_keys: &[String],
) -> Result<Vec<ForwardProxyEndpoint>, (StatusCode, String)> {
    let manager = state.forward_proxy.lock().await;
    let mut endpoints = Vec::with_capacity(proxy_keys.len());
    let mut missing = Vec::new();
    let mut seen = HashSet::new();
    for proxy_key in proxy_keys {
        let normalized = proxy_key.trim();
        if normalized.is_empty() || !seen.insert(normalized.to_string()) {
            continue;
        }
        match resolve_forward_proxy_endpoint_for_test(&manager, normalized) {
            Some(endpoint) => endpoints.push(endpoint),
            None => missing.push(normalized.to_string()),
        }
    }
    if endpoints.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "no proxy keys provided".to_string(),
        ));
    }
    if !missing.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("forward proxy node not found: {}", missing.join(", ")),
        ));
    }
    Ok(endpoints)
}

pub(crate) async fn timed_forward_proxy_egress_ip_probe(
    state: &AppState,
    selected_proxy: &SelectedForwardProxy,
    client: &Client,
    request_timeout: Duration,
) -> ForwardProxyLatencyProbeTargetResult {
    let started = Instant::now();
    match fetch_forward_proxy_egress_ip(client, request_timeout).await {
        Ok(ip) => {
            if let Err(err) =
                persist_forward_proxy_egress_ip_result(&state.pool, selected_proxy, Some(&ip), None)
                    .await
            {
                warn!(
                    proxy_key_ref = %forward_proxy_log_ref(&selected_proxy.key),
                    error = %err,
                    "failed to persist manual latency egress IP result"
                );
            }
            ForwardProxyLatencyProbeTargetResult {
                ok: true,
                latency_ms: Some(elapsed_ms(started)),
                ip: Some(ip),
                http_status: None,
                error: None,
            }
        }
        Err(err) => {
            if let Err(persist_err) = persist_forward_proxy_egress_ip_result(
                &state.pool,
                selected_proxy,
                None,
                Some(&err.to_string()),
            )
            .await
            {
                warn!(
                    proxy_key_ref = %forward_proxy_log_ref(&selected_proxy.key),
                    error = %persist_err,
                    "failed to persist manual latency egress IP failure"
                );
            }
            ForwardProxyLatencyProbeTargetResult {
                ok: false,
                latency_ms: None,
                ip: None,
                http_status: None,
                error: Some(err.to_string()),
            }
        }
    }
}

pub(crate) async fn timed_forward_proxy_oauth_upstream_probe(
    client: &Client,
    request_timeout: Duration,
) -> ForwardProxyLatencyProbeTargetResult {
    let started = Instant::now();
    let target = match oauth_codex_latency_probe_target("models") {
        Ok(target) => target,
        Err(err) => {
            return ForwardProxyLatencyProbeTargetResult {
                ok: false,
                latency_ms: None,
                ip: None,
                http_status: None,
                error: Some(err.to_string()),
            };
        }
    };

    let result = timeout(request_timeout, client.get(target).send()).await;
    match result {
        Ok(Ok(response)) => {
            let status = response.status();
            let ok = is_manual_latency_probe_reachable_status(status);
            ForwardProxyLatencyProbeTargetResult {
                ok,
                latency_ms: ok.then(|| elapsed_ms(started)),
                ip: None,
                http_status: Some(status.as_u16()),
                error: if ok {
                    None
                } else {
                    Some(format!("OAuth upstream returned status {status}"))
                },
            }
        }
        Ok(Err(err)) => ForwardProxyLatencyProbeTargetResult {
            ok: false,
            latency_ms: None,
            ip: None,
            http_status: None,
            error: Some(err.to_string()),
        },
        Err(_) => ForwardProxyLatencyProbeTargetResult {
            ok: false,
            latency_ms: None,
            ip: None,
            http_status: None,
            error: Some(format!(
                "manual latency test round timed out after {}s",
                timeout_seconds_for_message(request_timeout)
            )),
        },
    }
}

pub(crate) fn oauth_codex_latency_probe_target(path_segment: &str) -> Result<Url> {
    let mut target = oauth_bridge::oauth_codex_upstream_base_url()?;
    target.set_path(&format!(
        "{}/{}",
        target.path().trim_end_matches('/'),
        path_segment.trim_start_matches('/')
    ));
    Ok(target)
}

pub(crate) async fn timed_forward_proxy_codex_responses_probe(
    client: &Client,
    request_timeout: Duration,
) -> ForwardProxyLatencyProbeTargetResult {
    let started = Instant::now();
    let target = match oauth_codex_latency_probe_target("responses") {
        Ok(target) => target,
        Err(err) => {
            return ForwardProxyLatencyProbeTargetResult {
                ok: false,
                latency_ms: None,
                ip: None,
                http_status: None,
                error: Some(err.to_string()),
            };
        }
    };

    let result = timeout(request_timeout, client.get(target).send()).await;
    match result {
        Ok(Ok(response)) => {
            let status = response.status();
            let ok = is_manual_latency_probe_reachable_status(status);
            ForwardProxyLatencyProbeTargetResult {
                ok,
                latency_ms: ok.then(|| elapsed_ms(started)),
                ip: None,
                http_status: Some(status.as_u16()),
                error: if ok {
                    None
                } else {
                    Some(format!("Codex responses upstream returned status {status}"))
                },
            }
        }
        Ok(Err(err)) => ForwardProxyLatencyProbeTargetResult {
            ok: false,
            latency_ms: None,
            ip: None,
            http_status: None,
            error: Some(err.to_string()),
        },
        Err(_) => ForwardProxyLatencyProbeTargetResult {
            ok: false,
            latency_ms: None,
            ip: None,
            http_status: None,
            error: Some(format!(
                "manual latency test round timed out after {}s",
                timeout_seconds_for_message(request_timeout)
            )),
        },
    }
}

pub(crate) fn failed_forward_proxy_latency_targets(
    egress_ip: &ForwardProxyLatencyProbeTargetResult,
    oauth_upstream: &ForwardProxyLatencyProbeTargetResult,
    codex_responses: &ForwardProxyLatencyProbeTargetResult,
) -> Vec<&'static str> {
    let mut targets = Vec::new();
    if !egress_ip.ok {
        targets.push(FORWARD_PROXY_LATENCY_TARGET_EGRESS_IP);
    }
    if !oauth_upstream.ok {
        targets.push(FORWARD_PROXY_LATENCY_TARGET_OAUTH_UPSTREAM);
    }
    if !codex_responses.ok {
        targets.push(FORWARD_PROXY_LATENCY_TARGET_CODEX_RESPONSES);
    }
    targets
}

pub(crate) fn forward_proxy_latency_target_timed_out(
    result: &ForwardProxyLatencyProbeTargetResult,
) -> bool {
    let Some(error) = result.error.as_deref() else {
        return false;
    };
    error.contains("timed out") || error.contains("budget exhausted")
}

pub(crate) fn is_manual_latency_probe_reachable_status(status: StatusCode) -> bool {
    status.as_u16() < 500
}

pub(crate) async fn run_forward_proxy_latency_test_round(
    state: Arc<AppState>,
    endpoint: &ForwardProxyEndpoint,
    accumulator: &mut ForwardProxyLatencyAccumulator,
    round: usize,
    single_started: Instant,
    is_single_node_test: bool,
) -> ForwardProxyLatencyTestNodeProgress {
    let selected_proxy = SelectedForwardProxy::from_endpoint(endpoint);
    let total_timeout = if is_single_node_test {
        forward_proxy_manual_latency_single_timeout()
    } else {
        Duration::from_secs(u64::MAX / 4)
    };
    let remaining_total = remaining_timeout_budget(total_timeout, single_started.elapsed())
        .filter(|remaining| !remaining.is_zero());
    let round_timeout = remaining_total
        .map(|remaining| remaining.min(forward_proxy_manual_latency_round_timeout()))
        .unwrap_or_else(|| Duration::from_secs(0));
    let mut egress_ip = ForwardProxyLatencyProbeTargetResult {
        ok: false,
        latency_ms: None,
        ip: None,
        http_status: None,
        error: Some("manual latency test budget exhausted".to_string()),
    };
    let mut oauth_upstream = egress_ip.clone();
    let mut codex_responses = egress_ip.clone();
    let mut round_started = Instant::now();

    if !round_timeout.is_zero() {
        match resolve_forward_proxy_probe_endpoint_url(
            state.as_ref(),
            endpoint,
            round_timeout,
            Some(&state.shutdown),
        )
        .await
        {
            Ok((endpoint_url, temporary_xray_key)) => {
                let client_result = state
                    .http_clients
                    .client_for_forward_proxy(endpoint_url.as_ref());
                match client_result {
                    Ok(client) => {
                        round_started = Instant::now();
                        egress_ip = timed_forward_proxy_egress_ip_probe(
                            state.as_ref(),
                            &selected_proxy,
                            &client,
                            round_timeout,
                        )
                        .await;
                        let remaining_round =
                            remaining_timeout_budget(round_timeout, round_started.elapsed())
                                .filter(|remaining| !remaining.is_zero());
                        oauth_upstream = match remaining_round {
                            Some(remaining) => {
                                timed_forward_proxy_oauth_upstream_probe(&client, remaining).await
                            }
                            None => ForwardProxyLatencyProbeTargetResult {
                                ok: false,
                                latency_ms: None,
                                ip: None,
                                http_status: None,
                                error: Some(
                                    "manual latency test round budget exhausted".to_string(),
                                ),
                            },
                        };
                        let remaining_round =
                            remaining_timeout_budget(round_timeout, round_started.elapsed())
                                .filter(|remaining| !remaining.is_zero());
                        codex_responses = match remaining_round {
                            Some(remaining) => {
                                timed_forward_proxy_codex_responses_probe(&client, remaining).await
                            }
                            None => ForwardProxyLatencyProbeTargetResult {
                                ok: false,
                                latency_ms: None,
                                ip: None,
                                http_status: None,
                                error: Some(
                                    "manual latency test round budget exhausted".to_string(),
                                ),
                            },
                        };
                    }
                    Err(err) => {
                        egress_ip.error = Some(err.to_string());
                        oauth_upstream.error = Some(err.to_string());
                        codex_responses.error = Some(err.to_string());
                    }
                }
                if let Some(temp_key) = temporary_xray_key {
                    let mut supervisor = state.xray_supervisor.lock().await;
                    supervisor.remove_instance(&temp_key).await;
                }
            }
            Err(err) => {
                egress_ip.error = Some(err.to_string());
                oauth_upstream.error = Some(err.to_string());
                codex_responses.error = Some(err.to_string());
            }
        }
    }

    accumulator.record_round(&egress_ip, &oauth_upstream, &codex_responses);
    let round_success = egress_ip.ok && oauth_upstream.ok && codex_responses.ok;
    let round_latency_ms = {
        let samples = [
            egress_ip.success_latency(),
            oauth_upstream.success_latency(),
            codex_responses.success_latency(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        if samples.is_empty() {
            None
        } else {
            Some(samples.iter().sum::<f64>() / samples.len() as f64)
        }
    };
    record_forward_proxy_attempt(
        state,
        selected_proxy,
        round_success,
        round_latency_ms.or_else(|| Some(elapsed_ms(round_started))),
        if round_success {
            None
        } else {
            Some(FORWARD_PROXY_FAILURE_SEND_ERROR)
        },
        true,
    )
    .await;
    let done = round >= forward_proxy_manual_latency_round_count()
        || (is_single_node_test
            && timeout_budget_exhausted(
                forward_proxy_manual_latency_single_timeout(),
                single_started.elapsed(),
            ));
    let all_targets_ok = accumulator.all_targets_ok();
    let failed_targets = accumulator.failed_targets();
    let (display_egress_ip, display_oauth_upstream, display_codex_responses) =
        accumulated_forward_proxy_latency_target_results(
            accumulator,
            &egress_ip,
            &oauth_upstream,
            &codex_responses,
        );
    let timed_out = done
        && [&egress_ip, &oauth_upstream, &codex_responses]
            .into_iter()
            .any(forward_proxy_latency_target_timed_out);
    let message = if !failed_targets.is_empty() {
        format!("failed targets: {}", failed_targets.join(", "))
    } else if let Some(avg) = accumulator.average_latency_ms() {
        format!(
            "{avg} ms from {}/{} successful samples",
            accumulator.success_count,
            accumulator.completed_rounds * FORWARD_PROXY_MANUAL_LATENCY_TARGET_COUNT
        )
    } else if done {
        "timeout".to_string()
    } else {
        "waiting for first successful sample".to_string()
    };

    ForwardProxyLatencyTestNodeProgress {
        key: endpoint.key.clone(),
        display_name: endpoint.display_name.clone(),
        round,
        total_rounds: forward_proxy_manual_latency_round_count(),
        completed_rounds: accumulator.completed_rounds,
        success_count: accumulator.success_count,
        attempt_count: accumulator.completed_rounds * FORWARD_PROXY_MANUAL_LATENCY_TARGET_COUNT,
        average_latency_ms: accumulator.average_latency_ms(),
        egress_ip: display_egress_ip,
        oauth_upstream: display_oauth_upstream,
        codex_responses: display_codex_responses,
        all_targets_ok,
        failed_targets,
        done,
        timed_out,
        message,
    }
}

pub(crate) fn forward_proxy_latency_timeout_progress(
    endpoint: &ForwardProxyEndpoint,
    accumulator: &ForwardProxyLatencyAccumulator,
) -> ForwardProxyLatencyTestNodeProgress {
    let timeout_result = ForwardProxyLatencyProbeTargetResult {
        ok: false,
        latency_ms: None,
        ip: None,
        http_status: None,
        error: Some("manual latency test budget exhausted".to_string()),
    };
    let (egress_ip, oauth_upstream, codex_responses) = if accumulator.completed_rounds == 0 {
        (
            timeout_result.clone(),
            timeout_result.clone(),
            timeout_result.clone(),
        )
    } else {
        (
            accumulator
                .last_egress_ip
                .clone()
                .unwrap_or_else(|| timeout_result.clone()),
            accumulator
                .last_oauth_upstream
                .clone()
                .unwrap_or_else(|| timeout_result.clone()),
            accumulator
                .last_codex_responses
                .clone()
                .unwrap_or_else(|| timeout_result.clone()),
        )
    };
    let failed_targets = if accumulator.completed_rounds == 0 {
        failed_forward_proxy_latency_targets(&timeout_result, &timeout_result, &timeout_result)
    } else {
        accumulator.failed_targets()
    };
    let all_targets_ok = accumulator.all_targets_ok();

    ForwardProxyLatencyTestNodeProgress {
        key: endpoint.key.clone(),
        display_name: endpoint.display_name.clone(),
        round: accumulator
            .completed_rounds
            .min(forward_proxy_manual_latency_round_count()),
        total_rounds: forward_proxy_manual_latency_round_count(),
        completed_rounds: accumulator.completed_rounds,
        success_count: accumulator.success_count,
        attempt_count: accumulator.completed_rounds * FORWARD_PROXY_MANUAL_LATENCY_TARGET_COUNT,
        average_latency_ms: accumulator.average_latency_ms(),
        egress_ip,
        oauth_upstream,
        codex_responses,
        all_targets_ok,
        failed_targets: failed_targets.clone(),
        done: true,
        timed_out: !all_targets_ok,
        message: if !failed_targets.is_empty() {
            format!("failed targets: {}", failed_targets.join(", "))
        } else {
            accumulator
                .average_latency_ms()
                .map(|avg| {
                    format!(
                        "{avg} ms from {}/{} successful samples",
                        accumulator.success_count,
                        accumulator.completed_rounds * FORWARD_PROXY_MANUAL_LATENCY_TARGET_COUNT
                    )
                })
                .unwrap_or_else(|| "timeout".to_string())
        },
    }
}

pub(crate) fn stream_forward_proxy_latency_tests(
    state: Arc<AppState>,
    endpoints: Vec<ForwardProxyEndpoint>,
    is_single_node_test: bool,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let stream = stream::unfold(
        (
            state,
            endpoints,
            Vec::<ForwardProxyLatencyAccumulator>::new(),
            1usize,
            0usize,
            Vec::<Instant>::new(),
        ),
        move |(state, endpoints, mut accumulators, mut round, mut index, mut started_at)| async move {
            if accumulators.is_empty() {
                accumulators = vec![ForwardProxyLatencyAccumulator::default(); endpoints.len()];
                started_at = (0..endpoints.len()).map(|_| Instant::now()).collect();
            }
            if round > forward_proxy_manual_latency_round_count() || endpoints.is_empty() {
                return None;
            }
            if index < endpoints.len()
                && is_single_node_test
                && timeout_budget_exhausted(
                    forward_proxy_manual_latency_single_timeout(),
                    started_at[index].elapsed(),
                )
            {
                let endpoint = endpoints[index].clone();
                let progress =
                    forward_proxy_latency_timeout_progress(&endpoint, &accumulators[index]);
                index += 1;
                round = forward_proxy_manual_latency_round_count() + 1;
                let payload = ForwardProxyLatencyTestStreamEvent {
                    kind: "completed",
                    node: progress,
                };
                let event = forward_proxy_latency_test_event("completed", &payload).map(Ok);
                return event.map(|event| {
                    (
                        event,
                        (state, endpoints, accumulators, round, index, started_at),
                    )
                });
            }
            if index >= endpoints.len() {
                round += 1;
                index = 0;
                if round > forward_proxy_manual_latency_round_count() {
                    return None;
                }
            }
            let endpoint = endpoints[index].clone();
            let progress = run_forward_proxy_latency_test_round(
                state.clone(),
                &endpoint,
                &mut accumulators[index],
                round,
                started_at[index],
                is_single_node_test,
            )
            .await;
            let event_name = if progress.done {
                "completed"
            } else {
                "progress"
            };
            let payload = ForwardProxyLatencyTestStreamEvent {
                kind: event_name,
                node: progress,
            };
            index += 1;
            let event = forward_proxy_latency_test_event(event_name, &payload).map(Ok);
            event.map(|event| {
                (
                    event,
                    (state, endpoints, accumulators, round, index, started_at),
                )
            })
        },
    );
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

pub(crate) async fn stream_forward_proxy_node_latency_test(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(proxy_key): AxumPath<String>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)>
{
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    let endpoints =
        load_forward_proxy_endpoints_for_latency_test(state.as_ref(), &[proxy_key]).await?;
    Ok(stream_forward_proxy_latency_tests(state, endpoints, true))
}

pub(crate) async fn stream_forward_proxy_nodes_latency_test(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)>
{
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }
    let proxy_keys = parse_forward_proxy_nodes_latency_test_keys(uri.query().unwrap_or_default());
    let endpoints =
        load_forward_proxy_endpoints_for_latency_test(state.as_ref(), &proxy_keys).await?;
    Ok(stream_forward_proxy_latency_tests(state, endpoints, false))
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

    let context = SubscriptionValidationContext {
        cancellation: cancellation.clone(),
        endpoint_count,
        concurrency,
        attempts,
        attempt_timeout,
        validation_timeout,
        validation_started,
    };
    await_subscription_validation_results(&context, &mut rx).await
}

struct SubscriptionValidationContext {
    cancellation: CancellationToken,
    endpoint_count: usize,
    concurrency: usize,
    attempts: usize,
    attempt_timeout: Duration,
    validation_timeout: Duration,
    validation_started: Instant,
}

async fn await_subscription_validation_results(
    context: &SubscriptionValidationContext,
    rx: &mut mpsc::Receiver<Result<f64, String>>,
) -> Result<f64> {
    let mut completed = 0usize;
    let mut last_error = None;
    while completed < context.endpoint_count {
        let Some(remaining_timeout) = remaining_timeout_budget(
            context.validation_timeout,
            context.validation_started.elapsed(),
        ) else {
            context.cancellation.cancel();
            return Err(timeout_error_for_duration(context.validation_timeout));
        };
        let result = match timeout(remaining_timeout, rx.recv()).await {
            Ok(Some(result)) => result,
            Ok(None) => break,
            Err(_) => {
                context.cancellation.cancel();
                return Err(timeout_error_for_duration(context.validation_timeout));
            }
        };
        completed += 1;
        match result {
            Ok(latency_ms) => {
                context.cancellation.cancel();
                return Ok(latency_ms);
            }
            Err(err) => last_error = Some(err),
        }
    }
    let mut message = format!(
        "subscription validation scanned {} proxy entries with concurrency {}, {} attempts per entry, and {}s per attempt; no entry passed validation",
        context.endpoint_count,
        context.concurrency,
        context.attempts,
        timeout_seconds_for_message(context.attempt_timeout)
    );
    if let Some(err) = last_error {
        message.push_str(&format!("; last error: {err}"));
    }
    bail!(message)
}

pub(crate) struct ProbeCancellationGuard(CancellationToken);

impl Drop for ProbeCancellationGuard {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub(crate) async fn probe_subscription_endpoint_with_retries(
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
