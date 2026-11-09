use super::*;
#[derive(Debug, FromRow)]
pub(crate) struct RuntimeRecentAccountFallbackRow {
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) upstream_account_plan_type: Option<String>,
}

pub(crate) const DASHBOARD_ACTIVITY_EXCLUDED_IDS_INLINE_LIMIT: usize = 500;
pub(crate) const DASHBOARD_ACTIVITY_EXCLUDED_IDS_TABLE: &str =
    "dashboard_activity_excluded_invocation_ids";
pub(crate) const DASHBOARD_ACTIVITY_PREVIEW_ID_HYDRATION_CHUNK_SIZE: usize = 400;

#[derive(Clone, Copy)]
pub(crate) enum DashboardActivityExcludedInvocationIdsFilter<'a> {
    None,
    Inline(&'a HashSet<i64>),
    Table,
}

pub(crate) async fn prepare_dashboard_activity_excluded_invocation_ids_filter<'a>(
    pool: &Pool<Sqlite>,
    exclude_invocation_ids: Option<&'a HashSet<i64>>,
) -> Result<DashboardActivityExcludedInvocationIdsFilter<'a>, ApiError> {
    let Some(exclude_invocation_ids) =
        exclude_invocation_ids.filter(|exclude_invocation_ids| !exclude_invocation_ids.is_empty())
    else {
        return Ok(DashboardActivityExcludedInvocationIdsFilter::None);
    };
    if exclude_invocation_ids.len() <= DASHBOARD_ACTIVITY_EXCLUDED_IDS_INLINE_LIMIT {
        return Ok(DashboardActivityExcludedInvocationIdsFilter::Inline(
            exclude_invocation_ids,
        ));
    }

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS dashboard_activity_excluded_invocation_ids (\
             id INTEGER PRIMARY KEY\
         )",
    )
    .execute(pool)
    .await?;
    sqlx::query("DELETE FROM dashboard_activity_excluded_invocation_ids")
        .execute(pool)
        .await?;

    let ids = exclude_invocation_ids.iter().copied().collect::<Vec<_>>();
    for chunk in ids.chunks(DASHBOARD_ACTIVITY_EXCLUDED_IDS_INLINE_LIMIT) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT OR IGNORE INTO dashboard_activity_excluded_invocation_ids (id) ",
        );
        query.push_values(chunk.iter().copied(), |mut row, id| {
            row.push_bind(id);
        });
        query.build().execute(pool).await?;
    }

    Ok(DashboardActivityExcludedInvocationIdsFilter::Table)
}

pub(crate) fn push_excluded_invocation_ids_filter(
    query: &mut QueryBuilder<Sqlite>,
    exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter<'_>,
) {
    match exclude_invocation_ids {
        DashboardActivityExcludedInvocationIdsFilter::None => {}
        DashboardActivityExcludedInvocationIdsFilter::Inline(exclude_invocation_ids) => {
            query.push(" AND id NOT IN (");
            {
                let mut separated = query.separated(", ");
                for id in exclude_invocation_ids {
                    separated.push_bind(*id);
                }
            }
            query.push(")");
        }
        DashboardActivityExcludedInvocationIdsFilter::Table => {
            query.push(" AND id NOT IN (SELECT id FROM ");
            query.push(DASHBOARD_ACTIVITY_EXCLUDED_IDS_TABLE);
            query.push(")");
        }
    }
}

pub(crate) async fn query_live_upstream_account_activity_aggregate_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    use_attempt_fallback: bool,
    exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter<'_>,
) -> Result<Vec<UpstreamAccountActivityAggregateRow>, ApiError> {
    query_live_upstream_account_activity_aggregate_rows_with_telemetry(
        pool,
        source_scope,
        range,
        use_attempt_fallback,
        exclude_invocation_ids,
        None,
        None,
        "internal",
        "legacy_exact",
    )
    .await
}

