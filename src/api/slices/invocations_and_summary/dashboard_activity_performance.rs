#[derive(Debug, Clone, FromRow)]
struct SuccessfulBilledUsageDurationIntervalRow {
    upstream_account_id: Option<i64>,
    model: String,
    reasoning_effort: Option<String>,
    start_epoch_ms: f64,
    end_epoch_ms: f64,
}

#[derive(Debug, Default)]
struct ModelPerformanceDurationOverrides {
    total_wall_clock_ms: Option<f64>,
    by_account_wall_clock_ms: HashMap<Option<i64>, f64>,
    by_group_wall_clock_ms: HashMap<UsageBreakdownGroupKey, f64>,
    by_account_group_wall_clock_ms: HashMap<AccountModelGroupKey, f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AccountModelGroupKey {
    upstream_account_id: Option<i64>,
    group: UsageBreakdownGroupKey,
}

#[derive(Debug, Default, Clone, Copy)]
struct UsageDurationUnionAccumulator {
    saw_interval: bool,
    total_ms: f64,
    current_end_epoch_ms: Option<f64>,
}

impl UsageDurationUnionAccumulator {
    fn push_interval(&mut self, start_epoch_ms: f64, end_epoch_ms: f64) {
        if !start_epoch_ms.is_finite() || !end_epoch_ms.is_finite() || end_epoch_ms < start_epoch_ms
        {
            return;
        }
        self.saw_interval = true;
        match self.current_end_epoch_ms {
            None => {
                self.total_ms += end_epoch_ms - start_epoch_ms;
                self.current_end_epoch_ms = Some(end_epoch_ms);
            }
            Some(current_end_epoch_ms) if end_epoch_ms <= current_end_epoch_ms => {}
            Some(current_end_epoch_ms) => {
                self.total_ms += end_epoch_ms - start_epoch_ms.max(current_end_epoch_ms);
                self.current_end_epoch_ms = Some(end_epoch_ms);
            }
        }
    }

    fn total_ms(self) -> Option<f64> {
        self.saw_interval.then_some(self.total_ms)
    }
}

#[derive(Debug, Default)]
struct ModelPerformanceWallClockUnionState {
    total: UsageDurationUnionAccumulator,
    by_account: HashMap<Option<i64>, UsageDurationUnionAccumulator>,
    by_group: HashMap<UsageBreakdownGroupKey, UsageDurationUnionAccumulator>,
    by_account_group: HashMap<AccountModelGroupKey, UsageDurationUnionAccumulator>,
}

impl ModelPerformanceWallClockUnionState {
    fn push_row(&mut self, row: &SuccessfulBilledUsageDurationIntervalRow) {
        let group = UsageBreakdownGroupKey {
            model: row.model.clone(),
            reasoning_effort: row.reasoning_effort.clone(),
        };
        self.total
            .push_interval(row.start_epoch_ms, row.end_epoch_ms);
        self.by_account
            .entry(row.upstream_account_id)
            .or_default()
            .push_interval(row.start_epoch_ms, row.end_epoch_ms);
        self.by_group
            .entry(group.clone())
            .or_default()
            .push_interval(row.start_epoch_ms, row.end_epoch_ms);
        self.by_account_group
            .entry(AccountModelGroupKey {
                upstream_account_id: row.upstream_account_id,
                group,
            })
            .or_default()
            .push_interval(row.start_epoch_ms, row.end_epoch_ms);
    }

