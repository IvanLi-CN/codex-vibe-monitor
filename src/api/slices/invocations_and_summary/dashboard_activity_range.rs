#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AccountActivityRangeBuildTelemetry {
    pub(crate) covered_hour_count: usize,
    pub(crate) fallback_hour_count: usize,
    pub(crate) boundary_tail_count: usize,
    pub(crate) rollup_row_count: usize,
    pub(crate) raw_fallback_range_count: usize,
}

pub(crate) struct AccountActivityRangeBuild {
    pub(crate) rows: Vec<UpstreamAccountActivityAggregateRow>,
    pub(crate) telemetry: AccountActivityRangeBuildTelemetry,
}

fn merge_account_activity_rows(
    target: &mut UpstreamAccountActivityAggregateRow,
    source: UpstreamAccountActivityAggregateRow,
) {
    merge_latest_optional_timestamp(
        &mut target.latest_conversation_created_at,
        source.latest_conversation_created_at,
    );
    merge_latest_optional_timestamp(&mut target.last_invocation_at, source.last_invocation_at);
    target.request_count += source.request_count;
    target.success_count += source.success_count;
    target.failure_count += source.failure_count;
    target.non_success_count += source.non_success_count;
    target.total_tokens += source.total_tokens;
    target.success_tokens += source.success_tokens;
    target.non_success_tokens += source.non_success_tokens;
    target.failure_tokens += source.failure_tokens;
    target.failure_cost += source.failure_cost;
    target.non_success_cost += source.non_success_cost;
    target.cache_input_tokens += source.cache_input_tokens;
    target.total_cost += source.total_cost;
    target.first_response_byte_total_sample_count += source.first_response_byte_total_sample_count;
    target.first_response_byte_total_sum_ms += source.first_response_byte_total_sum_ms;
    target.first_token_sample_count += source.first_token_sample_count;
    target.first_token_sum_ms += source.first_token_sum_ms;
    target.total_latency_sample_count += source.total_latency_sample_count;
    target.total_latency_sum_ms += source.total_latency_sum_ms;
    merge_latest_timed_metric(
        &mut target.latest_first_response_byte_total_at,
        &mut target.latest_first_response_byte_total_ms,
        source.latest_first_response_byte_total_at,
        source.latest_first_response_byte_total_ms,
    );
    merge_latest_timed_metric(
        &mut target.latest_avg_total_at,
        &mut target.latest_avg_total_ms,
        source.latest_avg_total_at,
        source.latest_avg_total_ms,
    );
}

async fn query_account_activity_v2_rollup_rows(
    tx: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    bucket_epochs: &[i64],
) -> Result<Vec<UpstreamAccountActivityAggregateRow>, ApiError> {
    if bucket_epochs.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            CASE WHEN upstream_account_id = -1 THEN NULL ELSE upstream_account_id END AS upstream_account_id,
            activity_v2_latest_unkeyed_conversation_at AS latest_conversation_created_at,
            activity_v2_last_invocation_at AS last_invocation_at,
            activity_v2_request_count AS request_count,
            activity_v2_success_count AS success_count,
            activity_v2_failure_count AS failure_count,
            activity_v2_non_success_count AS non_success_count,
            activity_v2_total_tokens AS total_tokens,
            activity_v2_success_tokens AS success_tokens,
            activity_v2_non_success_tokens AS non_success_tokens,
            activity_v2_failure_tokens AS failure_tokens,
            activity_v2_failure_cost AS failure_cost,
            activity_v2_non_success_cost AS non_success_cost,
            activity_v2_cache_input_tokens AS cache_input_tokens,
            activity_v2_total_cost AS total_cost,
            activity_v2_first_response_sample_count AS first_response_byte_total_sample_count,
            activity_v2_first_response_sum_ms AS first_response_byte_total_sum_ms,
            activity_v2_first_token_sample_count AS first_token_sample_count,
            activity_v2_first_token_sum_ms AS first_token_sum_ms,
            activity_v2_total_latency_sample_count AS total_latency_sample_count,
            activity_v2_total_latency_sum_ms AS total_latency_sum_ms,
            activity_v2_latest_first_response_at AS latest_first_response_byte_total_at,
            activity_v2_latest_first_response_ms AS latest_first_response_byte_total_ms,
            activity_v2_latest_total_latency_at AS latest_avg_total_at,
            activity_v2_latest_total_latency_ms AS latest_avg_total_ms
        FROM upstream_account_stats_hourly
        WHERE bucket_start_epoch IN (
        "#,
    );
    {
        let mut separated = query.separated(", ");
        for bucket_epoch in bucket_epochs {
            separated.push_bind(*bucket_epoch);
        }
    }
    query.push(")");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    Ok(query
        .build_query_as::<UpstreamAccountActivityAggregateRow>()
        .fetch_all(&mut *tx)
        .await?)
}

