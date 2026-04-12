use super::*;

#[path = "hourly_rollup_support.rs"]
mod archive_hourly_rollup_support;
pub(crate) use archive_hourly_rollup_support::*;

pub(crate) async fn mark_retention_archived_hourly_rollup_targets_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    invocation_rows: &[InvocationHourlySourceRecord],
    forward_proxy_rows: &[ForwardProxyAttemptHourlySourceRecord],
) -> Result<()> {
    match dataset {
        "codex_invocations" => {
            mark_invocation_hourly_rollup_buckets_materialized_tx(tx, invocation_rows).await?;
        }
        "forward_proxy_attempts" => {
            mark_forward_proxy_hourly_rollup_buckets_materialized_tx(tx, forward_proxy_rows)
                .await?;
        }
        _ => {}
    }
    Ok(())
}

pub(crate) async fn mark_archive_batch_historical_rollups_materialized_tx(
    tx: &mut SqliteConnection,
    dataset: &str,
    file_path: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET historical_rollups_materialized_at = datetime('now')
        WHERE dataset = ?1
          AND file_path = ?2
        "#,
    )
    .bind(dataset)
    .bind(file_path)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn update_archive_batch_coverage_bounds_tx(
    tx: &mut SqliteConnection,
    archive_batch_id: i64,
    coverage_start_at: Option<&str>,
    coverage_end_at: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET coverage_start_at = COALESCE(coverage_start_at, ?2),
            coverage_end_at = COALESCE(coverage_end_at, ?3)
        WHERE id = ?1
        "#,
    )
    .bind(archive_batch_id)
    .bind(coverage_start_at)
    .bind(coverage_end_at)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn mark_hourly_rollup_bucket_materialized_tx(
    tx: &mut SqliteConnection,
    target: &str,
    bucket_start_epoch: i64,
    source: &str,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO hourly_rollup_materialized_buckets (
            target,
            bucket_start_epoch,
            source,
            materialized_at
        )
        VALUES (?1, ?2, ?3, datetime('now'))
        ON CONFLICT(target, bucket_start_epoch, source) DO UPDATE SET
            materialized_at = datetime('now')
        "#,
    )
    .bind(target)
    .bind(bucket_start_epoch)
    .bind(source)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn mark_invocation_hourly_rollup_buckets_materialized_tx(
    tx: &mut SqliteConnection,
    rows: &[InvocationHourlySourceRecord],
) -> Result<()> {
    let mut overall_targets = HashSet::new();
    let mut sticky_targets = HashSet::new();
    for row in rows {
        let bucket_start_epoch = invocation_bucket_start_epoch(&row.occurred_at)?;
        overall_targets.insert((bucket_start_epoch, row.source.clone()));
        sticky_targets.insert(bucket_start_epoch);
    }

    let live_targets = load_live_invocation_bucket_targets_tx(tx, &overall_targets).await?;
    let live_proxy_buckets = live_targets
        .iter()
        .filter_map(|(bucket_start_epoch, source)| {
            (source == SOURCE_PROXY).then_some(*bucket_start_epoch)
        })
        .collect::<HashSet<_>>();

    for (bucket_start_epoch, source) in overall_targets {
        if live_targets.contains(&(bucket_start_epoch, source.clone())) {
            continue;
        }
        for target in [
            HOURLY_ROLLUP_TARGET_INVOCATIONS,
            HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
            HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
            HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
        ] {
            mark_hourly_rollup_bucket_materialized_tx(tx, target, bucket_start_epoch, &source)
                .await?;
        }
        if source == SOURCE_PROXY && !live_proxy_buckets.contains(&bucket_start_epoch) {
            mark_hourly_rollup_bucket_materialized_tx(
                tx,
                HOURLY_ROLLUP_TARGET_PROXY_PERF,
                bucket_start_epoch,
                SOURCE_PROXY,
            )
            .await?;
        }
    }

    for bucket_start_epoch in sticky_targets {
        if live_proxy_buckets.contains(&bucket_start_epoch) {
            continue;
        }
        mark_hourly_rollup_bucket_materialized_tx(
            tx,
            HOURLY_ROLLUP_TARGET_STICKY_KEYS,
            bucket_start_epoch,
            HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
        )
        .await?;
    }

    Ok(())
}