    fn into_overrides(self) -> ModelPerformanceDurationOverrides {
        ModelPerformanceDurationOverrides {
            total_wall_clock_ms: self.total.total_ms(),
            by_account_wall_clock_ms: self
                .by_account
                .into_iter()
                .filter_map(|(upstream_account_id, accumulator)| {
                    accumulator
                        .total_ms()
                        .map(|total_ms| (upstream_account_id, total_ms))
                })
                .collect(),
            by_group_wall_clock_ms: self
                .by_group
                .into_iter()
                .filter_map(|(group, accumulator)| {
                    accumulator.total_ms().map(|total_ms| (group, total_ms))
                })
                .collect(),
            by_account_group_wall_clock_ms: self
                .by_account_group
                .into_iter()
                .filter_map(|(key, accumulator)| {
                    accumulator.total_ms().map(|total_ms| (key, total_ms))
                })
                .collect(),
        }
    }
}

#[derive(Debug, Default, Clone)]
struct ModelPerformanceAccumulator {
    total_tokens: i64,
    stream_output_tokens: i64,
    stream_duration_ms: f64,
    response_sample_count: i64,
    response_sum_ms: f64,
    first_byte_sample_count: i64,
    first_byte_sum_ms: f64,
    first_token_sample_count: i64,
    first_token_sum_ms: f64,
    cumulative_usage_duration_sample_count: i64,
    cumulative_usage_duration_sum_ms: f64,
    wall_clock_usage_duration_ms: Option<f64>,
    models: HashMap<UsageBreakdownGroupKey, ModelPerformanceAccumulator>,
}

impl ModelPerformanceAccumulator {
    fn add_aggregate_row(&mut self, row: &UpstreamAccountUsageBreakdownAggregateRow) {
        self.total_tokens += row.performance_total_tokens.max(0);
        self.stream_output_tokens += row.performance_stream_output_tokens.max(0);
        self.stream_duration_ms += row.performance_stream_duration_ms.max(0.0);
        self.response_sample_count += row.performance_response_sample_count.max(0);
        self.response_sum_ms += row.performance_response_sum_ms.max(0.0);
        self.first_byte_sample_count += row.performance_first_byte_sample_count.max(0);
        self.first_byte_sum_ms += row.performance_first_byte_sum_ms.max(0.0);
        self.first_token_sample_count += row.performance_first_token_sample_count.max(0);
        self.first_token_sum_ms += row.performance_first_token_sum_ms.max(0.0);
        self.cumulative_usage_duration_sample_count +=
            row.performance_usage_duration_sample_count.max(0);
        self.cumulative_usage_duration_sum_ms += row.performance_usage_duration_sum_ms.max(0.0);

        let entry = self
            .models
            .entry(UsageBreakdownGroupKey {
                model: row.model.clone(),
                reasoning_effort: row.reasoning_effort.clone(),
            })
            .or_default();
        entry.total_tokens += row.performance_total_tokens.max(0);
        entry.stream_output_tokens += row.performance_stream_output_tokens.max(0);
        entry.stream_duration_ms += row.performance_stream_duration_ms.max(0.0);
        entry.response_sample_count += row.performance_response_sample_count.max(0);
        entry.response_sum_ms += row.performance_response_sum_ms.max(0.0);
        entry.first_byte_sample_count += row.performance_first_byte_sample_count.max(0);
        entry.first_byte_sum_ms += row.performance_first_byte_sum_ms.max(0.0);
        entry.first_token_sample_count += row.performance_first_token_sample_count.max(0);
        entry.first_token_sum_ms += row.performance_first_token_sum_ms.max(0.0);
        entry.cumulative_usage_duration_sample_count +=
            row.performance_usage_duration_sample_count.max(0);
        entry.cumulative_usage_duration_sum_ms += row.performance_usage_duration_sum_ms.max(0.0);
    }

    fn add_terminal_record(&mut self, record: &ApiInvocation) {
        self.add_terminal_delta(&dashboard_activity_terminal_delta(record));
    }

    fn add_terminal_delta(&mut self, delta: &DashboardActivityTerminalDelta) {
        let group = UsageBreakdownGroupKey {
            model: delta.model.clone(),
            reasoning_effort: delta.reasoning_effort.clone(),
        };
        self.add_terminal_delta_values(delta);
        self.models
            .entry(group)
            .or_default()
            .add_terminal_delta_values(delta);
    }