pub(crate) async fn query_live_upstream_account_activity_aggregate_rows_with_telemetry<'e, E>(
    executor: E,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    use_attempt_fallback: bool,
    exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter<'_>,
    min_invocation_id_exclusive: Option<i64>,
    bucket_min_invocation_ids_exclusive: Option<&BTreeMap<i64, i64>>,
    route: &'static str,
    purpose: &'static str,
) -> Result<Vec<UpstreamAccountActivityAggregateRow>, ApiError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let started_at = Instant::now();
    let upstream_account_id_sql = if use_attempt_fallback {
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations")
    } else {
        INVOCATION_UPSTREAM_ACCOUNT_ID_SQL.to_string()
    };
    let failure_class_sql = INVOCATION_RESOLVED_FAILURE_CLASS_SQL;
    let success_sql = format!(
        "LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'succeeded', 'completed', 'ok', '{warning_success}') AND ({failure_class_sql}) = 'none'",
        warning_success = INVOCATION_STATUS_WARNING_SUCCESS,
    );
    let failure_sql = format!(
        "LOWER(TRIM(COALESCE(status, ''))) NOT IN ('', 'running', 'pending') AND ({failure_class_sql}) <> 'none'"
    );
    let non_success_sql = format!(
        "(LOWER(TRIM(COALESCE(status, ''))) = 'interrupted' OR \
         (LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending') \
          AND ({failure_class_sql}) <> 'none'))"
    );
    let first_response_byte_components_valid_sql = format!(
        "(t_req_read_ms IS NULL OR {}) AND (t_req_parse_ms IS NULL OR {}) AND (t_upstream_connect_ms IS NULL OR {}) AND (t_upstream_ttfb_ms IS NULL OR {})",
        sqlite_nonnegative_timing_sql("t_req_read_ms"),
        sqlite_nonnegative_timing_sql("t_req_parse_ms"),
        sqlite_nonnegative_timing_sql("t_upstream_connect_ms"),
        sqlite_nonnegative_timing_sql("t_upstream_ttfb_ms"),
    );
    let first_response_byte_total_sql = format!(
        "CASE WHEN {first_response_byte_components_valid_sql} THEN COALESCE(t_req_read_ms, 0) + COALESCE(t_req_parse_ms, 0) + COALESCE(t_upstream_connect_ms, 0) + COALESCE(t_upstream_ttfb_ms, 0) END"
    );
    let ttfb_positive_sql = sqlite_positive_timing_sql("t_upstream_ttfb_ms");
    let final_first_token_sql = invocation_timing_sql_for_source(
        "filtered_invocations",
        "first_token_ms",
        use_attempt_fallback,
    );
    let first_token_nonnegative_sql =
        sqlite_nonnegative_timing_sql(&format!("({final_first_token_sql})"));
    let total_nonnegative_sql = sqlite_nonnegative_timing_sql("t_total_ms");
    let prompt_cache_key_sql = INVOCATION_PROMPT_CACHE_KEY_SQL;
    let valid_cache_input_tokens_sql = INVOCATION_VALID_CACHE_INPUT_TOKENS_SQL;
    let filtered_prompt_cache_key_sql = "filtered_invocations.prompt_cache_key";
    let preaggregated_conversation_created_at_sql = if use_attempt_fallback {
        prompt_cache_conversation_created_at_sql(filtered_prompt_cache_key_sql, source_scope)
    } else {
        invocation_history_conversation_created_at_sql(filtered_prompt_cache_key_sql, source_scope)
    };
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        r#"
        WITH filtered_invocations AS (
            SELECT
                id,
                invoke_id,
                occurred_at,
                status,
                total_tokens,
                input_tokens,
                cost,
                cache_input_tokens,
                payload,
                error_message,
                failure_kind,
                failure_class,
                is_actionable,
                t_req_read_ms,
                t_req_parse_ms,
                t_upstream_connect_ms,
                t_upstream_ttfb_ms,
                first_token_ms,
                t_total_ms,
                {upstream_account_id_sql} AS upstream_account_id,
                {prompt_cache_key_sql} AS prompt_cache_key
            FROM codex_invocations
            WHERE occurred_at >=
        "#,
    ));
    query
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(min_invocation_id_exclusive) = min_invocation_id_exclusive {
        query
            .push(" AND id > ")
            .push_bind(min_invocation_id_exclusive);
    }
    if let Some(bucket_min_ids) = bucket_min_invocation_ids_exclusive {
        query.push(" AND (");
        for (index, (bucket_epoch, min_id)) in bucket_min_ids.iter().enumerate() {
            let start = Utc
                .timestamp_opt(*bucket_epoch, 0)
                .single()
                .ok_or_else(|| ApiError::from(anyhow!("invalid covered-tail bucket epoch")))?;
            let end = start + ChronoDuration::hours(1);
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(id > ")
                .push_bind(*min_id)
                .push(" AND occurred_at >= ")
                .push_bind(db_occurred_at_lower_bound(start))
                .push(" AND occurred_at < ")
                .push_bind(db_occurred_at_upper_bound(end))
                .push(")");
        }
        query.push(")");
    }
    push_excluded_invocation_ids_filter(&mut query, exclude_invocation_ids);
    query.push(" AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')");
    query.push(format!(
        r#"
        ),
        conversation_created_at_by_key AS (
            SELECT
                filtered_invocations.prompt_cache_key AS prompt_cache_key,
                COALESCE(
                    {preaggregated_conversation_created_at_sql},
                    MIN(filtered_invocations.occurred_at)
                ) AS conversation_created_at
            FROM filtered_invocations
            WHERE filtered_invocations.prompt_cache_key IS NOT NULL
              AND filtered_invocations.prompt_cache_key <> ''
            GROUP BY filtered_invocations.prompt_cache_key
        ),
        latest_first_response_byte_total_by_account AS (
            SELECT
                ranked.upstream_account_id AS upstream_account_id,
                ranked.occurred_at AS latest_first_response_byte_total_at,
                ranked.first_response_byte_total_ms AS latest_first_response_byte_total_ms
            FROM (
                SELECT
                    filtered_invocations.upstream_account_id AS upstream_account_id,
                    filtered_invocations.occurred_at AS occurred_at,
                    {first_response_byte_total_sql} AS first_response_byte_total_ms,
                    ROW_NUMBER() OVER (
                        PARTITION BY filtered_invocations.upstream_account_id
                        ORDER BY filtered_invocations.occurred_at DESC, filtered_invocations.id DESC
                    ) AS row_num
                FROM filtered_invocations
                WHERE {success_sql}
                  AND {ttfb_positive_sql}
                  AND {first_response_byte_components_valid_sql}
            ) AS ranked
            WHERE ranked.row_num = 1
        ),
        latest_total_latency_by_account AS (
            SELECT
                ranked.upstream_account_id AS upstream_account_id,
                ranked.occurred_at AS latest_avg_total_at,
                ranked.t_total_ms AS latest_avg_total_ms
            FROM (
                SELECT
                    filtered_invocations.upstream_account_id AS upstream_account_id,
                    filtered_invocations.occurred_at AS occurred_at,
                    filtered_invocations.t_total_ms AS t_total_ms,
                    ROW_NUMBER() OVER (
                        PARTITION BY filtered_invocations.upstream_account_id
                        ORDER BY filtered_invocations.occurred_at DESC, filtered_invocations.id DESC
                    ) AS row_num
                FROM filtered_invocations
                WHERE {success_sql}
                  AND {total_nonnegative_sql}
            ) AS ranked
            WHERE ranked.row_num = 1
        )
        SELECT
            filtered_invocations.upstream_account_id AS upstream_account_id,
            MAX(COALESCE(conversation_created_at_by_key.conversation_created_at, filtered_invocations.occurred_at)) AS latest_conversation_created_at,
            MAX(filtered_invocations.occurred_at) AS last_invocation_at,
            COUNT(*) AS request_count,
            SUM(CASE WHEN {success_sql} THEN 1 ELSE 0 END) AS success_count,
            SUM(CASE WHEN {failure_sql} THEN 1 ELSE 0 END) AS failure_count,
            SUM(CASE WHEN {non_success_sql} THEN 1 ELSE 0 END) AS non_success_count,
            COALESCE(SUM(COALESCE(total_tokens, 0)), 0) AS total_tokens,
            COALESCE(SUM(CASE WHEN {success_sql} THEN COALESCE(total_tokens, 0) ELSE 0 END), 0) AS success_tokens,
            COALESCE(SUM(CASE WHEN {non_success_sql} THEN COALESCE(total_tokens, 0) ELSE 0 END), 0) AS non_success_tokens,
            COALESCE(SUM(CASE WHEN {failure_sql} THEN COALESCE(total_tokens, 0) ELSE 0 END), 0) AS failure_tokens,
            CAST(COALESCE(SUM(CASE WHEN {failure_sql} THEN COALESCE(cost, 0) ELSE 0 END), 0) AS REAL) AS failure_cost,
            CAST(COALESCE(SUM(CASE WHEN {non_success_sql} THEN COALESCE(cost, 0) ELSE 0 END), 0) AS REAL) AS non_success_cost,
            COALESCE(SUM({valid_cache_input_tokens_sql}), 0) AS cache_input_tokens,
            CAST(COALESCE(SUM(COALESCE(cost, 0)), 0) AS REAL) AS total_cost,
            SUM(CASE WHEN {success_sql} AND {ttfb_positive_sql} AND {first_response_byte_components_valid_sql} THEN 1 ELSE 0 END) AS first_response_byte_total_sample_count,
            CAST(COALESCE(SUM(CASE WHEN {success_sql} AND {ttfb_positive_sql} AND {first_response_byte_components_valid_sql} THEN {first_response_byte_total_sql} ELSE 0 END), 0) AS REAL) AS first_response_byte_total_sum_ms,
            SUM(CASE WHEN {first_token_nonnegative_sql} THEN 1 ELSE 0 END) AS first_token_sample_count,
            CAST(COALESCE(SUM(CASE WHEN {first_token_nonnegative_sql} THEN {final_first_token_sql} ELSE 0 END), 0) AS REAL) AS first_token_sum_ms,
            SUM(CASE WHEN {success_sql} AND {total_nonnegative_sql} THEN 1 ELSE 0 END) AS total_latency_sample_count,
            CAST(COALESCE(SUM(CASE WHEN {success_sql} AND {total_nonnegative_sql} THEN t_total_ms ELSE 0 END), 0) AS REAL) AS total_latency_sum_ms,
            latest_first_response_byte_total_by_account.latest_first_response_byte_total_at AS latest_first_response_byte_total_at,
            latest_first_response_byte_total_by_account.latest_first_response_byte_total_ms AS latest_first_response_byte_total_ms,
            latest_total_latency_by_account.latest_avg_total_at AS latest_avg_total_at,
            latest_total_latency_by_account.latest_avg_total_ms AS latest_avg_total_ms
        FROM filtered_invocations
        LEFT JOIN conversation_created_at_by_key
          ON conversation_created_at_by_key.prompt_cache_key = filtered_invocations.prompt_cache_key
        LEFT JOIN latest_first_response_byte_total_by_account
          ON latest_first_response_byte_total_by_account.upstream_account_id IS filtered_invocations.upstream_account_id
        LEFT JOIN latest_total_latency_by_account
          ON latest_total_latency_by_account.upstream_account_id IS filtered_invocations.upstream_account_id
        "#,
    ));
    query.push(" GROUP BY filtered_invocations.upstream_account_id");

    let rows = query
        .build_query_as::<UpstreamAccountActivityAggregateRow>()
        .fetch_all(executor)
        .await?;
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        tracing::warn!(
            route,
            builder = "account_activity_exact",
            endpoint = "/api/stats/upstream-account-activity",
            operation = "live_account_aggregate",
            aggregation_mode = "exact_fallback",
            purpose,
            fallback_reason = match purpose {
                "coverage_hole" => "v2_coverage_hole",
                "boundary_tail" => "boundary_tail",
                _ => "legacy_exact",
            },
            ?source_scope,
            start = %range.start,
            end = %range.end,
            row_count = rows.len(),
            elapsed_ms,
            "slow upstream-account activity aggregate"
        );
    }
    Ok(rows)
}

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