pub(crate) async fn load_live_invocation_bucket_targets_tx(
    tx: &mut SqliteConnection,
    bucket_targets: &HashSet<(i64, String)>,
) -> Result<HashSet<(i64, String)>> {
    if bucket_targets.is_empty() {
        return Ok(HashSet::new());
    }

    let min_bucket_epoch = bucket_targets
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .min()
        .ok_or_else(|| anyhow!("missing minimum invocation bucket epoch"))?;
    let max_bucket_epoch = bucket_targets
        .iter()
        .map(|(bucket_start_epoch, _)| *bucket_start_epoch)
        .max()
        .ok_or_else(|| anyhow!("missing maximum invocation bucket epoch"))?;
    let min_bucket_start = Utc
        .timestamp_opt(min_bucket_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid minimum invocation bucket epoch"))?;
    let max_bucket_end = Utc
        .timestamp_opt(max_bucket_epoch + 3_600, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid maximum invocation bucket epoch"))?;

    let rows = sqlx::query_as::<_, InvocationBucketPresenceRow>(
        r#"
        SELECT occurred_at, source
        FROM codex_invocations
        WHERE occurred_at >= ?1
          AND occurred_at < ?2
        ORDER BY id ASC
        "#,
    )
    .bind(db_occurred_at_lower_bound(min_bucket_start))
    .bind(db_occurred_at_lower_bound(max_bucket_end))
    .fetch_all(&mut *tx)
    .await?;

    let mut live_targets = HashSet::new();
    for row in rows {
        let key = (invocation_bucket_start_epoch(&row.occurred_at)?, row.source);
        if bucket_targets.contains(&key) {
            live_targets.insert(key);
        }
    }
    Ok(live_targets)
}

pub(crate) async fn mark_forward_proxy_hourly_rollup_buckets_materialized_tx(
    tx: &mut SqliteConnection,
    rows: &[ForwardProxyAttemptHourlySourceRecord],
) -> Result<()> {
    let mut buckets = HashSet::new();
    for row in rows {
        buckets.insert(forward_proxy_attempt_bucket_start_epoch(&row.occurred_at)?);
    }
    for bucket_start_epoch in buckets {
        mark_hourly_rollup_bucket_materialized_tx(
            tx,
            HOURLY_ROLLUP_TARGET_FORWARD_PROXY_ATTEMPTS,
            bucket_start_epoch,
            HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn upsert_invocation_rollups(
    tx: &mut sqlx::SqliteConnection,
    candidates: &[InvocationArchiveCandidate],
) -> Result<()> {
    let mut rollups: BTreeMap<(String, String), InvocationRollupDelta> = BTreeMap::new();
    for candidate in candidates {
        let stats_date = shanghai_day_key_from_local_naive(&candidate.occurred_at)?;
        let key = (stats_date, candidate.source.clone());
        let entry = rollups.entry(key).or_default();
        entry.total_count += 1;
        if matches!(candidate.status.as_deref(), Some("success")) {
            entry.success_count += 1;
        } else if candidate
            .status
            .as_deref()
            .is_some_and(|status| status != "success")
        {
            entry.failure_count += 1;
        }
        entry.total_tokens += candidate.total_tokens.unwrap_or_default();
        entry.total_cost += candidate.cost.unwrap_or_default();
    }

    for ((stats_date, source), delta) in rollups {
        sqlx::query(
            r#"
            INSERT INTO invocation_rollup_daily (
                stats_date,
                source,
                total_count,
                success_count,
                failure_count,
                total_tokens,
                total_cost,
                created_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))
            ON CONFLICT(stats_date, source) DO UPDATE SET
                total_count = invocation_rollup_daily.total_count + excluded.total_count,
                success_count = invocation_rollup_daily.success_count + excluded.success_count,
                failure_count = invocation_rollup_daily.failure_count + excluded.failure_count,
                total_tokens = invocation_rollup_daily.total_tokens + excluded.total_tokens,
                total_cost = invocation_rollup_daily.total_cost + excluded.total_cost
            "#,
        )
        .bind(&stats_date)
        .bind(&source)
        .bind(delta.total_count)
        .bind(delta.success_count)
        .bind(delta.failure_count)
        .bind(delta.total_tokens)
        .bind(delta.total_cost)
        .execute(&mut *tx)
        .await?;
    }

    Ok(())
}

pub(crate) async fn upsert_invocation_hourly_rollups_tx(
    tx: &mut SqliteConnection,
    rows: &[InvocationHourlySourceRecord],
    targets: &[&str],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let upsert_overall = targets.contains(&HOURLY_ROLLUP_TARGET_INVOCATIONS);
    let upsert_failures = targets.contains(&HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES);
    let upsert_perf = targets.contains(&HOURLY_ROLLUP_TARGET_PROXY_PERF);
    let upsert_prompt_cache = targets.contains(&HOURLY_ROLLUP_TARGET_PROMPT_CACHE);
    let upsert_prompt_cache_upstream_accounts =
        targets.contains(&HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS);
    let upsert_sticky_keys = targets.contains(&HOURLY_ROLLUP_TARGET_STICKY_KEYS);

    let mut overall: BTreeMap<(i64, String), InvocationHourlyRollupDelta> = BTreeMap::new();
    let mut failures: BTreeMap<(i64, String, String, i64, String), i64> = BTreeMap::new();
    let mut perf: BTreeMap<(i64, String), ProxyPerfStageHourlyDelta> = BTreeMap::new();
    let mut prompt_cache: BTreeMap<(i64, String, String), KeyedConversationHourlyDelta> =
        BTreeMap::new();
    let mut prompt_cache_upstream_accounts: BTreeMap<
        (i64, String, String, String, Option<i64>, Option<String>),
        KeyedConversationHourlyDelta,
    > = BTreeMap::new();
    let mut sticky_keys: BTreeMap<(i64, i64, String), KeyedConversationHourlyDelta> =
        BTreeMap::new();

    for row in rows {
        let bucket_start_epoch = invocation_bucket_start_epoch(&row.occurred_at)?;
        if upsert_overall {
            accumulate_invocation_hourly_overall_rollups(&mut overall, std::slice::from_ref(row))?;
        }

        if upsert_failures {
            let classification = resolve_failure_classification(
                row.status.as_deref(),
                row.error_message.as_deref(),
                row.failure_kind.as_deref(),
                row.failure_class.as_deref(),
                row.is_actionable,
            );
            if invocation_status_counts_toward_terminal_totals(row.status.as_deref())
                && classification.failure_class != FailureClass::None
            {
                let error_category =
                    categorize_error(row.error_message.as_deref().unwrap_or_default());
                *failures
                    .entry((
                        bucket_start_epoch,
                        row.source.clone(),
                        classification.failure_class.as_str().to_string(),
                        classification.is_actionable as i64,
                        error_category,
                    ))
                    .or_default() += 1;
            }
        }

        if upsert_perf && row.source == SOURCE_PROXY {
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_TOTAL,
                row.t_total_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_REQUEST_READ,
                row.t_req_read_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_REQUEST_PARSE,
                row.t_req_parse_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_UPSTREAM_CONNECT,
                row.t_upstream_connect_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_UPSTREAM_FIRST_BYTE,
                row.t_upstream_ttfb_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_UPSTREAM_STREAM,
                row.t_upstream_stream_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_RESPONSE_PARSE,
                row.t_resp_parse_ms,
            );
            record_proxy_perf_stage_sample(
                &mut perf,
                bucket_start_epoch,
                PERF_STAGE_PERSISTENCE,
                row.t_persist_ms,
            );
        }

        if (upsert_prompt_cache || upsert_prompt_cache_upstream_accounts)
            && let Some(prompt_cache_key) = prompt_cache_key_from_payload(row.payload.as_deref())
        {
            if upsert_prompt_cache {
                let entry = keyed_conversation_delta(
                    &mut prompt_cache,
                    bucket_start_epoch,
                    &row.source,
                    &prompt_cache_key,
                    &row.occurred_at,
                );
                entry.request_count += 1;
                if row.status.as_deref() == Some("success") {
                    entry.success_count += 1;
                } else {
                    entry.failure_count += 1;
                }
                entry.total_tokens += row.total_tokens.unwrap_or_default();
                entry.total_cost += row.cost.unwrap_or_default();
            }

            if upsert_prompt_cache_upstream_accounts {
                let upstream_account_id = upstream_account_id_from_payload(row.payload.as_deref());
                let upstream_account_name =
                    upstream_account_name_from_payload(row.payload.as_deref());
                let rollup_key = prompt_cache_upstream_account_rollup_key(
                    upstream_account_id,
                    upstream_account_name.as_deref(),
                );
                let entry = prompt_cache_upstream_accounts
                    .entry((
                        bucket_start_epoch,
                        row.source.clone(),
                        prompt_cache_key,
                        rollup_key,
                        upstream_account_id,
                        upstream_account_name.clone(),
                    ))
                    .or_insert_with(|| KeyedConversationHourlyDelta {
                        first_seen_at: row.occurred_at.clone(),
                        last_seen_at: row.occurred_at.clone(),
                        ..KeyedConversationHourlyDelta::default()
                    });
                if row.occurred_at < entry.first_seen_at {
                    entry.first_seen_at = row.occurred_at.clone();
                }
                if row.occurred_at > entry.last_seen_at {
                    entry.last_seen_at = row.occurred_at.clone();
                }
                entry.request_count += 1;
                if row.status.as_deref() == Some("success") {
                    entry.success_count += 1;
                } else {
                    entry.failure_count += 1;
                }
                entry.total_tokens += row.total_tokens.unwrap_or_default();
                entry.total_cost += row.cost.unwrap_or_default();
            }
        }

        if upsert_sticky_keys
            && let (Some(upstream_account_id), Some(sticky_key)) = (
                upstream_account_id_from_payload(row.payload.as_deref()),
                sticky_key_from_payload(row.payload.as_deref()),
            )
        {
            let entry = sticky_keys
                .entry((bucket_start_epoch, upstream_account_id, sticky_key))
                .or_insert_with(|| KeyedConversationHourlyDelta {
                    first_seen_at: row.occurred_at.clone(),
                    last_seen_at: row.occurred_at.clone(),
                    ..KeyedConversationHourlyDelta::default()
                });
            if row.occurred_at < entry.first_seen_at {
                entry.first_seen_at = row.occurred_at.clone();
            }
            if row.occurred_at > entry.last_seen_at {
                entry.last_seen_at = row.occurred_at.clone();
            }
            entry.request_count += 1;
            if row.status.as_deref() == Some("success") {
                entry.success_count += 1;
            } else {
                entry.failure_count += 1;
            }
            entry.total_tokens += row.total_tokens.unwrap_or_default();
            entry.total_cost += row.cost.unwrap_or_default();
        }
    }

    if upsert_overall {
        #[derive(sqlx::FromRow)]
        struct InvocationRollupHistogramRow {
            first_byte_histogram: String,
            first_response_byte_total_histogram: String,
        }

        for ((bucket_start_epoch, source), delta) in overall {
            let current_histograms = sqlx::query_as::<_, InvocationRollupHistogramRow>(
                r#"
                SELECT
                    first_byte_histogram,
                    first_response_byte_total_histogram
                FROM invocation_rollup_hourly
                WHERE bucket_start_epoch = ?1 AND source = ?2
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .fetch_optional(&mut *tx)
            .await?;
            let mut merged_first_byte_histogram = current_histograms
                .as_ref()
                .map(|row| decode_approx_histogram(&row.first_byte_histogram))
                .unwrap_or_else(empty_approx_histogram);
            merge_approx_histogram_into(
                &mut merged_first_byte_histogram,
                &delta.first_byte_histogram,
            )?;
            let mut merged_first_response_byte_total_histogram = current_histograms
                .as_ref()
                .map(|row| decode_approx_histogram(&row.first_response_byte_total_histogram))
                .unwrap_or_else(empty_approx_histogram);
            merge_approx_histogram_into(
                &mut merged_first_response_byte_total_histogram,
                &delta.first_response_byte_total_histogram,
            )?;
            sqlx::query(
                r#"
                INSERT INTO invocation_rollup_hourly (
                    bucket_start_epoch,
                    source,
                    total_count,
                    success_count,
                    failure_count,
                    total_tokens,
                    total_cost,
                    first_byte_sample_count,
                    first_byte_sum_ms,
                    first_byte_max_ms,
                    first_byte_histogram,
                    first_response_byte_total_sample_count,
                    first_response_byte_total_sum_ms,
                    first_response_byte_total_max_ms,
                    first_response_byte_total_histogram,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source) DO UPDATE SET
                    total_count = invocation_rollup_hourly.total_count + excluded.total_count,
                    success_count = invocation_rollup_hourly.success_count + excluded.success_count,
                    failure_count = invocation_rollup_hourly.failure_count + excluded.failure_count,
                    total_tokens = invocation_rollup_hourly.total_tokens + excluded.total_tokens,
                    total_cost = invocation_rollup_hourly.total_cost + excluded.total_cost,
                    first_byte_sample_count = invocation_rollup_hourly.first_byte_sample_count + excluded.first_byte_sample_count,
                    first_byte_sum_ms = invocation_rollup_hourly.first_byte_sum_ms + excluded.first_byte_sum_ms,
                    first_byte_max_ms = MAX(invocation_rollup_hourly.first_byte_max_ms, excluded.first_byte_max_ms),
                    first_byte_histogram = excluded.first_byte_histogram,
                    first_response_byte_total_sample_count = invocation_rollup_hourly.first_response_byte_total_sample_count + excluded.first_response_byte_total_sample_count,
                    first_response_byte_total_sum_ms = invocation_rollup_hourly.first_response_byte_total_sum_ms + excluded.first_response_byte_total_sum_ms,
                    first_response_byte_total_max_ms = MAX(invocation_rollup_hourly.first_response_byte_total_max_ms, excluded.first_response_byte_total_max_ms),
                    first_response_byte_total_histogram = excluded.first_response_byte_total_histogram,
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(delta.total_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.total_tokens)
            .bind(delta.total_cost)
            .bind(delta.first_byte_sample_count)
            .bind(delta.first_byte_sum_ms)
            .bind(delta.first_byte_max_ms)
            .bind(encode_approx_histogram(&merged_first_byte_histogram)?)
            .bind(delta.first_response_byte_total_sample_count)
            .bind(delta.first_response_byte_total_sum_ms)
            .bind(delta.first_response_byte_total_max_ms)
            .bind(encode_approx_histogram(
                &merged_first_response_byte_total_histogram,
            )?)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_failures {
        for (
            (bucket_start_epoch, source, failure_class, is_actionable, error_category),
            failure_count,
        ) in failures
        {
            sqlx::query(
                r#"
                INSERT INTO invocation_failure_rollup_hourly (
                    bucket_start_epoch,
                    source,
                    failure_class,
                    is_actionable,
                    error_category,
                    failure_count,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source, failure_class, is_actionable, error_category) DO UPDATE SET
                    failure_count = invocation_failure_rollup_hourly.failure_count + excluded.failure_count,
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(&failure_class)
            .bind(is_actionable)
            .bind(&error_category)
            .bind(failure_count)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_perf {
        for ((bucket_start_epoch, stage), delta) in perf {
            let current_histogram = sqlx::query_scalar::<_, String>(
                r#"
                SELECT histogram
                FROM proxy_perf_stage_hourly
                WHERE bucket_start_epoch = ?1 AND stage = ?2
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&stage)
            .fetch_optional(&mut *tx)
            .await?;
            let mut merged_histogram = current_histogram
                .as_deref()
                .map(decode_approx_histogram)
                .unwrap_or_else(empty_approx_histogram);
            merge_approx_histogram_into(&mut merged_histogram, &delta.histogram)?;
            sqlx::query(
                r#"
                INSERT INTO proxy_perf_stage_hourly (
                    bucket_start_epoch,
                    stage,
                    sample_count,
                    sum_ms,
                    max_ms,
                    histogram,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
                ON CONFLICT(bucket_start_epoch, stage) DO UPDATE SET
                    sample_count = proxy_perf_stage_hourly.sample_count + excluded.sample_count,
                    sum_ms = proxy_perf_stage_hourly.sum_ms + excluded.sum_ms,
                    max_ms = MAX(proxy_perf_stage_hourly.max_ms, excluded.max_ms),
                    histogram = excluded.histogram,
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&stage)
            .bind(delta.sample_count)
            .bind(delta.sum_ms)
            .bind(delta.max_ms)
            .bind(encode_approx_histogram(&merged_histogram)?)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_prompt_cache {
        for ((bucket_start_epoch, source, prompt_cache_key), delta) in prompt_cache {
            sqlx::query(
                r#"
                INSERT INTO prompt_cache_rollup_hourly (
                    bucket_start_epoch,
                    source,
                    prompt_cache_key,
                    request_count,
                    success_count,
                    failure_count,
                    total_tokens,
                    total_cost,
                    first_seen_at,
                    last_seen_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source, prompt_cache_key) DO UPDATE SET
                    request_count = prompt_cache_rollup_hourly.request_count + excluded.request_count,
                    success_count = prompt_cache_rollup_hourly.success_count + excluded.success_count,
                    failure_count = prompt_cache_rollup_hourly.failure_count + excluded.failure_count,
                    total_tokens = prompt_cache_rollup_hourly.total_tokens + excluded.total_tokens,
                    total_cost = prompt_cache_rollup_hourly.total_cost + excluded.total_cost,
                    first_seen_at = MIN(prompt_cache_rollup_hourly.first_seen_at, excluded.first_seen_at),
                    last_seen_at = MAX(prompt_cache_rollup_hourly.last_seen_at, excluded.last_seen_at),
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(&prompt_cache_key)
            .bind(delta.request_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.total_tokens)
            .bind(delta.total_cost)
            .bind(&delta.first_seen_at)
            .bind(&delta.last_seen_at)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_prompt_cache_upstream_accounts {
        for (
            (
                bucket_start_epoch,
                source,
                prompt_cache_key,
                upstream_account_key,
                upstream_account_id,
                upstream_account_name,
            ),
            delta,
        ) in prompt_cache_upstream_accounts
        {
            sqlx::query(
                r#"
                INSERT INTO prompt_cache_upstream_account_hourly (
                    bucket_start_epoch,
                    source,
                    prompt_cache_key,
                    upstream_account_key,
                    upstream_account_id,
                    upstream_account_name,
                    request_count,
                    success_count,
                    failure_count,
                    total_tokens,
                    total_cost,
                    first_seen_at,
                    last_seen_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source, prompt_cache_key, upstream_account_key) DO UPDATE SET
                    request_count = prompt_cache_upstream_account_hourly.request_count + excluded.request_count,
                    success_count = prompt_cache_upstream_account_hourly.success_count + excluded.success_count,
                    failure_count = prompt_cache_upstream_account_hourly.failure_count + excluded.failure_count,
                    total_tokens = prompt_cache_upstream_account_hourly.total_tokens + excluded.total_tokens,
                    total_cost = prompt_cache_upstream_account_hourly.total_cost + excluded.total_cost,
                    first_seen_at = MIN(prompt_cache_upstream_account_hourly.first_seen_at, excluded.first_seen_at),
                    last_seen_at = MAX(prompt_cache_upstream_account_hourly.last_seen_at, excluded.last_seen_at),
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(&prompt_cache_key)
            .bind(&upstream_account_key)
            .bind(upstream_account_id)
            .bind(upstream_account_name.as_deref())
            .bind(delta.request_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.total_tokens)
            .bind(delta.total_cost)
            .bind(&delta.first_seen_at)
            .bind(&delta.last_seen_at)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_sticky_keys {
        for ((bucket_start_epoch, upstream_account_id, sticky_key), delta) in sticky_keys {
            sqlx::query(
                r#"
                INSERT INTO upstream_sticky_key_hourly (
                    bucket_start_epoch,
                    upstream_account_id,
                    sticky_key,
                    request_count,
                    success_count,
                    failure_count,
                    total_tokens,
                    total_cost,
                    first_seen_at,
                    last_seen_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))
                ON CONFLICT(bucket_start_epoch, upstream_account_id, sticky_key) DO UPDATE SET
                    request_count = upstream_sticky_key_hourly.request_count + excluded.request_count,
                    success_count = upstream_sticky_key_hourly.success_count + excluded.success_count,
                    failure_count = upstream_sticky_key_hourly.failure_count + excluded.failure_count,
                    total_tokens = upstream_sticky_key_hourly.total_tokens + excluded.total_tokens,
                    total_cost = upstream_sticky_key_hourly.total_cost + excluded.total_cost,
                    first_seen_at = MIN(upstream_sticky_key_hourly.first_seen_at, excluded.first_seen_at),
                    last_seen_at = MAX(upstream_sticky_key_hourly.last_seen_at, excluded.last_seen_at),
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(upstream_account_id)
            .bind(&sticky_key)
            .bind(delta.request_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.total_tokens)
            .bind(delta.total_cost)
            .bind(&delta.first_seen_at)
            .bind(&delta.last_seen_at)
            .execute(&mut *tx)
            .await?;
        }
    }

    Ok(())
}

pub(crate) fn invocation_archive_target_needs_full_payload(target: &str) -> bool {
    matches!(
        target,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE
            | HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS
            | HOURLY_ROLLUP_TARGET_STICKY_KEYS
    )
}

pub(crate) async fn upsert_forward_proxy_attempt_hourly_rollups_tx(
    tx: &mut SqliteConnection,
    rows: &[ForwardProxyAttemptHourlySourceRecord],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }

    let mut deltas: BTreeMap<(String, i64), ForwardProxyAttemptHourlyDelta> = BTreeMap::new();
    for row in rows {
        let bucket_start_epoch = forward_proxy_attempt_bucket_start_epoch(&row.occurred_at)?;
        let entry = deltas
            .entry((row.proxy_key.clone(), bucket_start_epoch))
            .or_default();
        entry.attempts += 1;
        if row.is_success != 0 {
            entry.success_count += 1;
        } else {
            entry.failure_count += 1;
        }
        if let Some(latency_ms) = row.latency_ms
            && latency_ms.is_finite()
            && latency_ms >= 0.0
        {
            entry.latency_sample_count += 1;
            entry.latency_sum_ms += latency_ms;
            entry.latency_max_ms = entry.latency_max_ms.max(latency_ms);
        }
    }

    for ((proxy_key, bucket_start_epoch), delta) in deltas {
        sqlx::query(
            r#"
            INSERT INTO forward_proxy_attempt_hourly (
                proxy_key,
                bucket_start_epoch,
                attempts,
                success_count,
                failure_count,
                latency_sample_count,
                latency_sum_ms,
                latency_max_ms,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
            ON CONFLICT(proxy_key, bucket_start_epoch) DO UPDATE SET
                attempts = forward_proxy_attempt_hourly.attempts + excluded.attempts,
                success_count = forward_proxy_attempt_hourly.success_count + excluded.success_count,
                failure_count = forward_proxy_attempt_hourly.failure_count + excluded.failure_count,
                latency_sample_count = forward_proxy_attempt_hourly.latency_sample_count + excluded.latency_sample_count,
                latency_sum_ms = forward_proxy_attempt_hourly.latency_sum_ms + excluded.latency_sum_ms,
                latency_max_ms = MAX(forward_proxy_attempt_hourly.latency_max_ms, excluded.latency_max_ms),
                updated_at = datetime('now')
            "#,
        )
        .bind(&proxy_key)
        .bind(bucket_start_epoch)
        .bind(delta.attempts)
        .bind(delta.success_count)
        .bind(delta.failure_count)
        .bind(delta.latency_sample_count)
        .bind(delta.latency_sum_ms)
        .bind(delta.latency_max_ms)
        .execute(&mut *tx)
        .await?;
    }

    Ok(())
}

pub(crate) async fn delete_hourly_rollup_rows_for_bucket_epochs_tx(
    tx: &mut SqliteConnection,
    table: &str,
    bucket_epochs: &[i64],
) -> Result<()> {
    if bucket_epochs.is_empty() {
        return Ok(());
    }
    let mut query =
        QueryBuilder::<Sqlite>::new(format!("DELETE FROM {table} WHERE bucket_start_epoch IN ("));
    {
        let mut separated = query.separated(", ");
        for bucket_epoch in bucket_epochs {
            separated.push_bind(bucket_epoch);
        }
    }
    query.push(")");
    query.build().execute(&mut *tx).await?;
    Ok(())
}

pub(crate) async fn load_live_invocation_hourly_rows_for_bucket_epochs_tx(
    tx: &mut SqliteConnection,
    bucket_epochs: &[i64],
) -> Result<Vec<InvocationHourlySourceRecord>> {
    if bucket_epochs.is_empty() {
        return Ok(Vec::new());
    }

    let min_bucket_epoch = *bucket_epochs
        .iter()
        .min()
        .ok_or_else(|| anyhow!("missing minimum invocation bucket epoch"))?;
    let max_bucket_epoch = *bucket_epochs
        .iter()
        .max()
        .ok_or_else(|| anyhow!("missing maximum invocation bucket epoch"))?;
    let min_bucket_start = Utc
        .timestamp_opt(min_bucket_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid minimum invocation bucket epoch"))?;
    let max_bucket_end = Utc
        .timestamp_opt(max_bucket_epoch + 3_600, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid maximum invocation bucket epoch"))?;
    let bucket_epoch_set = bucket_epochs.iter().copied().collect::<HashSet<_>>();

    let rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(
        "SELECT \
            id,
            occurred_at,
            source,
            status,
            detail_level,
            total_tokens,
            cost,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
         FROM codex_invocations
         WHERE occurred_at >= ?1
           AND occurred_at < ?2
         ORDER BY id ASC",
    )
    .bind(db_occurred_at_lower_bound(min_bucket_start))
    .bind(db_occurred_at_lower_bound(max_bucket_end))
    .fetch_all(&mut *tx)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|row| {
            invocation_bucket_start_epoch(&row.occurred_at)
                .map(|bucket_epoch| bucket_epoch_set.contains(&bucket_epoch))
                .unwrap_or(false)
        })
        .collect())
}

pub(crate) async fn recompute_invocation_hourly_rollups_for_ids_tx(
    tx: &mut SqliteConnection,
    ids: &[i64],
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT DISTINCT occurred_at FROM codex_invocations WHERE id IN (",
    );
    {
        let mut separated = query.separated(", ");
        for id in ids {
            separated.push_bind(id);
        }
    }
    query.push(")");
    let occurred_rows = query
        .build_query_scalar::<String>()
        .fetch_all(&mut *tx)
        .await?;
    if occurred_rows.is_empty() {
        return Ok(());
    }

    let mut bucket_epochs = occurred_rows
        .iter()
        .map(|occurred_at| invocation_bucket_start_epoch(occurred_at))
        .collect::<Result<Vec<_>>>()?;
    bucket_epochs.sort_unstable();
    bucket_epochs.dedup();
    if bucket_epochs.is_empty() {
        return Ok(());
    }

    for table in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES,
        HOURLY_ROLLUP_TARGET_PROXY_PERF,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE,
        HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS,
        HOURLY_ROLLUP_TARGET_STICKY_KEYS,
    ] {
        delete_hourly_rollup_rows_for_bucket_epochs_tx(tx, table, &bucket_epochs).await?;
    }

    let rows = load_live_invocation_hourly_rows_for_bucket_epochs_tx(tx, &bucket_epochs).await?;
    upsert_invocation_hourly_rollups_tx(tx, &rows, &INVOCATION_HOURLY_ROLLUP_TARGETS).await?;
    Ok(())
}

pub(crate) async fn replay_live_invocation_hourly_rollups(pool: &Pool<Sqlite>) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress(pool, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(
        r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
            total_tokens,
            cost,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    let mut tx = pool.begin().await?;
    upsert_invocation_hourly_rollups_tx(tx.as_mut(), &rows, &INVOCATION_HOURLY_ROLLUP_TARGETS)
        .await?;
    save_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS, last_id)
        .await?;
    tx.commit().await?;
    Ok(rows.len() as u64)
}

pub(crate) async fn replay_live_invocation_hourly_rollups_tx(tx: &mut SqliteConnection) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(
        r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
            total_tokens,
            cost,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(&mut *tx)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    upsert_invocation_hourly_rollups_tx(tx, &rows, &INVOCATION_HOURLY_ROLLUP_TARGETS).await?;
    save_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS, last_id).await?;
    Ok(rows.len() as u64)
}

pub(crate) async fn replay_live_forward_proxy_attempt_hourly_rollups(pool: &Pool<Sqlite>) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress(pool, HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
            .await?;
    let rows = sqlx::query_as::<_, ForwardProxyAttemptHourlySourceRecord>(
        r#"
        SELECT
            id,
            proxy_key,
            occurred_at,
            is_success,
            latency_ms
        FROM forward_proxy_attempts
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    let mut tx = pool.begin().await?;
    upsert_forward_proxy_attempt_hourly_rollups_tx(tx.as_mut(), &rows).await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
        last_id,
    )
    .await?;
    tx.commit().await?;
    Ok(rows.len() as u64)
}

pub(crate) async fn replay_live_forward_proxy_attempt_hourly_rollups_tx(
    tx: &mut SqliteConnection,
) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
            .await?;
    let rows = sqlx::query_as::<_, ForwardProxyAttemptHourlySourceRecord>(
        r#"
        SELECT
            id,
            proxy_key,
            occurred_at,
            is_success,
            latency_ms
        FROM forward_proxy_attempts
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(&mut *tx)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    upsert_forward_proxy_attempt_hourly_rollups_tx(tx, &rows).await?;
    save_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS, last_id)
        .await?;
    Ok(rows.len() as u64)
}

pub(crate) async fn backfill_invocation_rollup_hourly_from_sources(pool: &Pool<Sqlite>) -> Result<usize> {
    let archive_files = sqlx::query_as::<_, ArchiveBatchFileRow>(
        r#"
        SELECT id, file_path, coverage_start_at, coverage_end_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
        ORDER BY month_key ASC, created_at ASC, id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .fetch_all(pool)
    .await?;
    let mut overall: BTreeMap<(i64, String), InvocationHourlyRollupDelta> = BTreeMap::new();
    let mut seen_ids = HashSet::new();

    for archive_file in archive_files {
        let archive_path = PathBuf::from(&archive_file.file_path);
        if !archive_path.exists() {
            warn!(
                dataset = HOURLY_ROLLUP_DATASET_INVOCATIONS,
                file_path = archive_file.file_path,
                "skipping missing archive batch during invocation hourly rollup backfill"
            );
            continue;
        }

        let temp_path = PathBuf::from(format!(
            "{}.{}.sqlite",
            archive_path.display(),
            retention_temp_suffix()
        ));
        if temp_path.exists() {
            let _ = fs::remove_file(&temp_path);
        }
        let temp_cleanup = TempSqliteCleanup(temp_path.clone());
        inflate_gzip_sqlite_file(&archive_path, &temp_path)?;
        let archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&sqlite_url_for_path(&temp_path))
            .await
            .with_context(|| format!("failed to open archive batch {}", archive_path.display()))?;
        let mut archive_cursor_id = 0_i64;
        loop {
            let mut rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(
                r#"
                SELECT
                    id,
                    occurred_at,
                    source,
                    status,
                    detail_level,
                    total_tokens,
                    cost,
                    error_message,
                    failure_kind,
                    failure_class,
                    is_actionable,
                    payload,
                    t_total_ms,
                    t_req_read_ms,
                    t_req_parse_ms,
                    t_upstream_connect_ms,
                    t_upstream_ttfb_ms,
                    t_upstream_stream_ms,
                    t_resp_parse_ms,
                    t_persist_ms
                FROM codex_invocations
                WHERE id > ?1
                ORDER BY id ASC
                LIMIT ?2
                "#,
            )
            .bind(archive_cursor_id)
            .bind(BACKFILL_BATCH_SIZE)
            .fetch_all(&archive_pool)
            .await?;
            if rows.is_empty() {
                break;
            }
            archive_cursor_id = rows.last().map(|row| row.id).unwrap_or(archive_cursor_id);
            rows.retain(|row| seen_ids.insert(row.id));
            if rows.is_empty() {
                continue;
            }
            accumulate_invocation_hourly_overall_rollups(&mut overall, &rows)?;
        }
        archive_pool.close().await;
        drop(temp_cleanup);
    }

    let mut cursor_id = 0_i64;
    loop {
        let mut rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(
            r#"
            SELECT
                id,
                occurred_at,
                source,
                status,
                detail_level,
                total_tokens,
                cost,
                error_message,
                failure_kind,
                failure_class,
                is_actionable,
                payload,
                t_total_ms,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                t_upstream_stream_ms,
                t_resp_parse_ms,
                t_persist_ms
            FROM codex_invocations
            WHERE id > ?1
            ORDER BY id ASC
            LIMIT ?2
            "#,
        )
        .bind(cursor_id)
        .bind(BACKFILL_BATCH_SIZE)
        .fetch_all(pool)
        .await?;
        if rows.is_empty() {
            break;
        }
        cursor_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
        rows.retain(|row| seen_ids.insert(row.id));
        if rows.is_empty() {
            continue;
        }
        accumulate_invocation_hourly_overall_rollups(&mut overall, &rows)?;
    }

    if overall.is_empty() {
        return Ok(0);
    }

    let mut tx = pool.begin().await?;
    for ((bucket_start_epoch, source), delta) in &overall {
        sqlx::query(
            r#"
            INSERT INTO invocation_rollup_hourly (
                bucket_start_epoch,
                source,
                total_count,
                success_count,
                failure_count,
                total_tokens,
                total_cost,
                first_byte_sample_count,
                first_byte_sum_ms,
                first_byte_max_ms,
                first_byte_histogram,
                first_response_byte_total_sample_count,
                first_response_byte_total_sum_ms,
                first_response_byte_total_max_ms,
                first_response_byte_total_histogram,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, datetime('now'))
            ON CONFLICT(bucket_start_epoch, source) DO UPDATE SET
                total_count = excluded.total_count,
                success_count = excluded.success_count,
                failure_count = excluded.failure_count,
                total_tokens = excluded.total_tokens,
                total_cost = excluded.total_cost,
                first_byte_sample_count = excluded.first_byte_sample_count,
                first_byte_sum_ms = excluded.first_byte_sum_ms,
                first_byte_max_ms = excluded.first_byte_max_ms,
                first_byte_histogram = excluded.first_byte_histogram,
                first_response_byte_total_sample_count = excluded.first_response_byte_total_sample_count,
                first_response_byte_total_sum_ms = excluded.first_response_byte_total_sum_ms,
                first_response_byte_total_max_ms = excluded.first_response_byte_total_max_ms,
                first_response_byte_total_histogram = excluded.first_response_byte_total_histogram,
                updated_at = datetime('now')
            "#,
        )
        .bind(*bucket_start_epoch)
        .bind(source)
        .bind(delta.total_count)
        .bind(delta.success_count)
        .bind(delta.failure_count)
        .bind(delta.total_tokens)
        .bind(delta.total_cost)
        .bind(delta.first_byte_sample_count)
        .bind(delta.first_byte_sum_ms)
        .bind(delta.first_byte_max_ms)
        .bind(encode_approx_histogram(&delta.first_byte_histogram)?)
        .bind(delta.first_response_byte_total_sample_count)
        .bind(delta.first_response_byte_total_sum_ms)
        .bind(delta.first_response_byte_total_max_ms)
        .bind(encode_approx_histogram(
            &delta.first_response_byte_total_histogram,
        )?)
        .execute(tx.as_mut())
        .await?;
    }
    tx.commit().await?;

    Ok(overall.len())
}
