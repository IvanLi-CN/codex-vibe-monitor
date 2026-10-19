use super::*;

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
    upsert_parallel_work_minute_key_rollups_tx(tx, rows).await?;
    let upsert_overall = targets.contains(&HOURLY_ROLLUP_TARGET_INVOCATIONS);
    let upsert_failures = targets.contains(&HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES);
    let upsert_perf = targets.contains(&HOURLY_ROLLUP_TARGET_PROXY_PERF);
    let upsert_prompt_cache = targets.contains(&HOURLY_ROLLUP_TARGET_PROMPT_CACHE);
    let upsert_prompt_cache_upstream_accounts =
        targets.contains(&HOURLY_ROLLUP_TARGET_PROMPT_CACHE_UPSTREAM_ACCOUNTS);
    let upsert_upstream_account_usage =
        targets.contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE);
    let upsert_upstream_account_usage_breakdown =
        targets.contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN);
    let upsert_upstream_account_stats_hourly =
        targets.contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY);
    let upsert_upstream_account_activity_v2 =
        targets.contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2);
    let upsert_upstream_account_stats_minute =
        targets.contains(&HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE);
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
    let mut upstream_account_usage: BTreeMap<(i64, i64), UpstreamAccountUsageHourlyDelta> =
        BTreeMap::new();
    let mut upstream_account_usage_breakdown: BTreeMap<
        (i64, String, String, Option<i64>, String, String),
        UpstreamAccountUsageBreakdownHourlyDelta,
    > = BTreeMap::new();
    let mut upstream_account_stats_hourly: BTreeMap<(i64, String, i64), UpstreamAccountStatsDelta> =
        BTreeMap::new();
    let mut upstream_account_activity_v2: BTreeMap<(i64, String, i64), UpstreamAccountStatsDelta> =
        BTreeMap::new();
    let mut upstream_account_stats_minute: BTreeMap<(i64, String, i64), UpstreamAccountStatsDelta> =
        BTreeMap::new();
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
                let error_category = categorize_error_with_failure_kind(
                    row.error_message.as_deref().unwrap_or_default(),
                    row.failure_kind.as_deref(),
                );
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
            let classification = resolve_failure_classification(
                row.status.as_deref(),
                row.error_message.as_deref(),
                row.failure_kind.as_deref(),
                row.failure_class.as_deref(),
                row.is_actionable,
            );
            let is_success_like = invocation_status_is_success_like(
                row.status.as_deref(),
                row.error_message.as_deref(),
            ) && classification.failure_class == FailureClass::None;
            if upsert_prompt_cache {
                let entry = keyed_conversation_delta(
                    &mut prompt_cache,
                    bucket_start_epoch,
                    &row.source,
                    &prompt_cache_key,
                    &row.occurred_at,
                );
                entry.request_count += 1;
                if is_success_like {
                    entry.success_count += 1;
                } else {
                    entry.failure_count += 1;
                }
                entry.total_tokens += row.total_tokens.unwrap_or_default();
                entry.total_cost += row.cost.unwrap_or_default();
            }

            if upsert_prompt_cache_upstream_accounts {
                let upstream_account_id = row.resolved_upstream_account_id();
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
                if is_success_like {
                    entry.success_count += 1;
                } else {
                    entry.failure_count += 1;
                }
                entry.total_tokens += row.total_tokens.unwrap_or_default();
                entry.total_cost += row.cost.unwrap_or_default();
            }
        }

        if upsert_upstream_account_usage
            && let Some(upstream_account_id) = row.resolved_upstream_account_id()
        {
            let entry = upstream_account_usage
                .entry((bucket_start_epoch, upstream_account_id))
                .or_insert_with(|| UpstreamAccountUsageHourlyDelta {
                    first_seen_at: row.occurred_at.clone(),
                    last_seen_at: row.occurred_at.clone(),
                    ..UpstreamAccountUsageHourlyDelta::default()
                });
            if row.occurred_at < entry.first_seen_at {
                entry.first_seen_at = row.occurred_at.clone();
            }
            if row.occurred_at > entry.last_seen_at {
                entry.last_seen_at = row.occurred_at.clone();
            }
            entry.request_count += 1;
            let classification = resolve_failure_classification(
                row.status.as_deref(),
                row.error_message.as_deref(),
                row.failure_kind.as_deref(),
                row.failure_class.as_deref(),
                row.is_actionable,
            );
            if invocation_status_is_success_like(
                row.status.as_deref(),
                row.error_message.as_deref(),
            ) && classification.failure_class == FailureClass::None
            {
                entry.success_count += 1;
            } else if invocation_status_counts_toward_terminal_totals(row.status.as_deref())
                && classification.failure_class != FailureClass::None
            {
                entry.failure_count += 1;
            }
            entry.total_tokens += row.total_tokens.unwrap_or_default();
            let cost = row.cost.unwrap_or_default();
            entry.total_cost += cost;
            if invocation_counts_toward_non_success_usage(
                row.status.as_deref(),
                row.error_message.as_deref(),
                row.failure_kind.as_deref(),
                row.failure_class.as_deref(),
                row.is_actionable,
            ) {
                entry.non_success_cost += cost;
            }
            entry.input_tokens += row.input_tokens.unwrap_or_default();
            entry.output_tokens += row.output_tokens.unwrap_or_default();
            entry.cache_input_tokens += row.cache_input_tokens.unwrap_or_default();
            entry.reasoning_tokens += row.reasoning_tokens.unwrap_or_default();
        }

        if upsert_upstream_account_usage_breakdown {
            accumulate_upstream_account_usage_breakdown_rollup(
                &mut upstream_account_usage_breakdown,
                row,
            )?;
        }

        if (upsert_upstream_account_stats_hourly || upsert_upstream_account_stats_minute)
            && let Some(upstream_account_id) = row.resolved_upstream_account_id()
        {
            if upsert_upstream_account_stats_hourly {
                let entry = upstream_account_stats_hourly
                    .entry((bucket_start_epoch, row.source.clone(), upstream_account_id))
                    .or_insert_with(|| UpstreamAccountStatsDelta {
                        first_byte_histogram: empty_approx_histogram(),
                        first_response_byte_total_histogram: empty_approx_histogram(),
                        first_token_histogram: empty_approx_histogram(),
                        ..UpstreamAccountStatsDelta::default()
                    });
                accumulate_upstream_account_stats_delta(entry, row);
            }

            if upsert_upstream_account_stats_minute {
                let minute_bucket_start_epoch =
                    invocation_bucket_start_epoch_for_seconds(&row.occurred_at, 60)?;
                let entry = upstream_account_stats_minute
                    .entry((
                        minute_bucket_start_epoch,
                        row.source.clone(),
                        upstream_account_id,
                    ))
                    .or_insert_with(|| UpstreamAccountStatsDelta {
                        first_byte_histogram: empty_approx_histogram(),
                        first_response_byte_total_histogram: empty_approx_histogram(),
                        first_token_histogram: empty_approx_histogram(),
                        ..UpstreamAccountStatsDelta::default()
                    });
                accumulate_upstream_account_stats_delta(entry, row);
            }
        }

        if upsert_upstream_account_activity_v2
            && invocation_row_counts_toward_account_activity_v2(row)
        {
            let upstream_account_id = row
                .resolved_upstream_account_id()
                .unwrap_or(UPSTREAM_ACCOUNT_ACTIVITY_UNASSIGNED_ID);
            let entry = upstream_account_activity_v2
                .entry((bucket_start_epoch, row.source.clone(), upstream_account_id))
                .or_insert_with(|| UpstreamAccountStatsDelta {
                    first_byte_histogram: empty_approx_histogram(),
                    first_response_byte_total_histogram: empty_approx_histogram(),
                    first_token_histogram: empty_approx_histogram(),
                    ..UpstreamAccountStatsDelta::default()
                });
            accumulate_upstream_account_activity_v2_delta(entry, row);
            if prompt_cache_key_from_payload(row.payload.as_deref()).is_none()
                && entry
                    .latest_unkeyed_conversation_at
                    .as_deref()
                    .is_none_or(|latest| row.occurred_at.as_str() > latest)
            {
                entry.latest_unkeyed_conversation_at = Some(row.occurred_at.clone());
            }
        }

        if upsert_sticky_keys
            && let (Some(upstream_account_id), Some(sticky_key)) = (
                row.resolved_upstream_account_id(),
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
            let classification = resolve_failure_classification(
                row.status.as_deref(),
                row.error_message.as_deref(),
                row.failure_kind.as_deref(),
                row.failure_class.as_deref(),
                row.is_actionable,
            );
            if invocation_status_is_success_like(
                row.status.as_deref(),
                row.error_message.as_deref(),
            ) && classification.failure_class == FailureClass::None
            {
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
            first_token_histogram: String,
        }

        for ((bucket_start_epoch, source), delta) in overall {
            let current_histograms = sqlx::query_as::<_, InvocationRollupHistogramRow>(
                r#"
                SELECT
                    first_byte_histogram,
                    first_response_byte_total_histogram,
                    first_token_histogram
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
            let mut merged_first_token_histogram = current_histograms
                .as_ref()
                .map(|row| decode_approx_histogram(&row.first_token_histogram))
                .unwrap_or_else(empty_approx_histogram);
            merge_approx_histogram_into(
                &mut merged_first_token_histogram,
                &delta.first_token_histogram,
            )?;
            sqlx::query(
                r#"
                INSERT INTO invocation_rollup_hourly (
                    bucket_start_epoch,
                    source,
                    total_count,
                    success_count,
                    failure_count,
                    terminal_count,
                    terminal_tokens,
                    terminal_cost,
                    terminal_proof_complete,
                    total_tokens,
                    cache_input_tokens,
                    total_cost,
                    non_success_cost,
                    total_latency_sample_count,
                    total_latency_sum_ms,
                    first_byte_sample_count,
                    first_byte_sum_ms,
                    first_byte_max_ms,
                    first_byte_histogram,
                    first_response_byte_total_sample_count,
                    first_response_byte_total_sum_ms,
                    first_response_byte_total_max_ms,
                    first_response_byte_total_histogram,
                    first_token_sample_count,
                    first_token_sum_ms,
                    first_token_max_ms,
                    first_token_histogram,
                    input_tokens,
                    output_tokens,
                    reasoning_tokens,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source) DO UPDATE SET
                    total_count = invocation_rollup_hourly.total_count + excluded.total_count,
                    success_count = invocation_rollup_hourly.success_count + excluded.success_count,
                    failure_count = invocation_rollup_hourly.failure_count + excluded.failure_count,
                    terminal_count = invocation_rollup_hourly.terminal_count + excluded.terminal_count,
                    terminal_tokens = invocation_rollup_hourly.terminal_tokens + excluded.terminal_tokens,
                    terminal_cost = invocation_rollup_hourly.terminal_cost + excluded.terminal_cost,
                    terminal_proof_complete = 0,
                    total_tokens = invocation_rollup_hourly.total_tokens + excluded.total_tokens,
                    cache_input_tokens = invocation_rollup_hourly.cache_input_tokens + excluded.cache_input_tokens,
                    input_tokens = invocation_rollup_hourly.input_tokens + excluded.input_tokens,
                    output_tokens = invocation_rollup_hourly.output_tokens + excluded.output_tokens,
                    reasoning_tokens = invocation_rollup_hourly.reasoning_tokens + excluded.reasoning_tokens,
                    total_cost = invocation_rollup_hourly.total_cost + excluded.total_cost,
                    non_success_cost = invocation_rollup_hourly.non_success_cost + excluded.non_success_cost,
                    total_latency_sample_count = invocation_rollup_hourly.total_latency_sample_count + excluded.total_latency_sample_count,
                    total_latency_sum_ms = invocation_rollup_hourly.total_latency_sum_ms + excluded.total_latency_sum_ms,
                    first_byte_sample_count = invocation_rollup_hourly.first_byte_sample_count + excluded.first_byte_sample_count,
                    first_byte_sum_ms = invocation_rollup_hourly.first_byte_sum_ms + excluded.first_byte_sum_ms,
                    first_byte_max_ms = MAX(invocation_rollup_hourly.first_byte_max_ms, excluded.first_byte_max_ms),
                    first_byte_histogram = excluded.first_byte_histogram,
                    first_response_byte_total_sample_count = invocation_rollup_hourly.first_response_byte_total_sample_count + excluded.first_response_byte_total_sample_count,
                    first_response_byte_total_sum_ms = invocation_rollup_hourly.first_response_byte_total_sum_ms + excluded.first_response_byte_total_sum_ms,
                    first_response_byte_total_max_ms = MAX(invocation_rollup_hourly.first_response_byte_total_max_ms, excluded.first_response_byte_total_max_ms),
                    first_response_byte_total_histogram = excluded.first_response_byte_total_histogram,
                    first_token_sample_count = invocation_rollup_hourly.first_token_sample_count + excluded.first_token_sample_count,
                    first_token_sum_ms = invocation_rollup_hourly.first_token_sum_ms + excluded.first_token_sum_ms,
                    first_token_max_ms = MAX(invocation_rollup_hourly.first_token_max_ms, excluded.first_token_max_ms),
                    first_token_histogram = excluded.first_token_histogram,
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(delta.total_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.terminal_count)
            .bind(delta.terminal_tokens)
            .bind(delta.terminal_cost)
            .bind(delta.total_tokens)
            .bind(delta.cache_input_tokens)
            .bind(delta.total_cost)
            .bind(delta.non_success_cost)
            .bind(delta.total_latency_sample_count)
            .bind(delta.total_latency_sum_ms)
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
            .bind(delta.first_token_sample_count)
            .bind(delta.first_token_sum_ms)
            .bind(delta.first_token_max_ms)
            .bind(encode_approx_histogram(&merged_first_token_histogram)?)
            .bind(delta.input_tokens)
            .bind(delta.output_tokens)
            .bind(delta.reasoning_tokens)
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

    if upsert_upstream_account_usage {
        for ((bucket_start_epoch, upstream_account_id), delta) in upstream_account_usage {
            sqlx::query(
                r#"
                INSERT INTO upstream_account_usage_hourly (
                    bucket_start_epoch,
                    upstream_account_id,
                    request_count,
                    success_count,
                    failure_count,
                    total_tokens,
                    total_cost,
                    non_success_cost,
                    input_tokens,
                    output_tokens,
                    cache_input_tokens,
                    reasoning_tokens,
                    first_seen_at,
                    last_seen_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, datetime('now'))
                ON CONFLICT(bucket_start_epoch, upstream_account_id) DO UPDATE SET
                    request_count = upstream_account_usage_hourly.request_count + excluded.request_count,
                    success_count = upstream_account_usage_hourly.success_count + excluded.success_count,
                    failure_count = upstream_account_usage_hourly.failure_count + excluded.failure_count,
                    total_tokens = upstream_account_usage_hourly.total_tokens + excluded.total_tokens,
                    total_cost = upstream_account_usage_hourly.total_cost + excluded.total_cost,
                    non_success_cost = upstream_account_usage_hourly.non_success_cost + excluded.non_success_cost,
                    input_tokens = upstream_account_usage_hourly.input_tokens + excluded.input_tokens,
                    output_tokens = upstream_account_usage_hourly.output_tokens + excluded.output_tokens,
                    cache_input_tokens = upstream_account_usage_hourly.cache_input_tokens + excluded.cache_input_tokens,
                    reasoning_tokens = upstream_account_usage_hourly.reasoning_tokens + excluded.reasoning_tokens,
                    first_seen_at = MIN(upstream_account_usage_hourly.first_seen_at, excluded.first_seen_at),
                    last_seen_at = MAX(upstream_account_usage_hourly.last_seen_at, excluded.last_seen_at),
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(upstream_account_id)
            .bind(delta.request_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.total_tokens)
            .bind(delta.total_cost)
            .bind(delta.non_success_cost)
            .bind(delta.input_tokens)
            .bind(delta.output_tokens)
            .bind(delta.cache_input_tokens)
            .bind(delta.reasoning_tokens)
            .bind(&delta.first_seen_at)
            .bind(&delta.last_seen_at)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_upstream_account_usage_breakdown {
        for (
            (
                bucket_start_epoch,
                source,
                upstream_account_key,
                upstream_account_id,
                normalized_model,
                normalized_reasoning_effort,
            ),
            delta,
        ) in upstream_account_usage_breakdown
        {
            sqlx::query(
                r#"
                INSERT INTO upstream_account_usage_breakdown_hourly (
                    bucket_start_epoch,
                    source,
                    upstream_account_key,
                    upstream_account_id,
                    normalized_model,
                    normalized_reasoning_effort,
                    request_count,
                    success_count,
                    failure_count,
                    cache_write_tokens,
                    cache_read_tokens,
                    output_tokens,
                    cost_input,
                    cost_cache_write,
                    cost_cache_read,
                    cost_output,
                    cost_reasoning,
                    cost_unknown,
                    has_cost,
                    performance_total_tokens,
                    performance_stream_output_tokens,
                    performance_stream_duration_ms,
                    performance_response_sample_count,
                    performance_response_sum_ms,
                    performance_first_byte_sample_count,
                    performance_first_byte_sum_ms,
                    performance_first_token_sample_count,
                    performance_first_token_sum_ms,
                    performance_usage_duration_sample_count,
                    performance_usage_duration_sum_ms,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source, upstream_account_key, normalized_model, normalized_reasoning_effort) DO UPDATE SET
                    request_count = upstream_account_usage_breakdown_hourly.request_count + excluded.request_count,
                    success_count = upstream_account_usage_breakdown_hourly.success_count + excluded.success_count,
                    failure_count = upstream_account_usage_breakdown_hourly.failure_count + excluded.failure_count,
                    cache_write_tokens = upstream_account_usage_breakdown_hourly.cache_write_tokens + excluded.cache_write_tokens,
                    cache_read_tokens = upstream_account_usage_breakdown_hourly.cache_read_tokens + excluded.cache_read_tokens,
                    output_tokens = upstream_account_usage_breakdown_hourly.output_tokens + excluded.output_tokens,
                    cost_input = upstream_account_usage_breakdown_hourly.cost_input + excluded.cost_input,
                    cost_cache_write = upstream_account_usage_breakdown_hourly.cost_cache_write + excluded.cost_cache_write,
                    cost_cache_read = upstream_account_usage_breakdown_hourly.cost_cache_read + excluded.cost_cache_read,
                    cost_output = upstream_account_usage_breakdown_hourly.cost_output + excluded.cost_output,
                    cost_reasoning = upstream_account_usage_breakdown_hourly.cost_reasoning + excluded.cost_reasoning,
                    cost_unknown = upstream_account_usage_breakdown_hourly.cost_unknown + excluded.cost_unknown,
                    has_cost = upstream_account_usage_breakdown_hourly.has_cost + excluded.has_cost,
                    performance_total_tokens = upstream_account_usage_breakdown_hourly.performance_total_tokens + excluded.performance_total_tokens,
                    performance_stream_output_tokens = upstream_account_usage_breakdown_hourly.performance_stream_output_tokens + excluded.performance_stream_output_tokens,
                    performance_stream_duration_ms = upstream_account_usage_breakdown_hourly.performance_stream_duration_ms + excluded.performance_stream_duration_ms,
                    performance_response_sample_count = upstream_account_usage_breakdown_hourly.performance_response_sample_count + excluded.performance_response_sample_count,
                    performance_response_sum_ms = upstream_account_usage_breakdown_hourly.performance_response_sum_ms + excluded.performance_response_sum_ms,
                    performance_first_byte_sample_count = upstream_account_usage_breakdown_hourly.performance_first_byte_sample_count + excluded.performance_first_byte_sample_count,
                    performance_first_byte_sum_ms = upstream_account_usage_breakdown_hourly.performance_first_byte_sum_ms + excluded.performance_first_byte_sum_ms,
                    performance_first_token_sample_count = upstream_account_usage_breakdown_hourly.performance_first_token_sample_count + excluded.performance_first_token_sample_count,
                    performance_first_token_sum_ms = upstream_account_usage_breakdown_hourly.performance_first_token_sum_ms + excluded.performance_first_token_sum_ms,
                    performance_usage_duration_sample_count = upstream_account_usage_breakdown_hourly.performance_usage_duration_sample_count + excluded.performance_usage_duration_sample_count,
                    performance_usage_duration_sum_ms = upstream_account_usage_breakdown_hourly.performance_usage_duration_sum_ms + excluded.performance_usage_duration_sum_ms,
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(&upstream_account_key)
            .bind(upstream_account_id)
            .bind(&normalized_model)
            .bind(&normalized_reasoning_effort)
            .bind(delta.request_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.cache_write_tokens)
            .bind(delta.cache_read_tokens)
            .bind(delta.output_tokens)
            .bind(delta.cost_input)
            .bind(delta.cost_cache_write)
            .bind(delta.cost_cache_read)
            .bind(delta.cost_output)
            .bind(delta.cost_reasoning)
            .bind(delta.cost_unknown)
            .bind(delta.has_cost)
            .bind(delta.performance_total_tokens)
            .bind(delta.performance_stream_output_tokens)
            .bind(delta.performance_stream_duration_ms)
            .bind(delta.performance_response_sample_count)
            .bind(delta.performance_response_sum_ms)
            .bind(delta.performance_first_byte_sample_count)
            .bind(delta.performance_first_byte_sum_ms)
            .bind(delta.performance_first_token_sample_count)
            .bind(delta.performance_first_token_sum_ms)
            .bind(delta.performance_usage_duration_sample_count)
            .bind(delta.performance_usage_duration_sum_ms)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_upstream_account_stats_hourly {
        #[derive(sqlx::FromRow)]
        struct AccountStatsHistogramRow {
            first_byte_histogram: String,
            first_response_byte_total_histogram: String,
            first_token_histogram: String,
        }

        for ((bucket_start_epoch, source, upstream_account_id), delta) in
            upstream_account_stats_hourly
        {
            let current_histograms = sqlx::query_as::<_, AccountStatsHistogramRow>(
                r#"
                SELECT
                    first_byte_histogram,
                    first_response_byte_total_histogram,
                    first_token_histogram
                FROM upstream_account_stats_hourly
                WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = ?3
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(upstream_account_id)
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
            let mut merged_first_token_histogram = current_histograms
                .as_ref()
                .map(|row| decode_approx_histogram(&row.first_token_histogram))
                .unwrap_or_else(empty_approx_histogram);
            merge_approx_histogram_into(
                &mut merged_first_token_histogram,
                &delta.first_token_histogram,
            )?;
            sqlx::query(
                r#"
                INSERT INTO upstream_account_stats_hourly (
                    bucket_start_epoch,
                    source,
                    upstream_account_id,
                    total_count,
                    success_count,
                    failure_count,
                    in_flight_count,
                    total_tokens,
                    input_tokens,
                    output_tokens,
                    cache_input_tokens,
                    total_cost,
                    non_success_cost,
                    total_latency_sample_count,
                    total_latency_sum_ms,
                    first_byte_sample_count,
                    first_byte_sum_ms,
                    first_byte_max_ms,
                    first_byte_histogram,
                    first_response_byte_total_sample_count,
                    first_response_byte_total_sum_ms,
                    first_response_byte_total_max_ms,
                    first_response_byte_total_histogram,
                    first_token_sample_count,
                    first_token_sum_ms,
                    first_token_max_ms,
                    first_token_histogram,
                    reasoning_tokens,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source, upstream_account_id) DO UPDATE SET
                    total_count = upstream_account_stats_hourly.total_count + excluded.total_count,
                    success_count = upstream_account_stats_hourly.success_count + excluded.success_count,
                    failure_count = upstream_account_stats_hourly.failure_count + excluded.failure_count,
                    in_flight_count = upstream_account_stats_hourly.in_flight_count + excluded.in_flight_count,
                    total_tokens = upstream_account_stats_hourly.total_tokens + excluded.total_tokens,
                    input_tokens = upstream_account_stats_hourly.input_tokens + excluded.input_tokens,
                    output_tokens = upstream_account_stats_hourly.output_tokens + excluded.output_tokens,
                    cache_input_tokens = upstream_account_stats_hourly.cache_input_tokens + excluded.cache_input_tokens,
                    reasoning_tokens = upstream_account_stats_hourly.reasoning_tokens + excluded.reasoning_tokens,
                    total_cost = upstream_account_stats_hourly.total_cost + excluded.total_cost,
                    non_success_cost = upstream_account_stats_hourly.non_success_cost + excluded.non_success_cost,
                    total_latency_sample_count = upstream_account_stats_hourly.total_latency_sample_count + excluded.total_latency_sample_count,
                    total_latency_sum_ms = upstream_account_stats_hourly.total_latency_sum_ms + excluded.total_latency_sum_ms,
                    first_byte_sample_count = upstream_account_stats_hourly.first_byte_sample_count + excluded.first_byte_sample_count,
                    first_byte_sum_ms = upstream_account_stats_hourly.first_byte_sum_ms + excluded.first_byte_sum_ms,
                    first_byte_max_ms = MAX(upstream_account_stats_hourly.first_byte_max_ms, excluded.first_byte_max_ms),
                    first_byte_histogram = excluded.first_byte_histogram,
                    first_response_byte_total_sample_count = upstream_account_stats_hourly.first_response_byte_total_sample_count + excluded.first_response_byte_total_sample_count,
                    first_response_byte_total_sum_ms = upstream_account_stats_hourly.first_response_byte_total_sum_ms + excluded.first_response_byte_total_sum_ms,
                    first_response_byte_total_max_ms = MAX(upstream_account_stats_hourly.first_response_byte_total_max_ms, excluded.first_response_byte_total_max_ms),
                    first_response_byte_total_histogram = excluded.first_response_byte_total_histogram,
                    first_token_sample_count = upstream_account_stats_hourly.first_token_sample_count + excluded.first_token_sample_count,
                    first_token_sum_ms = upstream_account_stats_hourly.first_token_sum_ms + excluded.first_token_sum_ms,
                    first_token_max_ms = MAX(upstream_account_stats_hourly.first_token_max_ms, excluded.first_token_max_ms),
                    first_token_histogram = excluded.first_token_histogram,
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(upstream_account_id)
            .bind(delta.total_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.in_flight_count)
            .bind(delta.total_tokens)
            .bind(delta.input_tokens)
            .bind(delta.output_tokens)
            .bind(delta.cache_input_tokens)
            .bind(delta.total_cost)
            .bind(delta.non_success_cost)
            .bind(delta.total_latency_sample_count)
            .bind(delta.total_latency_sum_ms)
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
            .bind(delta.first_token_sample_count)
            .bind(delta.first_token_sum_ms)
            .bind(delta.first_token_max_ms)
            .bind(encode_approx_histogram(&merged_first_token_histogram)?)
            .bind(delta.reasoning_tokens)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_upstream_account_activity_v2 {
        #[derive(sqlx::FromRow)]
        struct ActivityV2StatsHistogramRow {
            activity_v2_first_token_histogram: String,
        }

        for ((bucket_start_epoch, source, upstream_account_id), delta) in
            upstream_account_activity_v2
        {
            let current_histogram = sqlx::query_as::<_, ActivityV2StatsHistogramRow>(
                r#"
                SELECT activity_v2_first_token_histogram
                FROM upstream_account_stats_hourly
                WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = ?3
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(upstream_account_id)
            .fetch_optional(&mut *tx)
            .await?;
            let mut merged_first_token_histogram = current_histogram
                .as_ref()
                .map(|row| decode_approx_histogram(&row.activity_v2_first_token_histogram))
                .unwrap_or_else(empty_approx_histogram);
            merge_approx_histogram_into(
                &mut merged_first_token_histogram,
                &delta.first_token_histogram,
            )?;
            sqlx::query(
                r#"
                INSERT INTO upstream_account_stats_hourly (
                    bucket_start_epoch,
                    source,
                    upstream_account_id,
                    activity_v2_request_count,
                    activity_v2_success_count,
                    activity_v2_failure_count,
                    activity_v2_non_success_count,
                    activity_v2_total_tokens,
                    activity_v2_success_tokens,
                    activity_v2_non_success_tokens,
                    activity_v2_failure_tokens,
                    activity_v2_failure_cost,
                    activity_v2_non_success_cost,
                    activity_v2_cache_input_tokens,
                    activity_v2_total_cost,
                    activity_v2_first_response_sample_count,
                    activity_v2_first_response_sum_ms,
                    activity_v2_total_latency_sample_count,
                    activity_v2_total_latency_sum_ms,
                    activity_v2_last_invocation_at,
                    activity_v2_latest_unkeyed_conversation_at,
                    activity_v2_latest_first_response_at,
                    activity_v2_latest_first_response_ms,
                    activity_v2_latest_total_latency_at,
                    activity_v2_latest_total_latency_ms,
                    activity_v2_first_token_sample_count,
                    activity_v2_first_token_sum_ms,
                    activity_v2_first_token_max_ms,
                    activity_v2_first_token_histogram,
                    updated_at
                )
                VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                    ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23,
                    ?24, ?25, ?26, ?27, ?28, ?29, datetime('now')
                )
                ON CONFLICT(bucket_start_epoch, source, upstream_account_id) DO UPDATE SET
                    activity_v2_request_count = upstream_account_stats_hourly.activity_v2_request_count + excluded.activity_v2_request_count,
                    activity_v2_success_count = upstream_account_stats_hourly.activity_v2_success_count + excluded.activity_v2_success_count,
                    activity_v2_failure_count = upstream_account_stats_hourly.activity_v2_failure_count + excluded.activity_v2_failure_count,
                    activity_v2_non_success_count = upstream_account_stats_hourly.activity_v2_non_success_count + excluded.activity_v2_non_success_count,
                    activity_v2_total_tokens = upstream_account_stats_hourly.activity_v2_total_tokens + excluded.activity_v2_total_tokens,
                    activity_v2_success_tokens = upstream_account_stats_hourly.activity_v2_success_tokens + excluded.activity_v2_success_tokens,
                    activity_v2_non_success_tokens = upstream_account_stats_hourly.activity_v2_non_success_tokens + excluded.activity_v2_non_success_tokens,
                    activity_v2_failure_tokens = upstream_account_stats_hourly.activity_v2_failure_tokens + excluded.activity_v2_failure_tokens,
                    activity_v2_failure_cost = upstream_account_stats_hourly.activity_v2_failure_cost + excluded.activity_v2_failure_cost,
                    activity_v2_non_success_cost = upstream_account_stats_hourly.activity_v2_non_success_cost + excluded.activity_v2_non_success_cost,
                    activity_v2_cache_input_tokens = upstream_account_stats_hourly.activity_v2_cache_input_tokens + excluded.activity_v2_cache_input_tokens,
                    activity_v2_total_cost = upstream_account_stats_hourly.activity_v2_total_cost + excluded.activity_v2_total_cost,
                    activity_v2_first_response_sample_count = upstream_account_stats_hourly.activity_v2_first_response_sample_count + excluded.activity_v2_first_response_sample_count,
                    activity_v2_first_response_sum_ms = upstream_account_stats_hourly.activity_v2_first_response_sum_ms + excluded.activity_v2_first_response_sum_ms,
                    activity_v2_total_latency_sample_count = upstream_account_stats_hourly.activity_v2_total_latency_sample_count + excluded.activity_v2_total_latency_sample_count,
                    activity_v2_total_latency_sum_ms = upstream_account_stats_hourly.activity_v2_total_latency_sum_ms + excluded.activity_v2_total_latency_sum_ms,
                    activity_v2_last_invocation_at = CASE
                        WHEN upstream_account_stats_hourly.activity_v2_last_invocation_at IS NULL
                          OR excluded.activity_v2_last_invocation_at > upstream_account_stats_hourly.activity_v2_last_invocation_at
                        THEN excluded.activity_v2_last_invocation_at
                        ELSE upstream_account_stats_hourly.activity_v2_last_invocation_at
                    END,
                    activity_v2_latest_unkeyed_conversation_at = CASE
                        WHEN upstream_account_stats_hourly.activity_v2_latest_unkeyed_conversation_at IS NULL
                          OR excluded.activity_v2_latest_unkeyed_conversation_at > upstream_account_stats_hourly.activity_v2_latest_unkeyed_conversation_at
                        THEN excluded.activity_v2_latest_unkeyed_conversation_at
                        ELSE upstream_account_stats_hourly.activity_v2_latest_unkeyed_conversation_at
                    END,
                    activity_v2_latest_first_response_ms = CASE
                        WHEN upstream_account_stats_hourly.activity_v2_latest_first_response_at IS NULL
                          OR excluded.activity_v2_latest_first_response_at > upstream_account_stats_hourly.activity_v2_latest_first_response_at
                        THEN excluded.activity_v2_latest_first_response_ms
                        ELSE upstream_account_stats_hourly.activity_v2_latest_first_response_ms
                    END,
                    activity_v2_latest_first_response_at = CASE
                        WHEN upstream_account_stats_hourly.activity_v2_latest_first_response_at IS NULL
                          OR excluded.activity_v2_latest_first_response_at > upstream_account_stats_hourly.activity_v2_latest_first_response_at
                        THEN excluded.activity_v2_latest_first_response_at
                        ELSE upstream_account_stats_hourly.activity_v2_latest_first_response_at
                    END,
                    activity_v2_latest_total_latency_ms = CASE
                        WHEN upstream_account_stats_hourly.activity_v2_latest_total_latency_at IS NULL
                          OR excluded.activity_v2_latest_total_latency_at > upstream_account_stats_hourly.activity_v2_latest_total_latency_at
                        THEN excluded.activity_v2_latest_total_latency_ms
                        ELSE upstream_account_stats_hourly.activity_v2_latest_total_latency_ms
                    END,
                    activity_v2_latest_total_latency_at = CASE
                        WHEN upstream_account_stats_hourly.activity_v2_latest_total_latency_at IS NULL
                          OR excluded.activity_v2_latest_total_latency_at > upstream_account_stats_hourly.activity_v2_latest_total_latency_at
                        THEN excluded.activity_v2_latest_total_latency_at
                        ELSE upstream_account_stats_hourly.activity_v2_latest_total_latency_at
                    END,
                    activity_v2_first_token_sample_count = upstream_account_stats_hourly.activity_v2_first_token_sample_count + excluded.activity_v2_first_token_sample_count,
                    activity_v2_first_token_sum_ms = upstream_account_stats_hourly.activity_v2_first_token_sum_ms + excluded.activity_v2_first_token_sum_ms,
                    activity_v2_first_token_max_ms = MAX(upstream_account_stats_hourly.activity_v2_first_token_max_ms, excluded.activity_v2_first_token_max_ms),
                    activity_v2_first_token_histogram = excluded.activity_v2_first_token_histogram,
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(upstream_account_id)
            .bind(delta.total_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.non_success_count)
            .bind(delta.total_tokens)
            .bind(delta.success_tokens)
            .bind(delta.non_success_tokens)
            .bind(delta.failure_tokens)
            .bind(delta.failure_cost)
            .bind(delta.non_success_cost)
            .bind(delta.cache_input_tokens)
            .bind(delta.total_cost)
            .bind(delta.first_response_byte_total_sample_count)
            .bind(delta.first_response_byte_total_sum_ms)
            .bind(delta.total_latency_sample_count)
            .bind(delta.total_latency_sum_ms)
            .bind(delta.last_invocation_at.as_deref())
            .bind(delta.latest_unkeyed_conversation_at.as_deref())
            .bind(delta.latest_first_response_byte_total_at.as_deref())
            .bind(delta.latest_first_response_byte_total_ms)
            .bind(delta.latest_total_latency_at.as_deref())
            .bind(delta.latest_total_latency_ms)
            .bind(delta.first_token_sample_count)
            .bind(delta.first_token_sum_ms)
            .bind(delta.first_token_max_ms)
            .bind(encode_approx_histogram(&merged_first_token_histogram)?)
            .execute(&mut *tx)
            .await?;
        }
    }

    if upsert_upstream_account_stats_minute {
        #[derive(sqlx::FromRow)]
        struct AccountMinuteStatsHistogramRow {
            first_byte_histogram: String,
            first_response_byte_total_histogram: String,
            first_token_histogram: String,
        }

        for ((bucket_start_epoch, source, upstream_account_id), delta) in
            upstream_account_stats_minute
        {
            let current_histograms = sqlx::query_as::<_, AccountMinuteStatsHistogramRow>(
                r#"
                SELECT
                    first_byte_histogram,
                    first_response_byte_total_histogram,
                    first_token_histogram
                FROM upstream_account_stats_minute
                WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = ?3
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(upstream_account_id)
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
            let mut merged_first_token_histogram = current_histograms
                .as_ref()
                .map(|row| decode_approx_histogram(&row.first_token_histogram))
                .unwrap_or_else(empty_approx_histogram);
            merge_approx_histogram_into(
                &mut merged_first_token_histogram,
                &delta.first_token_histogram,
            )?;
            sqlx::query(
                r#"
                INSERT INTO upstream_account_stats_minute (
                    bucket_start_epoch,
                    source,
                    upstream_account_id,
                    total_count,
                    success_count,
                    failure_count,
                    in_flight_count,
                    total_tokens,
                    input_tokens,
                    output_tokens,
                    cache_input_tokens,
                    total_cost,
                    non_success_cost,
                    total_latency_sample_count,
                    total_latency_sum_ms,
                    first_byte_sample_count,
                    first_byte_sum_ms,
                    first_byte_max_ms,
                    first_byte_histogram,
                    first_response_byte_total_sample_count,
                    first_response_byte_total_sum_ms,
                    first_response_byte_total_max_ms,
                    first_response_byte_total_histogram,
                    first_token_sample_count,
                    first_token_sum_ms,
                    first_token_max_ms,
                    first_token_histogram,
                    reasoning_tokens,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source, upstream_account_id) DO UPDATE SET
                    total_count = upstream_account_stats_minute.total_count + excluded.total_count,
                    success_count = upstream_account_stats_minute.success_count + excluded.success_count,
                    failure_count = upstream_account_stats_minute.failure_count + excluded.failure_count,
                    in_flight_count = upstream_account_stats_minute.in_flight_count + excluded.in_flight_count,
                    total_tokens = upstream_account_stats_minute.total_tokens + excluded.total_tokens,
                    input_tokens = upstream_account_stats_minute.input_tokens + excluded.input_tokens,
                    output_tokens = upstream_account_stats_minute.output_tokens + excluded.output_tokens,
                    cache_input_tokens = upstream_account_stats_minute.cache_input_tokens + excluded.cache_input_tokens,
                    reasoning_tokens = upstream_account_stats_minute.reasoning_tokens + excluded.reasoning_tokens,
                    total_cost = upstream_account_stats_minute.total_cost + excluded.total_cost,
                    non_success_cost = upstream_account_stats_minute.non_success_cost + excluded.non_success_cost,
                    total_latency_sample_count = upstream_account_stats_minute.total_latency_sample_count + excluded.total_latency_sample_count,
                    total_latency_sum_ms = upstream_account_stats_minute.total_latency_sum_ms + excluded.total_latency_sum_ms,
                    first_byte_sample_count = upstream_account_stats_minute.first_byte_sample_count + excluded.first_byte_sample_count,
                    first_byte_sum_ms = upstream_account_stats_minute.first_byte_sum_ms + excluded.first_byte_sum_ms,
                    first_byte_max_ms = MAX(upstream_account_stats_minute.first_byte_max_ms, excluded.first_byte_max_ms),
                    first_byte_histogram = excluded.first_byte_histogram,
                    first_response_byte_total_sample_count = upstream_account_stats_minute.first_response_byte_total_sample_count + excluded.first_response_byte_total_sample_count,
                    first_response_byte_total_sum_ms = upstream_account_stats_minute.first_response_byte_total_sum_ms + excluded.first_response_byte_total_sum_ms,
                    first_response_byte_total_max_ms = MAX(upstream_account_stats_minute.first_response_byte_total_max_ms, excluded.first_response_byte_total_max_ms),
                    first_response_byte_total_histogram = excluded.first_response_byte_total_histogram,
                    first_token_sample_count = upstream_account_stats_minute.first_token_sample_count + excluded.first_token_sample_count,
                    first_token_sum_ms = upstream_account_stats_minute.first_token_sum_ms + excluded.first_token_sum_ms,
                    first_token_max_ms = MAX(upstream_account_stats_minute.first_token_max_ms, excluded.first_token_max_ms),
                    first_token_histogram = excluded.first_token_histogram,
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(upstream_account_id)
            .bind(delta.total_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.in_flight_count)
            .bind(delta.total_tokens)
            .bind(delta.input_tokens)
            .bind(delta.output_tokens)
            .bind(delta.cache_input_tokens)
            .bind(delta.total_cost)
            .bind(delta.non_success_cost)
            .bind(delta.total_latency_sample_count)
            .bind(delta.total_latency_sum_ms)
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
            .bind(delta.first_token_sample_count)
            .bind(delta.first_token_sum_ms)
            .bind(delta.first_token_max_ms)
            .bind(encode_approx_histogram(&merged_first_token_histogram)?)
            .bind(delta.reasoning_tokens)
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
            | HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE
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

pub(crate) async fn delete_rollup_rows_for_bucket_epochs_with_size_tx(
    tx: &mut SqliteConnection,
    table: &str,
    bucket_epochs: &[i64],
    bucket_seconds: i64,
) -> Result<()> {
    if bucket_epochs.is_empty() {
        return Ok(());
    }
    let normalized = if bucket_seconds == 3_600 {
        bucket_epochs.to_vec()
    } else {
        let mut values = Vec::new();
        for hour_epoch in bucket_epochs {
            let mut cursor = *hour_epoch;
            let hour_end = hour_epoch.saturating_add(3_600);
            while cursor < hour_end {
                values.push(cursor);
                cursor = cursor.saturating_add(bucket_seconds);
            }
        }
        values.sort_unstable();
        values.dedup();
        values
    };
    delete_hourly_rollup_rows_for_bucket_epochs_tx(tx, table, &normalized).await
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
    let upstream_account_id_sql = live_invocation_upstream_account_id_sql(
        "codex_invocations",
        load_pool_attempt_fallback_capability_tx(tx).await?,
    );
    let first_token_ms_sql = live_invocation_first_token_ms_sql_tx(tx).await?;

    let rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(&format!(
        "SELECT \
            id,
            occurred_at,
            source,
            status,
            detail_level,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            total_tokens,
            cost,
            {} AS upstream_account_id,
            cost_input,
            cost_cache_write,
            cost_cache_read,
            cost_output,
            cost_reasoning,
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
            {} AS first_token_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
         FROM codex_invocations
         WHERE occurred_at >= ?1
           AND occurred_at < ?2
         ORDER BY id ASC",
        upstream_account_id_sql, first_token_ms_sql,
    ))
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
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_STICKY_KEYS,
    ] {
        delete_hourly_rollup_rows_for_bucket_epochs_tx(tx, table, &bucket_epochs).await?;
    }
    delete_rollup_rows_for_bucket_epochs_with_size_tx(
        tx,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE,
        &bucket_epochs,
        60,
    )
    .await?;

    let rows = load_live_invocation_hourly_rows_for_bucket_epochs_tx(tx, &bucket_epochs).await?;
    upsert_invocation_hourly_rollups_tx(tx, &rows, &INVOCATION_HOURLY_ROLLUP_TARGETS).await?;
    rebuild_parallel_work_rollups_for_hours_tx(tx, &bucket_epochs).await?;
    let mut bucket_watermarks = BTreeMap::<i64, i64>::new();
    for row in rows
        .iter()
        .filter(|row| invocation_row_counts_toward_account_activity_v2(row))
    {
        let bucket_epoch = invocation_bucket_start_epoch(&row.occurred_at)?;
        bucket_watermarks
            .entry(bucket_epoch)
            .and_modify(|cursor_id| *cursor_id = (*cursor_id).max(row.id))
            .or_insert(row.id);
    }
    for (bucket_epoch, cursor_id) in bucket_watermarks {
        save_account_activity_v2_bucket_repair_watermark_tx(tx, bucket_epoch, cursor_id).await?;
    }
    Ok(())
}

pub(crate) async fn replay_live_invocation_hourly_rollups(pool: &Pool<Sqlite>) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress(pool, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let account_activity_v2_cursor = load_hourly_rollup_live_progress(
        pool,
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
    )
    .await?;
    let rows = {
        let mut conn = pool.acquire().await?;
        let upstream_account_id_sql = live_invocation_upstream_account_id_sql(
            "codex_invocations",
            load_pool_attempt_fallback_capability_tx(&mut conn).await?,
        );
        let first_token_ms_sql = live_invocation_first_token_ms_sql_tx(&mut conn).await?;
        sqlx::query_as::<_, InvocationHourlySourceRecord>(&format!(
            r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            total_tokens,
            cost,
            {} AS upstream_account_id,
            cost_input,
            cost_cache_write,
            cost_cache_read,
            cost_output,
            cost_reasoning,
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
            {} AS first_token_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
            upstream_account_id_sql, first_token_ms_sql,
        ))
        .bind(cursor_id)
        .bind(BACKFILL_BATCH_SIZE)
        .fetch_all(&mut *conn)
        .await?
    };
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    let mut tx = pool.begin().await?;
    upsert_live_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &rows,
        account_activity_v2_cursor,
        cursor_id,
    )
    .await?;
    save_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS, last_id)
        .await?;
    if account_activity_v2_cursor == cursor_id {
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
            last_id,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(rows.len() as u64)
}

pub(crate) async fn replay_live_invocation_hourly_rollups_tx(
    tx: &mut SqliteConnection,
) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let account_activity_v2_cursor = load_hourly_rollup_live_progress_tx(
        tx,
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
    )
    .await?;
    let upstream_account_id_sql = live_invocation_upstream_account_id_sql(
        "codex_invocations",
        load_pool_attempt_fallback_capability_tx(tx).await?,
    );
    let first_token_ms_sql = live_invocation_first_token_ms_sql_tx(tx).await?;
    let rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(&format!(
        r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            total_tokens,
            cost,
            {} AS upstream_account_id,
            cost_input,
            cost_cache_write,
            cost_cache_read,
            cost_output,
            cost_reasoning,
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
            {} AS first_token_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
        upstream_account_id_sql, first_token_ms_sql,
    ))
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(&mut *tx)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    upsert_live_invocation_hourly_rollups_tx(tx, &rows, account_activity_v2_cursor, cursor_id)
        .await?;
    save_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_INVOCATIONS, last_id).await?;
    if account_activity_v2_cursor == cursor_id {
        save_hourly_rollup_live_progress_tx(
            tx,
            INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
            last_id,
        )
        .await?;
    }
    Ok(rows.len() as u64)
}

async fn upsert_live_invocation_hourly_rollups_tx(
    tx: &mut SqliteConnection,
    rows: &[InvocationHourlySourceRecord],
    account_activity_v2_cursor: i64,
    shared_cursor: i64,
) -> Result<()> {
    let base_targets = INVOCATION_HOURLY_ROLLUP_TARGETS
        .iter()
        .copied()
        .filter(|target| *target != HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
        .collect::<Vec<_>>();
    upsert_invocation_hourly_rollups_tx(tx, rows, &base_targets).await?;

    if account_activity_v2_cursor == shared_cursor {
        let bucket_watermarks =
            load_account_activity_v2_bucket_repair_watermarks_for_rows_tx(tx, rows).await?;
        let v2_rows = rows
            .iter()
            .filter(|row| {
                invocation_bucket_start_epoch(&row.occurred_at)
                    .map(|bucket_epoch| {
                        row.id
                            > bucket_watermarks
                                .get(&bucket_epoch)
                                .copied()
                                .unwrap_or_default()
                    })
                    .unwrap_or(true)
            })
            .cloned()
            .collect::<Vec<_>>();
        upsert_invocation_hourly_rollups_tx(
            tx,
            &v2_rows,
            &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2],
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn repair_live_invocation_account_activity_v2_once(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    let mut tx = pool.begin().await?;
    ensure_account_activity_v2_repair_generation_tx(tx.as_mut()).await?;
    let shared_live_cursor =
        load_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let repair_cursor = load_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
    )
    .await?;

    if repair_cursor >= shared_live_cursor {
        tx.commit().await?;
        return Ok(0);
    }

    let upstream_account_id_sql = live_invocation_upstream_account_id_sql(
        "codex_invocations",
        load_pool_attempt_fallback_capability_tx(tx.as_mut()).await?,
    );
    let first_token_ms_sql = live_invocation_first_token_ms_sql_tx(tx.as_mut()).await?;
    let rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(&format!(
        r#"
        SELECT
            id, occurred_at, source, status, detail_level, model,
            input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost,
            {} AS upstream_account_id,
            cost_input, cost_cache_write, cost_cache_read, cost_output, cost_reasoning,
            error_message, failure_kind, failure_class, is_actionable, payload,
            t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms,
            t_upstream_ttfb_ms, {} AS first_token_ms, t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms
        FROM codex_invocations
        WHERE id > ?1 AND id <= ?2
        ORDER BY id ASC
        LIMIT ?3
        "#,
        upstream_account_id_sql, first_token_ms_sql,
    ))
    .bind(repair_cursor)
    .bind(shared_live_cursor)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(tx.as_mut())
    .await?;

    if rows.is_empty() {
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
            shared_live_cursor,
        )
        .await?;
        tx.commit().await?;
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(repair_cursor);
    let bucket_watermarks =
        load_account_activity_v2_bucket_repair_watermarks_for_rows_tx(tx.as_mut(), &rows).await?;
    let rows_to_replay = rows
        .iter()
        .filter(|row| {
            invocation_bucket_start_epoch(&row.occurred_at)
                .map(|bucket_epoch| {
                    row.id
                        > bucket_watermarks
                            .get(&bucket_epoch)
                            .copied()
                            .unwrap_or_default()
                })
                .unwrap_or(true)
        })
        .cloned()
        .collect::<Vec<_>>();
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &rows_to_replay,
        &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2],
    )
    .await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
        last_id,
    )
    .await?;
    mark_live_account_activity_v2_coverage_tx(tx.as_mut(), last_id, &rows).await?;
    tx.commit().await?;
    tracing::debug!(
        archive_replay_target = HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2,
        repair_cursor_before = repair_cursor,
        repair_cursor_after = last_id,
        shared_live_cursor,
        repaired_row_count = rows.len(),
        coverage_complete = last_id >= shared_live_cursor,
        "repaired live account activity v2 rollup batch"
    );
    Ok(rows.len() as u64)
}
