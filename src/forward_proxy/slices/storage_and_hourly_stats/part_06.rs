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
    let mut rows = query_live_pool_binding_window_rows(state, start_at, end_at).await?;
    rows.extend(
        query_archived_pool_binding_window_rows(
            state,
            start_at,
            end_at,
            pending_archive_file_paths,
        )
        .await?,
    );
    rows.extend(
        query_pending_pool_binding_window_rows(
            pending_archive_file_paths,
            start_at,
            end_at,
            pending_archive_rows,
        )
        .await?,
    );

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

async fn query_live_pool_binding_window_rows(
    state: &AppState,
    start_at: &str,
    end_at: &str,
) -> Result<Vec<PoolUpstreamBindingWindowStatsRow>> {
    let window_sql = pool_upstream_binding_window_sql();
    sqlx::query_as::<_, PoolUpstreamBindingWindowStatsRow>(&window_sql)
        .bind(start_at)
        .bind(end_at)
        .fetch_all(&state.pool)
        .await
        .with_context(|| {
            format!(
                "failed to query pool upstream binding window stats within [{start_at}, {end_at})"
            )
        })
}

fn pool_upstream_binding_window_sql() -> String {
    format!(
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
    )
}

async fn query_archived_pool_binding_window_rows(
    state: &AppState,
    start_at: &str,
    end_at: &str,
    pending_archive_file_paths: &[String],
) -> Result<Vec<PoolUpstreamBindingWindowStatsRow>> {
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
    archive_query
        .build_query_as::<PoolUpstreamBindingWindowStatsRow>()
        .fetch_all(&state.pool)
        .await
        .with_context(|| {
            format!(
                "failed to query cached pool upstream archive window stats within [{start_at}, {end_at})"
            )
        })
}

async fn query_pending_pool_binding_window_rows(
    pending_archive_file_paths: &[String],
    start_at: &str,
    end_at: &str,
    pending_archive_rows: Option<&[PendingPoolUpstreamBindingAttemptRow]>,
) -> Result<Vec<PoolUpstreamBindingWindowStatsRow>> {
    if let Some(pending_archive_rows) = pending_archive_rows {
        Ok(aggregate_pending_pool_upstream_binding_window_stats(
            pending_archive_rows,
            start_at,
            end_at,
        ))
    } else {
        query_pending_pool_upstream_binding_window_stats(
            pending_archive_file_paths,
            start_at,
            end_at,
        )
        .await
    }
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