#[derive(Debug, FromRow)]
struct AccountActivityConversationRollupRow {
    upstream_account_id: Option<i64>,
    latest_conversation_created_at: Option<String>,
}

async fn query_account_activity_conversation_rollup_rows(
    tx: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    bucket_epochs: &[i64],
) -> Result<Vec<AccountActivityConversationRollupRow>, ApiError> {
    if bucket_epochs.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        WITH active_keys AS (
            SELECT DISTINCT source, upstream_account_key, upstream_account_id, prompt_cache_key
            FROM prompt_cache_upstream_account_hourly
            WHERE bucket_start_epoch IN (
        "#,
    );
    {
        let mut separated = query.separated(", ");
        for bucket_epoch in bucket_epochs {
            separated.push_bind(*bucket_epoch);
        }
    }
    query.push(")");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(
        r#"
        ), conversation_starts AS (
            SELECT
                active.upstream_account_key,
                active.upstream_account_id,
                active.prompt_cache_key,
                MIN(all_rows.first_seen_at) AS conversation_created_at
            FROM active_keys AS active
            JOIN prompt_cache_upstream_account_hourly AS all_rows
             ON all_rows.upstream_account_key = active.upstream_account_key
             AND all_rows.prompt_cache_key = active.prompt_cache_key
        "#,
    );
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND all_rows.source = active.source");
    }
    query.push(
        r#"
            GROUP BY active.upstream_account_key, active.upstream_account_id, active.prompt_cache_key
        )
        SELECT
            upstream_account_id,
            MAX(conversation_created_at) AS latest_conversation_created_at
        FROM conversation_starts
        GROUP BY upstream_account_key, upstream_account_id
        "#,
    );
    Ok(query
        .build_query_as::<AccountActivityConversationRollupRow>()
        .fetch_all(&mut *tx)
        .await?)
}