    fn add_terminal_delta_values(&mut self, delta: &DashboardActivityTerminalDelta) {
        let success_like = delta.success;
        let success_billed = success_like && delta.has_cost;
        if success_billed {
            self.total_tokens += delta.total_tokens;
        }
        if let Some(stream_duration_ms) = success_like
            .then_some(delta.t_upstream_stream_ms)
            .flatten()
            .filter(|value| value.is_finite() && *value > 0.0)
        {
            if success_billed {
                self.stream_output_tokens += delta.output_tokens;
                self.stream_duration_ms += stream_duration_ms;
            }
            self.response_sample_count += 1;
            self.response_sum_ms += stream_duration_ms;
        }
        if success_billed
            && delta
                .t_upstream_ttfb_ms
                .is_some_and(|value| value.is_finite() && value > 0.0)
            && let Some(first_response_byte_total_ms) =
                crate::stats::resolve_first_response_byte_total_ms(
                    delta.t_req_read_ms,
                    delta.t_req_parse_ms,
                    delta.t_upstream_connect_ms,
                    delta.t_upstream_ttfb_ms,
                )
        {
            self.first_byte_sample_count += 1;
            self.first_byte_sum_ms += first_response_byte_total_ms;
        }
        if let Some(first_token_ms) = delta
            .first_token_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.first_token_sample_count += 1;
            self.first_token_sum_ms += first_token_ms;
        }
        if success_billed
            && let Some(total_ms) = delta
                .t_total_ms
                .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.cumulative_usage_duration_sample_count += 1;
            self.cumulative_usage_duration_sum_ms += total_ms;
            // The persisted baseline stores only the prior interval union, so a single new
            // interval cannot be merged exactly until reconciliation rebuilds that union.
            self.wall_clock_usage_duration_ms = None;
        }
    }

    fn subtract_terminal_delta(&mut self, delta: &DashboardActivityTerminalDelta) {
        let group = UsageBreakdownGroupKey {
            model: delta.model.clone(),
            reasoning_effort: delta.reasoning_effort.clone(),
        };
        self.subtract_terminal_delta_values(delta);
        if let Some(model) = self.models.get_mut(&group) {
            model.subtract_terminal_delta_values(delta);
            if model.total_tokens == 0
                && model.response_sample_count == 0
                && model.first_byte_sample_count == 0
                && model.first_token_sample_count == 0
                && model.cumulative_usage_duration_sample_count == 0
            {
                self.models.remove(&group);
            }
        }
    }

    fn subtract_terminal_delta_values(&mut self, delta: &DashboardActivityTerminalDelta) {
        let success_billed = delta.success && delta.has_cost;
        if success_billed {
            self.total_tokens = self.total_tokens.saturating_sub(delta.total_tokens);
        }
        if let Some(stream_duration_ms) = delta
            .success
            .then_some(delta.t_upstream_stream_ms)
            .flatten()
            .filter(|value| value.is_finite() && *value > 0.0)
        {
            if success_billed {
                self.stream_output_tokens = self
                    .stream_output_tokens
                    .saturating_sub(delta.output_tokens);
                self.stream_duration_ms = (self.stream_duration_ms - stream_duration_ms).max(0.0);
            }
            self.response_sample_count = self.response_sample_count.saturating_sub(1);
            self.response_sum_ms = (self.response_sum_ms - stream_duration_ms).max(0.0);
        }
        if success_billed
            && delta
                .t_upstream_ttfb_ms
                .is_some_and(|value| value.is_finite() && value > 0.0)
            && let Some(value) = crate::stats::resolve_first_response_byte_total_ms(
                delta.t_req_read_ms,
                delta.t_req_parse_ms,
                delta.t_upstream_connect_ms,
                delta.t_upstream_ttfb_ms,
            )
        {
            self.first_byte_sample_count = self.first_byte_sample_count.saturating_sub(1);
            self.first_byte_sum_ms = (self.first_byte_sum_ms - value).max(0.0);
        }
        if let Some(value) = delta
            .first_token_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.first_token_sample_count = self.first_token_sample_count.saturating_sub(1);
            self.first_token_sum_ms = (self.first_token_sum_ms - value).max(0.0);
        }
        if success_billed
            && let Some(value) = delta
                .t_total_ms
                .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.cumulative_usage_duration_sample_count = self
                .cumulative_usage_duration_sample_count
                .saturating_sub(1);
            self.cumulative_usage_duration_sum_ms =
                (self.cumulative_usage_duration_sum_ms - value).max(0.0);
            self.wall_clock_usage_duration_ms = None;
        }
    }

    fn metrics(&self, range: ExactUtcRange) -> ModelPerformanceMetricsResponse {
        let range_minutes = (range.end - range.start).num_milliseconds() as f64 / 60_000.0;
        let cumulative_usage_duration_ms = (self.cumulative_usage_duration_sample_count > 0)
            .then_some(self.cumulative_usage_duration_sum_ms);
        ModelPerformanceMetricsResponse {
            tokens_per_minute: if range_minutes > 0.0 {
                self.total_tokens as f64 / range_minutes
            } else {
                0.0
            },
            streaming_response_rate: (self.stream_duration_ms > 0.0)
                .then_some(self.stream_output_tokens as f64 / (self.stream_duration_ms / 1_000.0)),
            avg_response_ms: (self.response_sample_count > 0)
                .then_some(self.response_sum_ms / self.response_sample_count as f64),
            avg_first_response_byte_total_ms: (self.first_byte_sample_count > 0)
                .then_some(self.first_byte_sum_ms / self.first_byte_sample_count as f64),
            avg_first_token_ms: (self.first_token_sample_count > 0)
                .then_some(self.first_token_sum_ms / self.first_token_sample_count as f64),
            wall_clock_usage_duration_ms: self.wall_clock_usage_duration_ms,
            cumulative_usage_duration_ms,
            parallelism: match (
                self.wall_clock_usage_duration_ms,
                cumulative_usage_duration_ms,
            ) {
                (Some(wall_clock_ms), Some(cumulative_ms)) if wall_clock_ms > 0.0 => {
                    Some(cumulative_ms / wall_clock_ms)
                }
                _ => None,
            },
        }
    }

    fn into_response(self, range: ExactUtcRange, available: bool) -> ModelPerformanceResponse {
        let total = self.metrics(range);
        let mut models = self
            .models
            .into_iter()
            .filter_map(|(group, entry)| {
                (entry.total_tokens > 0
                    || entry.stream_duration_ms > 0.0
                    || entry.response_sample_count > 0
                    || entry.first_byte_sample_count > 0
                    || entry.first_token_sample_count > 0
                    || entry.cumulative_usage_duration_sample_count > 0
                    || entry.wall_clock_usage_duration_ms.is_some())
                .then_some(ModelPerformanceModelResponse {
                    model: group.model,
                    reasoning_effort: group.reasoning_effort,
                    metrics: entry.metrics(range),
                })
            })
            .collect::<Vec<_>>();
        models.sort_by(|left, right| {
            right
                .metrics
                .cumulative_usage_duration_ms
                .unwrap_or_default()
                .total_cmp(
                    &left
                        .metrics
                        .cumulative_usage_duration_ms
                        .unwrap_or_default(),
                )
                .then_with(|| left.model.cmp(&right.model))
                .then_with(|| left.reasoning_effort.cmp(&right.reasoning_effort))
        });
        ModelPerformanceResponse {
            available,
            total,
            models,
        }
    }

    fn unavailable(range: ExactUtcRange) -> ModelPerformanceResponse {
        Self::default().into_response(range, false)
    }
}