pub(crate) fn merge_account_activity_rows(
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

pub(crate) async fn query_account_activity_v2_rollup_rows(
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
pub(crate) struct AccountActivityConversationRollupRow {
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) latest_conversation_created_at: Option<String>,
}

pub(crate) async fn query_account_activity_conversation_rollup_rows(
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

pub(crate) async fn load_account_activity_v2_covered_hours(
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

pub(crate) async fn load_account_activity_v2_bucket_repair_watermarks(
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

pub(crate) struct AccountActivityRangePlan {
    pub(crate) covered_hours: Vec<i64>,
    pub(crate) uncovered_hours: Vec<i64>,
    pub(crate) exact_ranges: Vec<ExactUtcRange>,
    pub(crate) boundary_tail_count: usize,
}

pub(crate) async fn plan_account_activity_range(
    state: &AppState,
    range: ExactUtcRange,
) -> Result<AccountActivityRangePlan, ApiError> {
    let mut tx = state.pool.begin().await?;
    let plan = plan_account_activity_range_tx(state, tx.as_mut(), range).await?;
    tx.commit().await?;
    Ok(plan)
}

pub(crate) async fn plan_account_activity_range_tx(
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

    let rollup_rows =
        query_account_activity_v2_rollup_rows(tx.as_mut(), source_scope, &plan.covered_hours)
            .await?;
    let rollup_row_count = rollup_rows.len();
    let raw_fallback_range_count = plan.exact_ranges.len();
    let mut merged = HashMap::<Option<i64>, UpstreamAccountActivityAggregateRow>::new();
    for row in rollup_rows {
        match merged.entry(row.upstream_account_id) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(row);
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                merge_account_activity_rows(entry.get_mut(), row);
            }
        }
    }
    for conversation in query_account_activity_conversation_rollup_rows(
        tx.as_mut(),
        source_scope,
        &plan.covered_hours,
    )
    .await?
    {
        if let Some(entry) = merged.get_mut(&conversation.upstream_account_id) {
            merge_latest_optional_timestamp(
                &mut entry.latest_conversation_created_at,
                conversation.latest_conversation_created_at,
            );
        }
    }
    let repair_cursor = load_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET,
    )
    .await?;
    let bucket_watermarks =
        load_account_activity_v2_bucket_repair_watermarks(tx.as_mut(), &plan.covered_hours).await?;
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
    if let (Some(first_bucket), Some(last_bucket)) =
        (plan.covered_hours.first(), plan.covered_hours.last())
    {
        let covered_start = Utc
            .timestamp_opt(*first_bucket, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid covered live tail start")))?;
        let covered_end = Utc
            .timestamp_opt(*last_bucket + 3_600, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid covered live tail end")))?;
        for row in query_live_upstream_account_activity_aggregate_rows_with_telemetry(
            tx.as_mut(),
            source_scope,
            ExactUtcRange {
                start: covered_start,
                end: covered_end,
            },
            true,
            DashboardActivityExcludedInvocationIdsFilter::None,
            None,
            Some(&covered_tail_min_ids),
            route,
            "covered_live_tail",
        )
        .await?
        {
            match merged.entry(row.upstream_account_id) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(row);
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    merge_account_activity_rows(entry.get_mut(), row);
                }
            }
        }
    }
    for (index, exact_range) in plan.exact_ranges.into_iter().enumerate() {
        let purpose = if index < plan.boundary_tail_count {
            "boundary_tail"
        } else {
            "coverage_hole"
        };
        for row in query_live_upstream_account_activity_aggregate_rows_with_telemetry(
            tx.as_mut(),
            source_scope,
            exact_range,
            true,
            DashboardActivityExcludedInvocationIdsFilter::None,
            None,
            None,
            route,
            purpose,
        )
        .await?
        {
            match merged.entry(row.upstream_account_id) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(row);
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    merge_account_activity_rows(entry.get_mut(), row);
                }
            }
        }
    }

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