async fn load_account_activity_v2_covered_hours(
    tx: &mut SqliteConnection,
    start_epoch: i64,
    end_epoch: i64,
) -> Result<HashSet<i64>, ApiError> {
    Ok(sqlx::query_scalar::<_, i64>(
        r#"
        SELECT bucket_start_epoch
        FROM hourly_rollup_materialized_buckets
        WHERE target = ?1
          AND source = ?2
          AND bucket_start_epoch >= ?3
          AND bucket_start_epoch < ?4
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
    .bind(HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE)
    .bind(start_epoch)
    .bind(end_epoch)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect())
}

async fn load_account_activity_v2_bucket_repair_watermarks(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    bucket_epochs: &[i64],
) -> Result<BTreeMap<i64, i64>, ApiError> {
    if bucket_epochs.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT bucket_start_epoch, cursor_id FROM account_activity_v2_bucket_repair_watermarks WHERE bucket_start_epoch IN (",
    );
    {
        let mut separated = query.separated(", ");
        for bucket_epoch in bucket_epochs {
            separated.push_bind(*bucket_epoch);
        }
    }
    query.push(")");
    Ok(query
        .build_query_as::<(i64, i64)>()
        .fetch_all(executor)
        .await?
        .into_iter()
        .collect())
}

struct AccountActivityRangePlan {
    covered_hours: Vec<i64>,
    uncovered_hours: Vec<i64>,
    exact_ranges: Vec<ExactUtcRange>,
    boundary_tail_count: usize,
}

async fn plan_account_activity_range(
    state: &AppState,
    range: ExactUtcRange,
) -> Result<AccountActivityRangePlan, ApiError> {
    let mut tx = state.pool.begin().await?;
    let plan = plan_account_activity_range_tx(state, tx.as_mut(), range).await?;
    tx.commit().await?;
    Ok(plan)
}

async fn plan_account_activity_range_tx(
    state: &AppState,
    tx: &mut SqliteConnection,
    range: ExactUtcRange,
) -> Result<AccountActivityRangePlan, ApiError> {
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    let live_start = range.start.max(retention_cutoff);
    if live_start >= range.end {
        return Ok(AccountActivityRangePlan {
            covered_hours: Vec::new(),
            uncovered_hours: Vec::new(),
            exact_ranges: Vec::new(),
            boundary_tail_count: 0,
        });
    }

    let full_start_epoch = ceil_hour_epoch(live_start.timestamp());
    let full_end_epoch = align_bucket_epoch(range.end.timestamp(), 3_600, 0);
    let covered =
        load_account_activity_v2_covered_hours(tx, full_start_epoch, full_end_epoch).await?;
    let all_full_hours = (full_start_epoch..full_end_epoch)
        .step_by(3_600)
        .collect::<Vec<_>>();
    let covered_hours = all_full_hours
        .iter()
        .copied()
        .filter(|hour| covered.contains(hour))
        .collect::<Vec<_>>();
    let uncovered_hours = all_full_hours
        .iter()
        .copied()
        .filter(|hour| !covered.contains(hour))
        .collect::<Vec<_>>();

    let full_start = Utc
        .timestamp_opt(full_start_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid account activity full-hour start")))?;
    let full_end = Utc
        .timestamp_opt(full_end_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid account activity full-hour end")))?;
    let mut exact_ranges = Vec::new();
    if full_start_epoch >= full_end_epoch {
        push_exact_range(&mut exact_ranges, live_start, range.end)?;
    } else {
        push_exact_range(&mut exact_ranges, live_start, range.end.min(full_start))?;
        push_exact_range(&mut exact_ranges, live_start.max(full_end), range.end)?;
    }
    let boundary_tail_count = exact_ranges.len();

    let mut index = 0;
    while index < uncovered_hours.len() {
        let start_epoch = uncovered_hours[index];
        let mut end_epoch = start_epoch + 3_600;
        index += 1;
        while index < uncovered_hours.len() && uncovered_hours[index] == end_epoch {
            end_epoch += 3_600;
            index += 1;
        }
        let start = Utc
            .timestamp_opt(start_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid account activity fallback start")))?;
        let end = Utc
            .timestamp_opt(end_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid account activity fallback end")))?;
        push_exact_range(&mut exact_ranges, start, end)?;
    }

    Ok(AccountActivityRangePlan {
        covered_hours,
        uncovered_hours,
        exact_ranges,
        boundary_tail_count,
    })
}

pub(crate) async fn load_upstream_account_activity_range_rows(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    route: &'static str,
) -> Result<AccountActivityRangeBuild, ApiError> {
    let started_at = Instant::now();
    let mut tx = state.pool.begin().await?;
    let plan = plan_account_activity_range_tx(state, tx.as_mut(), range).await?;
    if plan.covered_hours.is_empty()
        && plan.uncovered_hours.is_empty()
        && plan.exact_ranges.is_empty()
    {
        return Ok(AccountActivityRangeBuild {
            rows: Vec::new(),
            telemetry: AccountActivityRangeBuildTelemetry::default(),
        });
    }

    let (mut merged, rollup_row_count) =
        load_account_activity_range_rollups(tx.as_mut(), source_scope, &plan).await?;
    let raw_fallback_range_count = plan.exact_ranges.len();
    merge_account_activity_range_covered_tail(tx.as_mut(), source_scope, &plan, route, &mut merged)
        .await?;
    merge_account_activity_range_exact_fallback(
        tx.as_mut(),
        source_scope,
        &plan,
        route,
        &mut merged,
    )
    .await?;

    let telemetry = AccountActivityRangeBuildTelemetry {
        covered_hour_count: plan.covered_hours.len(),
        fallback_hour_count: plan.uncovered_hours.len(),
        boundary_tail_count: plan.boundary_tail_count,
        rollup_row_count,
        raw_fallback_range_count,
    };
    tx.commit().await?;
    tracing::info!(
        route,
        builder = "account_activity_hourly_v2",
        aggregation_mode = if plan.uncovered_hours.is_empty() {
            "rollup_plus_boundary"
        } else {
            "partial_rollup_with_exact_fallback"
        },
        covered_hour_count = telemetry.covered_hour_count,
        fallback_hour_count = telemetry.fallback_hour_count,
        boundary_tail_count = telemetry.boundary_tail_count,
        rollup_row_count = telemetry.rollup_row_count,
        raw_fallback_range_count = telemetry.raw_fallback_range_count,
        fallback_reason = if plan.uncovered_hours.is_empty() {
            "none"
        } else {
            "v2_coverage_hole"
        },
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        "built upstream-account activity range"
    );
    Ok(AccountActivityRangeBuild {
        rows: merged.into_values().collect(),
        telemetry,
    })
}

async fn load_account_activity_range_rollups(
    tx: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    plan: &AccountActivityRangePlan,
) -> Result<
    (
        HashMap<Option<i64>, UpstreamAccountActivityAggregateRow>,
        usize,
    ),
    ApiError,
> {
    let rollup_rows =
        query_account_activity_v2_rollup_rows(tx, source_scope, &plan.covered_hours).await?;
    let rollup_row_count = rollup_rows.len();
    let mut merged = HashMap::<Option<i64>, UpstreamAccountActivityAggregateRow>::new();
    for row in rollup_rows {
        merge_account_activity_range_row(&mut merged, row);
    }
    for conversation in
        query_account_activity_conversation_rollup_rows(tx, source_scope, &plan.covered_hours)
            .await?
    {
        if let Some(entry) = merged.get_mut(&conversation.upstream_account_id) {
            merge_latest_optional_timestamp(
                &mut entry.latest_conversation_created_at,
                conversation.latest_conversation_created_at,
            );
        }
    }
    Ok((merged, rollup_row_count))
}

async fn merge_account_activity_range_covered_tail(
    tx: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    plan: &AccountActivityRangePlan,
    route: &'static str,
    merged: &mut HashMap<Option<i64>, UpstreamAccountActivityAggregateRow>,
) -> Result<(), ApiError> {
    let repair_cursor = load_hourly_rollup_live_progress_tx(
        tx,
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
    )
    .await?;
    let bucket_watermarks =
        load_account_activity_v2_bucket_repair_watermarks(&mut *tx, &plan.covered_hours).await?;
    let covered_tail_min_ids = plan
        .covered_hours
        .iter()
        .map(|bucket_epoch| {
            (
                *bucket_epoch,
                repair_cursor.max(
                    bucket_watermarks
                        .get(bucket_epoch)
                        .copied()
                        .unwrap_or_default(),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let (Some(first_bucket), Some(last_bucket)) =
        (plan.covered_hours.first(), plan.covered_hours.last())
    else {
        return Ok(());
    };
    let covered_start = Utc
        .timestamp_opt(*first_bucket, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid covered live tail start")))?;
    let covered_end = Utc
        .timestamp_opt(*last_bucket + 3_600, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid covered live tail end")))?;
    for row in query_live_upstream_account_activity_aggregate_rows_with_telemetry(
        tx,
        UpstreamAccountActivityAggregateQueryInput {
            source_scope,
            range: ExactUtcRange {
                start: covered_start,
                end: covered_end,
            },
            use_attempt_fallback: true,
            exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter::None,
            min_invocation_id_exclusive: None,
            bucket_min_invocation_ids_exclusive: Some(&covered_tail_min_ids),
            route,
            purpose: "covered_live_tail",
        },
    )
    .await?
    {
        merge_account_activity_range_row(merged, row);
    }
    Ok(())
}

async fn merge_account_activity_range_exact_fallback(
    tx: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    plan: &AccountActivityRangePlan,
    route: &'static str,
    merged: &mut HashMap<Option<i64>, UpstreamAccountActivityAggregateRow>,
) -> Result<(), ApiError> {
    for (index, exact_range) in plan.exact_ranges.iter().copied().enumerate() {
        let purpose = if index < plan.boundary_tail_count {
            "boundary_tail"
        } else {
            "coverage_hole"
        };
        for row in query_live_upstream_account_activity_aggregate_rows_with_telemetry(
            &mut *tx,
            UpstreamAccountActivityAggregateQueryInput {
                source_scope,
                range: exact_range,
                use_attempt_fallback: true,
                exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter::None,
                min_invocation_id_exclusive: None,
                bucket_min_invocation_ids_exclusive: None,
                route,
                purpose,
            },
        )
        .await?
        {
            merge_account_activity_range_row(merged, row);
        }
    }
    Ok(())
}

fn merge_account_activity_range_row(
    merged: &mut HashMap<Option<i64>, UpstreamAccountActivityAggregateRow>,
    row: UpstreamAccountActivityAggregateRow,
) {
    match merged.entry(row.upstream_account_id) {
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(row);
        }
        std::collections::hash_map::Entry::Occupied(mut entry) => {
            merge_account_activity_rows(entry.get_mut(), row);
        }
    }
}

async fn query_live_upstream_account_prompt_cache_created_at_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    use_attempt_fallback: bool,
    exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter<'_>,
) -> Result<Vec<UpstreamAccountPromptCacheCreatedAtRow>, ApiError> {
    let upstream_account_id_sql = if use_attempt_fallback {
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations")
    } else {
        INVOCATION_UPSTREAM_ACCOUNT_ID_SQL.to_string()
    };
    let prompt_cache_key_sql = INVOCATION_PROMPT_CACHE_KEY_SQL;
    let normalized_status_sql = INVOCATION_STATUS_NORMALIZED_SQL;
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    query
        .push(upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, ")
        .push(prompt_cache_key_sql)
        .push(" AS prompt_cache_key, MIN(occurred_at) AS first_occurred_at FROM codex_invocations WHERE occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end))
        .push(" AND ")
        .push(normalized_status_sql)
        .push(" NOT IN ('running', 'pending') AND ")
        .push(prompt_cache_key_sql)
        .push(" IS NOT NULL AND ")
        .push(prompt_cache_key_sql)
        .push(" <> ''");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    push_excluded_invocation_ids_filter(&mut query, exclude_invocation_ids);
    query.push(" GROUP BY upstream_account_id, prompt_cache_key");
    query
        .build_query_as::<UpstreamAccountPromptCacheCreatedAtRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

struct UpstreamAccountUsageBreakdownQueryInput<'a, E> {
    executor: E,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    has_cost_breakdown_columns: bool,
    use_attempt_fallback: bool,
    exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter<'a>,
    start_after_id: Option<i64>,
    snapshot_id: Option<i64>,
    upstream_account_id_filter: Option<i64>,
}

struct UpstreamAccountUsageBreakdownSql {
    upstream_account_id: String,
    model: String,
    reasoning_effort: String,
    cost_complete: String,
    cost_input: String,
    cost_cache_write: String,
    cost_cache_read: String,
    cost_output: String,
    cost_reasoning: String,
    success_like: String,
    success_billed: String,
    failure_count: String,
    final_stream_timing: String,
    final_stream_measured: String,
    ttfb_positive: String,
    final_first_token_timing: String,
    final_first_token_measured: String,
    total_nonnegative: String,
    first_response_byte_components_valid: String,
    first_response_byte_total: String,
}

fn build_upstream_account_usage_breakdown_sql(
    has_cost_breakdown_columns: bool,
    use_attempt_fallback: bool,
) -> UpstreamAccountUsageBreakdownSql {
    let upstream_account_id = if use_attempt_fallback {
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations")
    } else {
        INVOCATION_UPSTREAM_ACCOUNT_ID_SQL.to_string()
    };
    let model = format!(
        "COALESCE(NULLIF(TRIM({}), ''), NULLIF(TRIM(model), ''), 'unknown')",
        INVOCATION_RESPONSE_MODEL_SQL
    );
    let reasoning_effort = format!("NULLIF(TRIM({}), '')", INVOCATION_REASONING_EFFORT_SQL);
    let cost_complete = if has_cost_breakdown_columns {
        "cost_input IS NOT NULL AND cost_cache_write IS NOT NULL AND cost_cache_read IS NOT NULL AND cost_output IS NOT NULL AND cost_reasoning IS NOT NULL".to_string()
    } else {
        "0".to_string()
    };
    let cost_sql = |column: &str| {
        if has_cost_breakdown_columns {
            column.to_string()
        } else {
            "0".to_string()
        }
    };
    let failure_class = INVOCATION_RESOLVED_FAILURE_CLASS_SQL;
    let success_like = format!(
        "LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed', '{warning_success}') AND ({failure_class}) = 'none'",
        warning_success = INVOCATION_STATUS_WARNING_SUCCESS,
    );
    let success_billed = format!("{success_like} AND cost IS NOT NULL");
    let failure_count = format!(
        "LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending') AND ({failure_class}) IN ('service_failure', 'client_failure', 'client_abort')"
    );
    let final_stream_timing = invocation_timing_sql_for_source(
        "codex_invocations",
        "t_upstream_stream_ms",
        use_attempt_fallback,
    );
    let final_stream_measured = format!("({final_stream_timing}) IS NOT NULL");
    let ttfb_positive = sqlite_positive_timing_sql("t_upstream_ttfb_ms");
    let final_first_token_timing = invocation_timing_sql_for_source(
        "codex_invocations",
        "first_token_ms",
        use_attempt_fallback,
    );
    let final_first_token_measured = format!("({final_first_token_timing}) IS NOT NULL");
    let total_nonnegative = sqlite_nonnegative_timing_sql("t_total_ms");
    let first_response_byte_components_valid = format!(
        "(t_req_read_ms IS NULL OR {}) AND (t_req_parse_ms IS NULL OR {}) AND (t_upstream_connect_ms IS NULL OR {}) AND (t_upstream_ttfb_ms IS NULL OR {})",
        sqlite_nonnegative_timing_sql("t_req_read_ms"),
        sqlite_nonnegative_timing_sql("t_req_parse_ms"),
        sqlite_nonnegative_timing_sql("t_upstream_connect_ms"),
        sqlite_nonnegative_timing_sql("t_upstream_ttfb_ms"),
    );
    let first_response_byte_total = format!(
        "CASE WHEN {first_response_byte_components_valid} THEN COALESCE(t_req_read_ms, 0) + COALESCE(t_req_parse_ms, 0) + COALESCE(t_upstream_connect_ms, 0) + COALESCE(t_upstream_ttfb_ms, 0) END"
    );
    UpstreamAccountUsageBreakdownSql {
        upstream_account_id,
        model,
        reasoning_effort,
        cost_complete,
        cost_input: cost_sql("cost_input"),
        cost_cache_write: cost_sql("cost_cache_write"),
        cost_cache_read: cost_sql("cost_cache_read"),
        cost_output: cost_sql("cost_output"),
        cost_reasoning: cost_sql("cost_reasoning"),
        success_like,
        success_billed,
        failure_count,
        final_stream_timing,
        final_stream_measured,
        ttfb_positive,
        final_first_token_timing,
        final_first_token_measured,
        total_nonnegative,
        first_response_byte_components_valid,
        first_response_byte_total,
    }
}

async fn query_upstream_account_usage_breakdown_rows_from_executor<'e, E>(
    input: UpstreamAccountUsageBreakdownQueryInput<'_, E>,
) -> Result<Vec<UpstreamAccountUsageBreakdownAggregateRow>, ApiError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let UpstreamAccountUsageBreakdownQueryInput {
        executor,
        source_scope,
        range,
        has_cost_breakdown_columns,
        use_attempt_fallback,
        exclude_invocation_ids,
        start_after_id,
        snapshot_id,
        upstream_account_id_filter,
    } = input;
    let sql = build_upstream_account_usage_breakdown_sql(
        has_cost_breakdown_columns,
        use_attempt_fallback,
    );
    let mut query = build_upstream_account_usage_breakdown_query(&sql);
    query
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if let Some(start_after_id) = start_after_id {
        query.push(" AND id > ").push_bind(start_after_id);
    }
    if let Some(snapshot_id) = snapshot_id {
        query.push(" AND id <= ").push_bind(snapshot_id);
    }
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id_filter {
        query
            .push(" AND ")
            .push(sql.upstream_account_id.as_str())
            .push(" = ")
            .push_bind(upstream_account_id);
    }
    push_excluded_invocation_ids_filter(&mut query, exclude_invocation_ids);
    query.push(" AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')");
    query.push(" GROUP BY upstream_account_id, model, reasoning_effort");
    query
        .build_query_as::<UpstreamAccountUsageBreakdownAggregateRow>()
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

fn build_upstream_account_usage_breakdown_query(
    sql: &UpstreamAccountUsageBreakdownSql,
) -> QueryBuilder<'static, Sqlite> {
    QueryBuilder::<Sqlite>::new(format!(
        r#"
        SELECT
            {upstream_account_id} AS upstream_account_id,
            {model} AS model,
            {reasoning_effort} AS reasoning_effort,
            COUNT(*) AS request_count,
            COALESCE(SUM(CASE WHEN {success_like} THEN 1 ELSE 0 END), 0) AS success_count,
            COALESCE(SUM(CASE WHEN {failure_count} THEN 1 ELSE 0 END), 0) AS failure_count,
            COALESCE(SUM(MAX(COALESCE(input_tokens, 0) - COALESCE(cache_input_tokens, 0), 0)), 0) AS cache_write_tokens,
            COALESCE(SUM(COALESCE(cache_input_tokens, 0)), 0) AS cache_read_tokens,
            COALESCE(SUM(COALESCE(output_tokens, 0)), 0) AS output_tokens,
            CAST(COALESCE(SUM(CASE WHEN cost IS NOT NULL AND {cost_complete} THEN {cost_input} ELSE 0 END), 0) AS REAL) AS cost_input,
            CAST(COALESCE(SUM(CASE WHEN cost IS NOT NULL AND {cost_complete} THEN {cost_cache_write} ELSE 0 END), 0) AS REAL) AS cost_cache_write,
            CAST(COALESCE(SUM(CASE WHEN cost IS NOT NULL AND {cost_complete} THEN {cost_cache_read} ELSE 0 END), 0) AS REAL) AS cost_cache_read,
            CAST(COALESCE(SUM(CASE WHEN cost IS NOT NULL AND {cost_complete} THEN {cost_output} ELSE 0 END), 0) AS REAL) AS cost_output,
            CAST(COALESCE(SUM(CASE WHEN cost IS NOT NULL AND {cost_complete} THEN {cost_reasoning} ELSE 0 END), 0) AS REAL) AS cost_reasoning,
            CAST(COALESCE(SUM(CASE WHEN cost IS NOT NULL AND NOT ({cost_complete}) THEN cost ELSE 0 END), 0) AS REAL) AS cost_unknown,
            SUM(CASE WHEN cost IS NOT NULL THEN 1 ELSE 0 END) AS has_cost,
            COALESCE(SUM(CASE WHEN {success_billed} THEN COALESCE(total_tokens, 0) ELSE 0 END), 0) AS performance_total_tokens,
            COALESCE(SUM(CASE WHEN {success_billed} AND {final_stream_measured} THEN COALESCE(output_tokens, 0) ELSE 0 END), 0) AS performance_stream_output_tokens,
            CAST(COALESCE(SUM(CASE WHEN {success_billed} AND {final_stream_measured} THEN {final_stream_timing} ELSE 0 END), 0) AS REAL) AS performance_stream_duration_ms,
            SUM(CASE WHEN {success_like} AND {final_stream_measured} THEN 1 ELSE 0 END) AS performance_response_sample_count,
            CAST(COALESCE(SUM(CASE WHEN {success_like} AND {final_stream_measured} THEN {final_stream_timing} ELSE 0 END), 0) AS REAL) AS performance_response_sum_ms,
            SUM(CASE WHEN {success_billed} AND {ttfb_positive} AND {first_response_byte_components_valid} THEN 1 ELSE 0 END) AS performance_first_byte_sample_count,
            CAST(COALESCE(SUM(CASE WHEN {success_billed} AND {ttfb_positive} AND {first_response_byte_components_valid} THEN {first_response_byte_total} ELSE 0 END), 0) AS REAL) AS performance_first_byte_sum_ms,
            SUM(CASE WHEN {final_first_token_measured} THEN 1 ELSE 0 END) AS performance_first_token_sample_count,
            CAST(COALESCE(SUM(CASE WHEN {final_first_token_measured} THEN {final_first_token_timing} ELSE 0 END), 0) AS REAL) AS performance_first_token_sum_ms,
            SUM(CASE WHEN {success_billed} AND {total_nonnegative} THEN 1 ELSE 0 END) AS performance_usage_duration_sample_count,
            CAST(COALESCE(SUM(CASE WHEN {success_billed} AND {total_nonnegative} THEN t_total_ms ELSE 0 END), 0) AS REAL) AS performance_usage_duration_sum_ms
        FROM codex_invocations
        WHERE occurred_at >=
        "#,
        upstream_account_id = sql.upstream_account_id.as_str(),
        model = sql.model.as_str(),
        reasoning_effort = sql.reasoning_effort.as_str(),
        success_like = sql.success_like.as_str(),
        failure_count = sql.failure_count.as_str(),
        cost_complete = sql.cost_complete.as_str(),
        cost_input = sql.cost_input.as_str(),
        cost_cache_write = sql.cost_cache_write.as_str(),
        cost_cache_read = sql.cost_cache_read.as_str(),
        cost_output = sql.cost_output.as_str(),
        cost_reasoning = sql.cost_reasoning.as_str(),
        success_billed = sql.success_billed.as_str(),
        final_stream_measured = sql.final_stream_measured.as_str(),
        final_stream_timing = sql.final_stream_timing.as_str(),
        ttfb_positive = sql.ttfb_positive.as_str(),
        first_response_byte_components_valid = sql.first_response_byte_components_valid.as_str(),
        first_response_byte_total = sql.first_response_byte_total.as_str(),
        final_first_token_measured = sql.final_first_token_measured.as_str(),
        final_first_token_timing = sql.final_first_token_timing.as_str(),
        total_nonnegative = sql.total_nonnegative.as_str(),
    ))
}

async fn query_live_upstream_account_usage_breakdown_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    has_cost_breakdown_columns: bool,
    use_attempt_fallback: bool,
    exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter<'_>,
) -> Result<Vec<UpstreamAccountUsageBreakdownAggregateRow>, ApiError> {
    let started_at = Instant::now();
    let rows = query_upstream_account_usage_breakdown_rows_from_executor(
        UpstreamAccountUsageBreakdownQueryInput {
            executor: pool,
            source_scope,
            range,
            has_cost_breakdown_columns,
            use_attempt_fallback,
            exclude_invocation_ids,
            start_after_id: None,
            snapshot_id: None,
            upstream_account_id_filter: None,
        },
    )
    .await?;
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        tracing::warn!(
            endpoint = "/api/stats/upstream-account-activity",
            operation = "live_usage_breakdown",
            ?source_scope,
            start = %range.start,
            end = %range.end,
            row_count = rows.len(),
            elapsed_ms,
            "slow upstream-account usage breakdown"
        );
    }
    Ok(rows)
}

async fn query_live_upstream_account_usage_breakdown_rows_tx(
    input: UpstreamAccountUsageBreakdownQueryInput<'_, &mut SqliteConnection>,
) -> Result<Vec<UpstreamAccountUsageBreakdownAggregateRow>, ApiError> {
    query_upstream_account_usage_breakdown_rows_from_executor(input).await
}