fn compute_model_performance_duration_overrides(
    rows: &[SuccessfulBilledUsageDurationIntervalRow],
) -> ModelPerformanceDurationOverrides {
    let mut union_state = ModelPerformanceWallClockUnionState::default();
    for row in rows {
        union_state.push_row(row);
    }
    union_state.into_overrides()
}

#[derive(Debug, FromRow)]
struct RuntimeRecentAccountFallbackRow {
    invoke_id: String,
    occurred_at: String,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<String>,
    upstream_account_plan_type: Option<String>,
}

const DASHBOARD_ACTIVITY_EXCLUDED_IDS_INLINE_LIMIT: usize = 500;
const DASHBOARD_ACTIVITY_EXCLUDED_IDS_TABLE: &str = "dashboard_activity_excluded_invocation_ids";
const DASHBOARD_ACTIVITY_PREVIEW_ID_HYDRATION_CHUNK_SIZE: usize = 400;

#[derive(Clone, Copy)]
enum DashboardActivityExcludedInvocationIdsFilter<'a> {
    None,
    Inline(&'a HashSet<i64>),
    Table,
}

async fn prepare_dashboard_activity_excluded_invocation_ids_filter<'a>(
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

fn push_excluded_invocation_ids_filter(
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

async fn query_live_upstream_account_activity_aggregate_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    use_attempt_fallback: bool,
    exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter<'_>,
) -> Result<Vec<UpstreamAccountActivityAggregateRow>, ApiError> {
    query_live_upstream_account_activity_aggregate_rows_with_telemetry(
        pool,
        UpstreamAccountActivityAggregateQueryInput {
            source_scope,
            range,
            use_attempt_fallback,
            exclude_invocation_ids,
            min_invocation_id_exclusive: None,
            bucket_min_invocation_ids_exclusive: None,
            route: "internal",
            purpose: "legacy_exact",
        },
    )
    .await
}