pub(crate) async fn query_live_upstream_account_prompt_cache_created_at_rows(
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

pub(crate) async fn query_upstream_account_usage_breakdown_rows_from_executor<'e, E>(
    executor: E,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    has_cost_breakdown_columns: bool,
    use_attempt_fallback: bool,
    exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter<'_>,
    start_after_id: Option<i64>,
    snapshot_id: Option<i64>,
    upstream_account_id_filter: Option<i64>,
) -> Result<Vec<UpstreamAccountUsageBreakdownAggregateRow>, ApiError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let upstream_account_id_sql = if use_attempt_fallback {
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations")
    } else {
        INVOCATION_UPSTREAM_ACCOUNT_ID_SQL.to_string()
    };
    let model_sql = format!(
        "COALESCE(NULLIF(TRIM({}), ''), NULLIF(TRIM(model), ''), 'unknown')",
        INVOCATION_RESPONSE_MODEL_SQL
    );
    let reasoning_effort_sql = format!("NULLIF(TRIM({}), '')", INVOCATION_REASONING_EFFORT_SQL);
    let cost_complete_sql = if has_cost_breakdown_columns {
        "cost_input IS NOT NULL AND cost_cache_write IS NOT NULL AND cost_cache_read IS NOT NULL AND cost_output IS NOT NULL AND cost_reasoning IS NOT NULL"
    } else {
        "0"
    };
    let cost_input_sql = if has_cost_breakdown_columns {
        "cost_input"
    } else {
        "0"
    };
    let cost_cache_write_sql = if has_cost_breakdown_columns {
        "cost_cache_write"
    } else {
        "0"
    };
    let cost_cache_read_sql = if has_cost_breakdown_columns {
        "cost_cache_read"
    } else {
        "0"
    };
    let cost_output_sql = if has_cost_breakdown_columns {
        "cost_output"
    } else {
        "0"
    };
    let cost_reasoning_sql = if has_cost_breakdown_columns {
        "cost_reasoning"
    } else {
        "0"
    };
    let failure_class_sql = INVOCATION_RESOLVED_FAILURE_CLASS_SQL;
    let valid_cache_write_tokens_sql = INVOCATION_VALID_CACHE_WRITE_TOKENS_SQL;
    let valid_cache_input_tokens_sql = INVOCATION_VALID_CACHE_INPUT_TOKENS_SQL;
    let success_like_sql = format!(
        "LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'succeeded', 'completed', 'ok', '{warning_success}') AND ({failure_class_sql}) = 'none'",
        warning_success = INVOCATION_STATUS_WARNING_SUCCESS,
    );
    let success_billed_sql = format!("{success_like_sql} AND cost IS NOT NULL");
    let failure_count_sql = format!(
        "LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending') AND ({failure_class_sql}) IN ('service_failure', 'client_failure', 'client_abort')"
    );
    let final_stream_timing_sql = invocation_timing_sql_for_source(
        "codex_invocations",
        "t_upstream_stream_ms",
        use_attempt_fallback,
    );
    let final_stream_measured_sql = format!("({final_stream_timing_sql}) IS NOT NULL");
    let ttfb_positive_sql = sqlite_positive_timing_sql("t_upstream_ttfb_ms");
    let final_first_token_timing_sql = invocation_timing_sql_for_source(
        "codex_invocations",
        "first_token_ms",
        use_attempt_fallback,
    );
    let final_first_token_measured_sql = format!("({final_first_token_timing_sql}) IS NOT NULL");
    let total_nonnegative_sql = sqlite_nonnegative_timing_sql("t_total_ms");
    let first_response_byte_components_valid_sql = format!(
        "(t_req_read_ms IS NULL OR {}) AND (t_req_parse_ms IS NULL OR {}) AND (t_upstream_connect_ms IS NULL OR {}) AND (t_upstream_ttfb_ms IS NULL OR {})",
        sqlite_nonnegative_timing_sql("t_req_read_ms"),
        sqlite_nonnegative_timing_sql("t_req_parse_ms"),
        sqlite_nonnegative_timing_sql("t_upstream_connect_ms"),
        sqlite_nonnegative_timing_sql("t_upstream_ttfb_ms"),
    );
    let first_response_byte_total_sql = format!(
        "CASE WHEN {first_response_byte_components_valid_sql} THEN COALESCE(t_req_read_ms, 0) + COALESCE(t_req_parse_ms, 0) + COALESCE(t_upstream_connect_ms, 0) + COALESCE(t_upstream_ttfb_ms, 0) END"
    );
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        r#"
        SELECT
            {upstream_account_id_sql} AS upstream_account_id,
            {model_sql} AS model,
            {reasoning_effort_sql} AS reasoning_effort,
            COUNT(*) AS request_count,
            COALESCE(SUM(CASE WHEN {success_like_sql} THEN 1 ELSE 0 END), 0) AS success_count,
            COALESCE(SUM(CASE WHEN {failure_count_sql} THEN 1 ELSE 0 END), 0) AS failure_count,
            COALESCE(SUM({valid_cache_write_tokens_sql}), 0) AS cache_write_tokens,
            COALESCE(SUM({valid_cache_input_tokens_sql}), 0) AS cache_read_tokens,
            COALESCE(SUM(COALESCE(output_tokens, 0)), 0) AS output_tokens,
            CAST(COALESCE(SUM(CASE WHEN cost IS NOT NULL AND {cost_complete_sql} THEN {cost_input_sql} ELSE 0 END), 0) AS REAL) AS cost_input,
            CAST(COALESCE(SUM(CASE WHEN cost IS NOT NULL AND {cost_complete_sql} THEN {cost_cache_write_sql} ELSE 0 END), 0) AS REAL) AS cost_cache_write,
            CAST(COALESCE(SUM(CASE WHEN cost IS NOT NULL AND {cost_complete_sql} THEN {cost_cache_read_sql} ELSE 0 END), 0) AS REAL) AS cost_cache_read,
            CAST(COALESCE(SUM(CASE WHEN cost IS NOT NULL AND {cost_complete_sql} THEN {cost_output_sql} ELSE 0 END), 0) AS REAL) AS cost_output,
            CAST(COALESCE(SUM(CASE WHEN cost IS NOT NULL AND {cost_complete_sql} THEN {cost_reasoning_sql} ELSE 0 END), 0) AS REAL) AS cost_reasoning,
            CAST(COALESCE(SUM(CASE WHEN cost IS NOT NULL AND NOT ({cost_complete_sql}) THEN cost ELSE 0 END), 0) AS REAL) AS cost_unknown,
            SUM(CASE WHEN cost IS NOT NULL THEN 1 ELSE 0 END) AS has_cost,
            COALESCE(SUM(CASE WHEN {success_billed_sql} THEN COALESCE(total_tokens, 0) ELSE 0 END), 0) AS performance_total_tokens,
            COALESCE(SUM(CASE WHEN {success_billed_sql} AND {final_stream_measured_sql} THEN COALESCE(output_tokens, 0) ELSE 0 END), 0) AS performance_stream_output_tokens,
            CAST(COALESCE(SUM(CASE WHEN {success_billed_sql} AND {final_stream_measured_sql} THEN {final_stream_timing_sql} ELSE 0 END), 0) AS REAL) AS performance_stream_duration_ms,
            SUM(CASE WHEN {success_like_sql} AND {final_stream_measured_sql} THEN 1 ELSE 0 END) AS performance_response_sample_count,
            CAST(COALESCE(SUM(CASE WHEN {success_like_sql} AND {final_stream_measured_sql} THEN {final_stream_timing_sql} ELSE 0 END), 0) AS REAL) AS performance_response_sum_ms,
            SUM(CASE WHEN {success_billed_sql} AND {ttfb_positive_sql} AND {first_response_byte_components_valid_sql} THEN 1 ELSE 0 END) AS performance_first_byte_sample_count,
            CAST(COALESCE(SUM(CASE WHEN {success_billed_sql} AND {ttfb_positive_sql} AND {first_response_byte_components_valid_sql} THEN {first_response_byte_total_sql} ELSE 0 END), 0) AS REAL) AS performance_first_byte_sum_ms,
            SUM(CASE WHEN {final_first_token_measured_sql} THEN 1 ELSE 0 END) AS performance_first_token_sample_count,
            CAST(COALESCE(SUM(CASE WHEN {final_first_token_measured_sql} THEN {final_first_token_timing_sql} ELSE 0 END), 0) AS REAL) AS performance_first_token_sum_ms,
            SUM(CASE WHEN {success_billed_sql} AND {total_nonnegative_sql} THEN 1 ELSE 0 END) AS performance_usage_duration_sample_count,
            CAST(COALESCE(SUM(CASE WHEN {success_billed_sql} AND {total_nonnegative_sql} THEN t_total_ms ELSE 0 END), 0) AS REAL) AS performance_usage_duration_sum_ms
        FROM codex_invocations
        WHERE occurred_at >=
        "#,
    ));
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
            .push(upstream_account_id_sql.as_str())
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