struct UpstreamAccountActivityAggregateQueryInput<'a> {
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    use_attempt_fallback: bool,
    exclude_invocation_ids: DashboardActivityExcludedInvocationIdsFilter<'a>,
    min_invocation_id_exclusive: Option<i64>,
    bucket_min_invocation_ids_exclusive: Option<&'a BTreeMap<i64, i64>>,
    route: &'static str,
    purpose: &'static str,
}

struct UpstreamAccountActivityAggregateSql {
    upstream_account_id: String,
    prompt_cache_key: String,
    preaggregated_conversation_created_at: String,
    first_response_byte_components_valid: String,
    first_response_byte_total: String,
    success: String,
    failure: String,
    non_success: String,
    ttfb_positive: String,
    final_first_token: String,
    first_token_nonnegative: String,
    total_nonnegative: String,
}

const UPSTREAM_ACCOUNT_ACTIVITY_AGGREGATE_PREFIX: &str = r#"
        WITH filtered_invocations AS (
            SELECT
                id,
                invoke_id,
                occurred_at,
                status,
                total_tokens,
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
        "#;

const UPSTREAM_ACCOUNT_ACTIVITY_AGGREGATE_SUFFIX: &str = r#"
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
            COALESCE(SUM(COALESCE(cache_input_tokens, 0)), 0) AS cache_input_tokens,
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
        "#;

fn build_upstream_account_activity_aggregate_sql(
    source_scope: InvocationSourceScope,
    use_attempt_fallback: bool,
) -> UpstreamAccountActivityAggregateSql {
    let upstream_account_id = if use_attempt_fallback {
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations")
    } else {
        INVOCATION_UPSTREAM_ACCOUNT_ID_SQL.to_string()
    };
    let prompt_cache_key = INVOCATION_PROMPT_CACHE_KEY_SQL.to_string();
    let filtered_prompt_cache_key = "filtered_invocations.prompt_cache_key";
    let failure_class = INVOCATION_RESOLVED_FAILURE_CLASS_SQL;
    let success = format!(
        "LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed', '{warning_success}') AND ({failure_class}) = 'none'",
        warning_success = INVOCATION_STATUS_WARNING_SUCCESS,
    );
    let failure = format!(
        "LOWER(TRIM(COALESCE(status, ''))) NOT IN ('', 'running', 'pending') AND ({failure_class}) <> 'none'"
    );
    let non_success = format!(
        "(LOWER(TRIM(COALESCE(status, ''))) = 'interrupted' OR (LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending') AND ({failure_class}) <> 'none'))"
    );
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
    let ttfb_positive = sqlite_positive_timing_sql("t_upstream_ttfb_ms");
    let final_first_token = invocation_timing_sql_for_source(
        "filtered_invocations",
        "first_token_ms",
        use_attempt_fallback,
    );
    let first_token_nonnegative = sqlite_nonnegative_timing_sql(&format!("({final_first_token})"));
    let total_nonnegative = sqlite_nonnegative_timing_sql("t_total_ms");
    let preaggregated_conversation_created_at = if use_attempt_fallback {
        prompt_cache_conversation_created_at_sql(filtered_prompt_cache_key, source_scope)
    } else {
        invocation_history_conversation_created_at_sql(filtered_prompt_cache_key, source_scope)
    };
    UpstreamAccountActivityAggregateSql {
        upstream_account_id,
        prompt_cache_key,
        preaggregated_conversation_created_at,
        first_response_byte_components_valid,
        first_response_byte_total,
        success,
        failure,
        non_success,
        ttfb_positive,
        final_first_token,
        first_token_nonnegative,
        total_nonnegative,
    }
}

fn push_upstream_account_activity_aggregate_filters<'a>(
    query: &mut QueryBuilder<'a, Sqlite>,
    input: &UpstreamAccountActivityAggregateQueryInput<'a>,
) -> Result<(), ApiError> {
    query
        .push_bind(db_occurred_at_lower_bound(input.range.start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(input.range.end));
    if input.source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(min_id) = input.min_invocation_id_exclusive {
        query.push(" AND id > ").push_bind(min_id);
    }
    if let Some(bucket_min_ids) = input.bucket_min_invocation_ids_exclusive {
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
    push_excluded_invocation_ids_filter(query, input.exclude_invocation_ids);
    query.push(" AND LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')");
    Ok(())
}

fn render_sql_template(template: &str, values: &[(&str, &str)]) -> String {
    values
        .iter()
        .fold(template.to_owned(), |sql, (name, value)| {
            sql.replace(&format!("{{{name}}}"), value)
        })
}

fn build_upstream_account_activity_aggregate_query<'a>(
    input: &UpstreamAccountActivityAggregateQueryInput<'a>,
    sql: &UpstreamAccountActivityAggregateSql,
) -> Result<QueryBuilder<'a, Sqlite>, ApiError> {
    let mut query = QueryBuilder::<Sqlite>::new(render_sql_template(
        UPSTREAM_ACCOUNT_ACTIVITY_AGGREGATE_PREFIX,
        &[
            ("upstream_account_id_sql", sql.upstream_account_id.as_str()),
            ("prompt_cache_key_sql", sql.prompt_cache_key.as_str()),
        ],
    ));
    push_upstream_account_activity_aggregate_filters(&mut query, input)?;
    query.push(render_sql_template(
        UPSTREAM_ACCOUNT_ACTIVITY_AGGREGATE_SUFFIX,
        &[
            (
                "preaggregated_conversation_created_at_sql",
                sql.preaggregated_conversation_created_at.as_str(),
            ),
            (
                "first_response_byte_total_sql",
                sql.first_response_byte_total.as_str(),
            ),
            ("success_sql", sql.success.as_str()),
            ("failure_sql", sql.failure.as_str()),
            ("non_success_sql", sql.non_success.as_str()),
            ("ttfb_positive_sql", sql.ttfb_positive.as_str()),
            (
                "first_response_byte_components_valid_sql",
                sql.first_response_byte_components_valid.as_str(),
            ),
            (
                "first_token_nonnegative_sql",
                sql.first_token_nonnegative.as_str(),
            ),
            ("final_first_token_sql", sql.final_first_token.as_str()),
            ("total_nonnegative_sql", sql.total_nonnegative.as_str()),
        ],
    ));
    query.push(" GROUP BY filtered_invocations.upstream_account_id");
    Ok(query)
}

async fn query_live_upstream_account_activity_aggregate_rows_with_telemetry<'e, E>(
    executor: E,
    input: UpstreamAccountActivityAggregateQueryInput<'_>,
) -> Result<Vec<UpstreamAccountActivityAggregateRow>, ApiError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let started_at = Instant::now();
    let UpstreamAccountActivityAggregateQueryInput {
        source_scope,
        range,
        use_attempt_fallback,
        route,
        purpose,
        ..
    } = input;
    let sql = build_upstream_account_activity_aggregate_sql(source_scope, use_attempt_fallback);
    let mut query = build_upstream_account_activity_aggregate_query(&input, &sql)?;
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