pub(crate) async fn query_live_upstream_account_usage_breakdown_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    has_cost_breakdown_columns: bool,
    use_attempt_fallback: bool,
    exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter<'_>,
) -> Result<Vec<UpstreamAccountUsageBreakdownAggregateRow>, ApiError> {
    let started_at = Instant::now();
    let rows = query_upstream_account_usage_breakdown_rows_from_executor(
        pool,
        source_scope,
        range,
        has_cost_breakdown_columns,
        use_attempt_fallback,
        exclude_invocation_ids,
        None,
        None,
        None,
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

pub(crate) async fn query_live_upstream_account_usage_breakdown_rows_tx(
    tx: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    has_cost_breakdown_columns: bool,
    use_attempt_fallback: bool,
    exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter<'_>,
    start_after_id: Option<i64>,
    snapshot_id: Option<i64>,
    upstream_account_id_filter: Option<i64>,
) -> Result<Vec<UpstreamAccountUsageBreakdownAggregateRow>, ApiError> {
    query_upstream_account_usage_breakdown_rows_from_executor(
        &mut *tx,
        source_scope,
        range,
        has_cost_breakdown_columns,
        use_attempt_fallback,
        exclude_invocation_ids,
        start_after_id,
        snapshot_id,
        upstream_account_id_filter,
    )
    .await
}

pub(crate) async fn query_live_model_performance_duration_overrides(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    use_attempt_fallback: bool,
) -> Result<ModelPerformanceDurationOverrides, ApiError> {
    let started_at = Instant::now();
    let upstream_account_id_sql = if use_attempt_fallback {
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations")
    } else {
        INVOCATION_UPSTREAM_ACCOUNT_ID_SQL.to_string()
    };
    let model_sql = format!(
        "COALESCE(NULLIF(TRIM({}), ''), NULLIF(TRIM(model), ''), 'unknown')",
        INVOCATION_RESPONSE_MODEL_SQL
    );
    let reasoning_effort_sql = format!("NULLIF(TRIM({}), '')", INVOCATION_REASONING_EFFORT_SQL);
    let occurred_at_epoch_ms_sql = "(CAST(CASE WHEN instr(occurred_at, 'T') > 0 THEN strftime('%s', occurred_at) ELSE strftime('%s', occurred_at || '+08:00') END AS REAL) * 1000.0)";
    let failure_class_sql = INVOCATION_RESOLVED_FAILURE_CLASS_SQL;
    let success_billed_sql = format!(
        "LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'succeeded', 'completed', 'ok', '{warning_success}') AND ({failure_class_sql}) = 'none' AND cost IS NOT NULL",
        warning_success = INVOCATION_STATUS_WARNING_SUCCESS,
    );
    let range_end_epoch_ms = range.end.timestamp_millis() as f64;
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    query
        .push(upstream_account_id_sql.as_str())
        .push(" AS upstream_account_id, ")
        .push(model_sql.as_str())
        .push(" AS model, ")
        .push(reasoning_effort_sql.as_str())
        .push(" AS reasoning_effort, ")
        .push(occurred_at_epoch_ms_sql)
        .push(" AS start_epoch_ms, MIN(")
        .push(occurred_at_epoch_ms_sql)
        .push(" + t_total_ms, ")
        .push_bind(range_end_epoch_ms)
        .push(") AS end_epoch_ms FROM codex_invocations WHERE occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query
        .push(" AND ")
        .push(success_billed_sql.as_str())
        .push(" AND ")
        .push(sqlite_nonnegative_timing_sql("t_total_ms").as_str())
        .push(" ORDER BY occurred_at ASC, id ASC");
    let mut rows = query
        .build_query_as::<SuccessfulBilledUsageDurationIntervalRow>()
        .fetch(pool);
    let mut row_count = 0usize;
    let mut union_state = ModelPerformanceWallClockUnionState::default();
    while let Some(row) = rows.try_next().await? {
        row_count += 1;
        union_state.push_row(&row);
    }
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if elapsed_ms >= 1_000 {
        tracing::warn!(
            endpoint = "/api/stats/upstream-account-activity",
            operation = "live_model_performance_usage_duration",
            ?source_scope,
            start = %range.start,
            end = %range.end,
            row_count,
            elapsed_ms,
            "slow model performance usage duration query"
        );
    }
    Ok(union_state.into_overrides())
}

#[derive(Debug, Default)]
pub(crate) struct QueryCompletedInvocationArchiveActivityAggregateRows {
    pub(crate) aggregates: Vec<UpstreamAccountActivityAggregateRow>,
    pub(crate) skipped_materialized_ranges: Vec<ExactUtcRange>,
}

pub(crate) fn dashboard_activity_archive_row_overlap_range(
    archive_row: &crate::stats::ArchiveBatchPathRow,
    requested_range: ExactUtcRange,
) -> Option<ExactUtcRange> {
    let coverage_start =
        parse_to_utc_datetime(archive_row.coverage_start_at()?).unwrap_or(requested_range.start);
    let coverage_end =
        parse_to_utc_datetime(archive_row.coverage_end_at()?).unwrap_or(requested_range.end);
    let start = coverage_start.max(requested_range.start);
    let end = coverage_end.min(requested_range.end);
    (start < end).then_some(ExactUtcRange { start, end })
}

pub(crate) async fn query_completed_invocation_archive_activity_aggregate_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<QueryCompletedInvocationArchiveActivityAggregateRows, ApiError> {
    let archive_rows = crate::stats::load_completed_invocation_archive_paths_in_range(
        pool,
        Some((range.start, range.end)),
    )
    .await?;
    let mut aggregates = Vec::new();
    let mut skipped_materialized_ranges = Vec::new();
    let mut earliest_created_at_by_prompt_cache_key = HashMap::<String, String>::new();
    let mut prompt_cache_keys_by_account = HashMap::<Option<i64>, HashSet<String>>::new();
    for archive_row in archive_rows {
        let Some((archive_pool, temp_cleanup)) = crate::stats::open_invocation_archive_batch_pool(
            &archive_row,
            "dashboard-activity-summary",
        )
        .await?
        else {
            if archive_row.has_materialized_historical_rollups()
                && let Some(skipped_range) =
                    dashboard_activity_archive_row_overlap_range(&archive_row, range)
            {
                skipped_materialized_ranges.push(skipped_range);
            }
            continue;
        };
        let overlapping_live_ids = query_archive_upstream_account_activity_overlapping_live_ids(
            pool,
            &archive_pool,
            source_scope,
            range,
        )
        .await?;
        let exclude_invocation_ids_filter =
            prepare_dashboard_activity_excluded_invocation_ids_filter(
                &archive_pool,
                Some(&overlapping_live_ids),
            )
            .await?;
        aggregates.extend(
            query_live_upstream_account_activity_aggregate_rows(
                &archive_pool,
                source_scope,
                range,
                false,
                exclude_invocation_ids_filter,
            )
            .await?,
        );
        for row in query_live_upstream_account_prompt_cache_created_at_rows(
            &archive_pool,
            source_scope,
            range,
            false,
            exclude_invocation_ids_filter,
        )
        .await?
        {
            let UpstreamAccountPromptCacheCreatedAtRow {
                upstream_account_id,
                prompt_cache_key,
                first_occurred_at,
            } = row;
            earliest_created_at_by_prompt_cache_key
                .entry(prompt_cache_key.clone())
                .and_modify(|current| {
                    if first_occurred_at < *current {
                        *current = first_occurred_at.clone();
                    }
                })
                .or_insert(first_occurred_at);
            prompt_cache_keys_by_account
                .entry(upstream_account_id)
                .or_default()
                .insert(prompt_cache_key);
        }
        archive_pool.close().await;
        drop(temp_cleanup);
    }
    let mut latest_created_at_by_account = HashMap::<Option<i64>, String>::new();
    for (upstream_account_id, prompt_cache_keys) in prompt_cache_keys_by_account {
        let mut latest_created_at = None;
        for prompt_cache_key in prompt_cache_keys {
            merge_latest_optional_timestamp(
                &mut latest_created_at,
                earliest_created_at_by_prompt_cache_key
                    .get(&prompt_cache_key)
                    .cloned(),
            );
        }
        if let Some(latest_created_at) = latest_created_at {
            latest_created_at_by_account.insert(upstream_account_id, latest_created_at);
        }
    }
    for aggregate in &mut aggregates {
        if let Some(latest_created_at) =
            latest_created_at_by_account.get(&aggregate.upstream_account_id)
        {
            aggregate.latest_conversation_created_at = Some(latest_created_at.clone());
        }
    }
    Ok(QueryCompletedInvocationArchiveActivityAggregateRows {
        aggregates,
        skipped_materialized_ranges,
    })
}
