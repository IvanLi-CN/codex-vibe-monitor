#[derive(Debug, FromRow)]
pub(crate) struct DashboardActivityRateUsageRow {
    invoke_id: String,
    occurred_at: String,
    upstream_account_id: Option<i64>,
    total_tokens: i64,
    total_cost: f64,
}

pub(crate) fn validate_dashboard_activity_params(
    endpoint: &str,
    range: &str,
    recent_limit: Option<i64>,
) -> Result<usize, ApiError> {
    if !matches!(range, "today" | "yesterday" | "1d" | "7d") {
        return Err(ApiError::bad_request(anyhow!(
            "unsupported {endpoint} range: {range}",
        )));
    }

    match recent_limit {
        Some(value) if !(1..=16).contains(&value) => Err(ApiError::bad_request(anyhow!(
            "recentLimit must be between 1 and 16"
        ))),
        Some(value) => Ok(value as usize),
        None => Ok(4),
    }
}

pub(crate) fn sum_optional_rates(
    accounts: &[DashboardActivityAccountResponse],
    value_of: impl Fn(&DashboardActivityAccountResponse) -> Option<f64>,
) -> Option<f64> {
    let mut saw_value = false;
    let mut total = 0.0;
    for account in accounts {
        if let Some(value) = value_of(account).filter(|value| value.is_finite()) {
            saw_value = true;
            total += value;
        }
    }
    saw_value.then_some(total)
}

async fn load_dashboard_activity_current_minute_rows(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<UpstreamAccountInvocationPreviewRow>, ApiError> {
    let current_window = dashboard_activity_last_complete_minute_window(range);
    let mut rows = query_live_upstream_account_activity_preview_rows_with_limit(
        &state.pool,
        DashboardActivityPreviewRowsQuery {
            source_scope,
            range: current_window,
            upstream_account_id: None,
            limit: None,
            max_id: None,
            in_progress_only: false,
            telemetry: UpstreamAccountActivityPreviewReadTelemetry {
                route: "dashboard",
                builder: "current_minute",
                purpose: "current_minute_preview_rows",
            },
        },
    )
    .await?;
    let mut rows_by_key = rows
        .drain(..)
        .map(|row| ((row.invoke_id.clone(), row.occurred_at.clone()), row))
        .collect::<HashMap<_, _>>();
    let mut unresolved_runtime_account_keys = HashSet::new();

    for record in state.proxy_runtime_invocations.snapshot() {
        let Some(mut row) =
            runtime_upstream_account_activity_preview_row_with_terminal(record, source_scope, true)
        else {
            continue;
        };
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            continue;
        };
        if occurred_at < current_window.start || occurred_at >= current_window.end {
            continue;
        }
        let key = (row.invoke_id.clone(), row.occurred_at.clone());
        if let Some(existing) = rows_by_key.get(&key) {
            if row.upstream_account_id.is_none() {
                row.upstream_account_id = existing.upstream_account_id;
            }
            if row.upstream_account_name.is_none() {
                row.upstream_account_name = existing.upstream_account_name.clone();
            }
            if row.upstream_account_plan_type.is_none() {
                row.upstream_account_plan_type = existing.upstream_account_plan_type.clone();
            }
        }
        if row.upstream_account_id.is_none() {
            unresolved_runtime_account_keys.insert(key.clone());
        }
        rows_by_key.insert(key, row);
    }

    if !unresolved_runtime_account_keys.is_empty() {
        for fallback_row in query_runtime_recent_account_fallback_rows(
            &state.pool,
            source_scope,
            &unresolved_runtime_account_keys,
        )
        .await?
        {
            let key = (
                fallback_row.invoke_id.clone(),
                fallback_row.occurred_at.clone(),
            );
            if let Some(row) = rows_by_key.get_mut(&key) {
                if row.upstream_account_id.is_none() {
                    row.upstream_account_id = fallback_row.upstream_account_id;
                }
                if row.upstream_account_name.is_none() {
                    row.upstream_account_name = fallback_row.upstream_account_name;
                }
                if row.upstream_account_plan_type.is_none() {
                    row.upstream_account_plan_type = fallback_row.upstream_account_plan_type;
                }
            }
        }
    }

    Ok(rows_by_key.into_values().collect())
}

async fn load_dashboard_activity_current_minute_accumulators_by_account(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<HashMap<Option<i64>, DashboardActivityCurrentMinuteAccumulator>, ApiError> {
    let mut accumulators = HashMap::<Option<i64>, DashboardActivityCurrentMinuteAccumulator>::new();
    for row in load_dashboard_activity_current_minute_rows(state, source_scope, range).await? {
        accumulators
            .entry(row.upstream_account_id)
            .or_default()
            .add_row(&row);
    }
    Ok(accumulators)
}

fn sum_dashboard_activity_current_minute_accumulators(
    accumulators: impl IntoIterator<Item = DashboardActivityCurrentMinuteAccumulator>,
) -> DashboardActivityCurrentMinuteAccumulator {
    let mut total = DashboardActivityCurrentMinuteAccumulator::default();
    for accumulator in accumulators {
        total.merge(accumulator);
    }
    total
}

pub(crate) fn sum_dashboard_activity_current_snapshots(
    snapshots: impl IntoIterator<Item = DashboardActivityCurrentSnapshot>,
) -> DashboardActivityCurrentSnapshot {
    let mut total = DashboardActivityCurrentSnapshot::default();
    for snapshot in snapshots {
        total.add_assign(snapshot);
    }
    total
}

pub(crate) fn build_dashboard_activity_summary(
    accounts: &[DashboardActivityAccountResponse],
    include_live_counts: bool,
    current_snapshot: DashboardActivityCurrentSnapshot,
    latest_first_response_byte_total_in_range: Option<f64>,
    latest_avg_total_in_range: Option<f64>,
    model_performance: ModelPerformanceResponse,
) -> DashboardActivitySummaryResponse {
    let ttfb_sum = accounts
        .iter()
        .map(|account| account.in_progress_wait_sum_ms)
        .sum::<f64>();
    let ttfb_count = accounts
        .iter()
        .map(|account| account.in_progress_wait_sample_count)
        .sum::<i64>();

    let mut usage_breakdown = UsageBreakdownAccumulator::default();
    for account in accounts {
        usage_breakdown.merge_response(&account.usage_breakdown);
    }
    let stats = StatsResponse {
        total_count: accounts.iter().map(|account| account.request_count).sum(),
        success_count: accounts.iter().map(|account| account.success_count).sum(),
        failure_count: accounts.iter().map(|account| account.failure_count).sum(),
        total_cost: accounts.iter().map(|account| account.total_cost).sum(),
        total_tokens: accounts.iter().map(|account| account.total_tokens).sum(),
        usage_breakdown: Some(usage_breakdown.into_response()),
        in_progress_conversation_count: include_live_counts.then(|| {
            accounts
                .iter()
                .map(|account| account.in_progress_invocation_count.unwrap_or(0))
                .sum()
        }),
        in_progress_retry_conversation_count: include_live_counts.then(|| {
            accounts
                .iter()
                .map(|account| account.retry_invocation_count.unwrap_or(0))
                .sum()
        }),
        in_progress_avg_wait_ms: (ttfb_count > 0).then_some(ttfb_sum / ttfb_count as f64),
        in_progress_phase_counts: include_live_counts.then(|| {
            accounts.iter().fold(
                InvocationPhaseCountsResponse::default(),
                |mut total, account| {
                    if let Some(counts) = account.in_progress_phase_counts {
                        total.queued += counts.queued;
                        total.requesting += counts.requesting;
                        total.responding += counts.responding;
                    }
                    total
                },
            )
        }),
        non_success_cost: Some(
            accounts
                .iter()
                .map(|account| account.non_success_cost)
                .sum(),
        ),
        non_success_tokens: Some(
            accounts
                .iter()
                .map(|account| account.non_success_tokens)
                .sum(),
        ),
        maintenance: None,
    };

    DashboardActivitySummaryResponse {
        stats,
        tokens_per_minute: Some(
            sum_optional_rates(accounts, |account| account.tokens_per_minute)
                .unwrap_or(current_snapshot.qualified_tokens.max(0) as f64),
        ),
        spend_rate: Some(
            sum_optional_rates(accounts, |account| account.spend_rate)
                .unwrap_or(current_snapshot.total_cost.max(0.0)),
        ),
        current_first_response_byte_total_avg_ms: current_snapshot
            .first_response_byte_total_avg_ms()
            .or(latest_first_response_byte_total_in_range),
        current_first_token_avg_ms: current_snapshot.first_token_avg_ms(),
        current_avg_total_ms: current_snapshot
            .avg_total_ms()
            .or(latest_avg_total_in_range),
        current_avg_response_ms: current_snapshot.avg_response_duration_ms(),
        model_performance,
    }
}

fn build_dashboard_activity_latency_summary(
    current_snapshot: DashboardActivityCurrentSnapshot,
    latest_first_response_byte_total_in_range: Option<f64>,
    latest_avg_total_in_range: Option<f64>,
) -> (Option<f64>, Option<f64>) {
    (
        current_snapshot
            .first_response_byte_total_avg_ms()
            .or(latest_first_response_byte_total_in_range),
        current_snapshot
            .avg_total_ms()
            .or(latest_avg_total_in_range),
    )
}

async fn load_dashboard_activity_current_latency_fallback(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<(Option<f64>, Option<f64>), ApiError> {
    let metric_range = ExactUtcRange {
        start: range
            .start
            .max(range.end - ChronoDuration::minutes(DASHBOARD_ACTIVITY_RATE_WINDOW_MINUTES)),
        end: range.end,
    };
    let mut latest_first_response_byte_total = LatestTimedMetricValue::default();
    let mut latest_avg_total = LatestTimedMetricValue::default();
    for row in query_live_upstream_account_activity_aggregate_rows(
        &state.pool,
        source_scope,
        metric_range,
        true,
        DashboardActivityExcludedInvocationIdsFilter::None,
    )
    .await?
    {
        latest_first_response_byte_total.update(
            row.latest_first_response_byte_total_at,
            row.latest_first_response_byte_total_ms,
        );
        latest_avg_total.update(row.latest_avg_total_at, row.latest_avg_total_ms);
    }

    Ok((
        latest_first_response_byte_total.value,
        latest_avg_total.value,
    ))
}

pub(crate) async fn query_dashboard_activity_rate_usage_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<DashboardActivityRateUsageRow>, ApiError> {
    let rate_window_start = range
        .start
        .max(range.end - ChronoDuration::minutes(DASHBOARD_ACTIVITY_RATE_WINDOW_MINUTES));
    let upstream_account_id_sql =
        invocation_upstream_account_id_with_attempt_fallback_sql("codex_invocations");
    let mut query = QueryBuilder::<Sqlite>::new("SELECT invoke_id, occurred_at, ");
    query
        .push(upstream_account_id_sql.as_str())
        .push(
            " AS upstream_account_id, \
             COALESCE(total_tokens, 0) AS total_tokens, \
             COALESCE(cost, 0.0) AS total_cost \
             FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(db_occurred_at_lower_bound(rate_window_start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_upper_bound(range.end));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    Ok(query
        .build_query_as::<DashboardActivityRateUsageRow>()
        .fetch_all(pool)
        .await?)
}

pub(crate) async fn load_dashboard_activity_rate_events_by_account(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<HashMap<Option<i64>, Vec<UpstreamAccountRateUsageEvent>>, ApiError> {
    let rows = query_dashboard_activity_rate_usage_rows(&state.pool, source_scope, range).await?;
    let db_account_by_key = rows
        .iter()
        .map(|row| {
            (
                (row.invoke_id.clone(), row.occurred_at.clone()),
                row.upstream_account_id,
            )
        })
        .collect::<HashMap<_, _>>();
    let runtime_snapshot = state.proxy_runtime_invocations.snapshot();
    let terminal_keys =
        query_terminal_db_keys_for_runtime_records(&state.pool, &runtime_snapshot, None).await?;
    let rate_window_start = range
        .start
        .max(range.end - ChronoDuration::minutes(DASHBOARD_ACTIVITY_RATE_WINDOW_MINUTES));
    let mut runtime_events_by_key =
        HashMap::<(String, String), (Option<i64>, UpstreamAccountRateUsageEvent)>::new();
    for record in runtime_snapshot {
        if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
            continue;
        }
        if !matches!(
            normalized_runtime_text(record.status.as_deref()).as_str(),
            "running" | "pending"
        ) {
            continue;
        }
        let key = (record.invoke_id.clone(), record.occurred_at.clone());
        if terminal_keys.contains(&key) {
            continue;
        }
        let Some(occurred_at) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        if occurred_at < rate_window_start || occurred_at >= range.end {
            continue;
        }
        let upstream_account_id = record
            .upstream_account_id
            .or_else(|| db_account_by_key.get(&key).copied().flatten());
        runtime_events_by_key.insert(
            key,
            (
                upstream_account_id,
                UpstreamAccountRateUsageEvent {
                    occurred_at_epoch_ms: occurred_at.timestamp_millis(),
                    total_tokens: record.total_tokens.unwrap_or_default().max(0),
                    total_cost: record.cost.unwrap_or_default().max(0.0),
                },
            ),
        );
    }

    let mut events_by_account = HashMap::<Option<i64>, Vec<UpstreamAccountRateUsageEvent>>::new();
    for row in rows {
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            continue;
        };
        if runtime_events_by_key.contains_key(&(row.invoke_id.clone(), row.occurred_at.clone())) {
            continue;
        }
        events_by_account
            .entry(row.upstream_account_id)
            .or_default()
            .push(UpstreamAccountRateUsageEvent {
                occurred_at_epoch_ms: occurred_at.timestamp_millis(),
                total_tokens: row.total_tokens.max(0),
                total_cost: row.total_cost.max(0.0),
            });
    }

    for (_key, (upstream_account_id, event)) in runtime_events_by_key {
        events_by_account
            .entry(upstream_account_id)
            .or_default()
            .push(event);
    }

    Ok(events_by_account)
}

pub(crate) fn sum_dashboard_activity_rate_events(
    events_by_account: &HashMap<Option<i64>, Vec<UpstreamAccountRateUsageEvent>>,
    range: ExactUtcRange,
) -> (Option<f64>, Option<f64>) {
    let mut saw_tokens_per_minute = false;
    let mut saw_spend_rate = false;
    let mut tokens_per_minute_total = 0.0;
    let mut spend_rate_total = 0.0;

    for events in events_by_account.values() {
        let (tokens_per_minute, spend_rate) =
            compute_upstream_account_activity_rates(events, range.start, range.end);
        if let Some(value) = tokens_per_minute.filter(|value| value.is_finite()) {
            saw_tokens_per_minute = true;
            tokens_per_minute_total += value;
        }
        if let Some(value) = spend_rate.filter(|value| value.is_finite()) {
            saw_spend_rate = true;
            spend_rate_total += value;
        }
    }

    (
        saw_tokens_per_minute.then_some(tokens_per_minute_total),
        saw_spend_rate.then_some(spend_rate_total),
    )
}

pub(crate) fn dashboard_activity_last_complete_minute_window(
    range: ExactUtcRange,
) -> ExactUtcRange {
    let closed_window_end = range
        .end
        .with_second(0)
        .and_then(|value| value.with_nanosecond(0))
        .expect("valid exact minute");
    let closed_window_start = (closed_window_end - ChronoDuration::minutes(1)).max(range.start);
    ExactUtcRange {
        start: closed_window_start,
        end: closed_window_end.max(closed_window_start),
    }
}

pub(crate) async fn load_dashboard_activity_summary_only_snapshot(
    state: &AppState,
    range_name: &str,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<DashboardActivitySnapshot, ApiError> {
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    let model_performance_available = range.start >= retention_cutoff;
    let mut totals =
        query_hourly_backed_summary_range(state, range.start, range.end, source_scope).await?;
    totals = totals.add(
        dashboard_activity_historical_live_gap_totals(state, source_scope, range, retention_cutoff)
            .await?,
    );
    let mut stats = totals.into_response();
    stats.non_success_cost = Some(totals.non_success_cost);
    let augmentation = load_summary_live_augmentation(
        state,
        source_scope,
        None,
        Some((range.start, range.end)),
        SummaryLiveAugmentationPolicy {
            include_in_progress: range_name != "yesterday",
            // Hourly summary rollups do not retain non-success token totals; a raw full-range
            // scan here would defeat the summary-only fast path.
            include_non_success_tokens: false,
        },
        None,
    )
    .await?;
    apply_summary_live_augmentation(&mut stats, augmentation);
    let current_snapshot = if range_name == "yesterday" {
        let current_minute_by_account =
            load_dashboard_activity_current_minute_accumulators_by_account(
                state,
                source_scope,
                range,
            )
            .await?;
        sum_dashboard_activity_current_minute_accumulators(current_minute_by_account.into_values())
            .into_current_snapshot()
    } else {
        sum_dashboard_activity_current_snapshots(
            state
                .dashboard_network_speed_cache
                .snapshot_dashboard_activity_accounts(range.end)
                .into_values(),
        )
    };
    let snapshot_first_response_byte_total_avg_ms =
        current_snapshot.first_response_byte_total_avg_ms();
    let snapshot_avg_total_ms = current_snapshot.avg_total_ms();
    let (latest_first_response_byte_total_avg_ms, latest_avg_total_ms) =
        if snapshot_first_response_byte_total_avg_ms.is_none() || snapshot_avg_total_ms.is_none() {
            load_dashboard_activity_current_latency_fallback(state, source_scope, range).await?
        } else {
            (None, None)
        };
    let (current_first_response_byte_total_avg_ms, current_avg_total_ms) =
        build_dashboard_activity_latency_summary(
            current_snapshot,
            latest_first_response_byte_total_avg_ms,
            latest_avg_total_ms,
        );

    Ok(DashboardActivitySnapshot {
        range: range_name.to_string(),
        range_start: range.start,
        range_end: range.end,
        terminal_sequence: 0,
        accounts: Vec::new(),
        summary: DashboardActivitySummaryResponse {
            stats,
            tokens_per_minute: Some(current_snapshot.qualified_tokens.max(0) as f64),
            spend_rate: Some(current_snapshot.total_cost.max(0.0)),
            current_first_token_avg_ms: current_snapshot.first_token_avg_ms(),
            current_first_response_byte_total_avg_ms,
            current_avg_total_ms,
            current_avg_response_ms: current_snapshot.avg_response_duration_ms(),
            model_performance: ModelPerformanceAccumulator::default()
                .into_response(range, model_performance_available),
        },
        summary_model_performance_accumulator: ModelPerformanceAccumulator::default(),
        account_model_performance_accumulators: HashMap::new(),
        account_latency_accumulators: HashMap::new(),
        model_performance_accumulator_ready: false,
        materialized_archive_fallback_totals: StatsTotals::default(),
        materialized_archive_details_limited: false,
        build_telemetry: DashboardActivityBuildTelemetry::default(),
    })
}

pub(crate) async fn load_dashboard_activity_snapshot(
    state: &AppState,
    range_name: &str,
    reporting_tz: Tz,
    recent_limit: usize,
    include_accounts: bool,
    include_recent: bool,
    in_progress_counts_override: Option<HashMap<Option<i64>, UpstreamAccountInProgressSummary>>,
) -> Result<DashboardActivitySnapshot, ApiError> {
    let range = resolve_dashboard_activity_exact_range(range_name, reporting_tz)?;
    load_dashboard_activity_snapshot_for_range_input(DashboardActivitySnapshotForRangeInput {
        state,
        range_name,
        range,
        recent_limit,
        include_accounts,
        include_recent,
        in_progress_counts_override,
    })
    .await
}

async fn load_dashboard_activity_snapshot_for_range_input(
    input: DashboardActivitySnapshotForRangeInput<'_>,
) -> Result<DashboardActivitySnapshot, ApiError> {
    let DashboardActivitySnapshotForRangeInput {
        state,
        range_name,
        range,
        recent_limit,
        include_accounts,
        include_recent,
        in_progress_counts_override,
    } = input;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    if !include_accounts {
        return load_dashboard_activity_summary_only_snapshot(
            state,
            range_name,
            source_scope,
            range,
        )
        .await;
    }
    let build = load_dashboard_activity_account_build_result(
        state,
        range_name,
        range,
        recent_limit,
        include_recent,
        in_progress_counts_override,
        DashboardActivityAccountBuilderKind::DashboardFull,
    )
    .await?;
    let mut summary = build_dashboard_activity_summary(
        &build.accounts,
        range_name != "yesterday",
        build.current_snapshot_summary,
        build.latest_first_response_byte_total_in_range,
        build.latest_avg_total_in_range,
        build.summary_model_performance,
    );
    dashboard_activity_apply_materialized_archive_fallback_to_stats(
        &mut summary.stats,
        build.materialized_archive_fallback_totals,
    );
    if build.materialized_archive_details_limited {
        dashboard_activity_clear_materialized_archive_detail_fields(&mut summary.stats);
    }
    Ok(DashboardActivitySnapshot {
        range: range_name.to_string(),
        range_start: range.start,
        range_end: range.end,
        terminal_sequence: 0,
        accounts: build.accounts,
        summary,
        summary_model_performance_accumulator: build.summary_model_performance_accumulator,
        account_model_performance_accumulators: build.account_model_performance_accumulators,
        account_latency_accumulators: build.account_latency_accumulators,
        model_performance_accumulator_ready: true,
        materialized_archive_fallback_totals: build.materialized_archive_fallback_totals,
        materialized_archive_details_limited: build.materialized_archive_details_limited,
        build_telemetry: build.build_telemetry,
    })
}

struct DashboardActivitySnapshotForRangeInput<'a> {
    state: &'a AppState,
    range_name: &'a str,
    range: ExactUtcRange,
    recent_limit: usize,
    include_accounts: bool,
    include_recent: bool,
    in_progress_counts_override: Option<HashMap<Option<i64>, UpstreamAccountInProgressSummary>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DashboardActivityAccountBuilderKind {
    DashboardFull,
    UpstreamAccount,
}

impl DashboardActivityAccountBuilderKind {
    fn preview_read_telemetry(self) -> UpstreamAccountActivityPreviewReadTelemetry {
        match self {
            Self::DashboardFull => UpstreamAccountActivityPreviewReadTelemetry {
                route: "dashboard",
                builder: "dashboard_full",
                purpose: "bounded_per_account_recent",
            },
            Self::UpstreamAccount => UpstreamAccountActivityPreviewReadTelemetry {
                route: "upstream_account",
                builder: "upstream_account",
                purpose: "bounded_per_account_recent",
            },
        }
    }
}

#[derive(Debug)]
struct DashboardActivityAccountBuildResult {
    accounts: Vec<DashboardActivityAccountResponse>,
    current_snapshot_summary: DashboardActivityCurrentSnapshot,
    latest_first_response_byte_total_in_range: Option<f64>,
    latest_avg_total_in_range: Option<f64>,
    summary_model_performance: ModelPerformanceResponse,
    summary_model_performance_accumulator: ModelPerformanceAccumulator,
    account_model_performance_accumulators: HashMap<Option<i64>, ModelPerformanceAccumulator>,
    account_latency_accumulators: HashMap<Option<i64>, DashboardActivityAccountLatencyAccumulator>,
    materialized_archive_fallback_totals: StatsTotals,
    materialized_archive_details_limited: bool,
    build_telemetry: DashboardActivityBuildTelemetry,
}

struct DashboardActivityAccountResponseInput<'a> {
    range_name: &'a str,
    range: ExactUtcRange,
    model_performance_available: bool,
    current_snapshot_by_account: &'a HashMap<Option<i64>, DashboardActivityCurrentSnapshot>,
    account_meta: &'a HashMap<i64, UpstreamAccountActivityMetaRow>,
    effective_routing_rules: &'a HashMap<i64, crate::upstream_accounts::EffectiveRoutingRule>,
    in_progress_counts: &'a HashMap<Option<i64>, UpstreamAccountInProgressSummary>,
    upstream_account_id: Option<i64>,
    aggregate: UpstreamAccountActivityAccumulator,
}

struct DashboardActivityPreparedAccountResponse {
    upstream_account_id: Option<i64>,
    account_key: String,
    display_name: String,
    group_name: Option<String>,
    plan_type: Option<String>,
    status_fields: Option<UpstreamAccountActivityStatusFields>,
    aggregate: UpstreamAccountActivityAccumulator,
    model_performance: ModelPerformanceResponse,
    current_first_response_byte_total_avg_ms: Option<f64>,
    current_first_token_avg_ms: Option<f64>,
    current_avg_total_ms: Option<f64>,
    current_avg_response_ms: Option<f64>,
    tokens_per_minute: Option<f64>,
    spend_rate: Option<f64>,
    in_progress_invocation_count: Option<i64>,
    in_progress_phase_counts: Option<InvocationPhaseCountsResponse>,
    retry_invocation_count: Option<i64>,
    effective_routing_rule: Option<crate::upstream_accounts::EffectiveRoutingRule>,
}

fn build_dashboard_activity_account_response(
    input: DashboardActivityAccountResponseInput<'_>,
) -> DashboardActivityAccountResponse {
    build_dashboard_activity_account_response_value(prepare_dashboard_activity_account_response(
        input,
    ))
}

fn prepare_dashboard_activity_account_response(
    input: DashboardActivityAccountResponseInput<'_>,
) -> DashboardActivityPreparedAccountResponse {
    let DashboardActivityAccountResponseInput {
        range_name,
        range,
        model_performance_available,
        current_snapshot_by_account,
        account_meta,
        effective_routing_rules,
        in_progress_counts,
        upstream_account_id,
        aggregate,
    } = input;
    let meta = upstream_account_id.and_then(|id| account_meta.get(&id));
    let status_fields =
        meta.map(|row| build_upstream_account_activity_status_fields(row, Utc::now()));
    let model_performance = aggregate
        .model_performance
        .clone()
        .into_response(range, model_performance_available);
    let current_snapshot = current_snapshot_by_account
        .get(&upstream_account_id)
        .copied()
        .unwrap_or_default();
    let (current_first_response_byte_total_avg_ms, current_avg_total_ms) =
        build_dashboard_activity_latency_summary(
            current_snapshot,
            aggregate.latest_first_response_byte_total_ms,
            aggregate.latest_avg_total_ms,
        );
    let tokens_per_minute = Some(current_snapshot.qualified_tokens.max(0) as f64);
    let spend_rate = Some(current_snapshot.total_cost.max(0.0));
    let (in_progress_invocation_count, in_progress_phase_counts, retry_invocation_count) =
        if range_name == "yesterday" {
            (None, None, None)
        } else {
            let summary = in_progress_counts
                .get(&upstream_account_id)
                .copied()
                .unwrap_or_default();
            (
                Some(summary.in_progress_count),
                Some(summary.phase_counts),
                Some(summary.retry_count),
            )
        };
    let account_key = upstream_account_id
        .map(|id| format!("upstream:{id}"))
        .unwrap_or_else(|| "unassigned".to_string());
    let display_name = upstream_account_id
        .map(|id| {
            resolve_upstream_account_activity_display_name(
                id,
                meta,
                aggregate.display_name_hint.as_deref(),
            )
        })
        .unwrap_or_else(|| "未分配上游账号".to_string());
    let group_name =
        normalize_trimmed_optional_string_local(meta.and_then(|row| row.group_name.clone()));
    let plan_type = normalize_trimmed_optional_string_local(
        meta.and_then(|row| row.plan_type.clone())
            .or(aggregate.plan_type_hint.clone()),
    );
    let effective_routing_rule = upstream_account_id.map(|id| {
        effective_routing_rules
            .get(&id)
            .cloned()
            .unwrap_or_else(crate::upstream_accounts::default_effective_routing_rule)
    });
    DashboardActivityPreparedAccountResponse {
        upstream_account_id,
        account_key,
        display_name,
        group_name,
        plan_type,
        status_fields,
        aggregate,
        model_performance,
        current_first_response_byte_total_avg_ms,
        current_first_token_avg_ms: current_snapshot.first_token_avg_ms(),
        current_avg_total_ms,
        current_avg_response_ms: current_snapshot.avg_response_duration_ms(),
        tokens_per_minute,
        spend_rate,
        in_progress_invocation_count,
        in_progress_phase_counts,
        retry_invocation_count,
        effective_routing_rule,
    }
}

fn build_dashboard_activity_account_response_value(
    prepared: DashboardActivityPreparedAccountResponse,
) -> DashboardActivityAccountResponse {
    let DashboardActivityPreparedAccountResponse {
        upstream_account_id,
        account_key,
        display_name,
        group_name,
        plan_type,
        status_fields,
        aggregate,
        model_performance,
        current_first_response_byte_total_avg_ms,
        current_first_token_avg_ms,
        current_avg_total_ms,
        current_avg_response_ms,
        tokens_per_minute,
        spend_rate,
        in_progress_invocation_count,
        in_progress_phase_counts,
        retry_invocation_count,
        effective_routing_rule,
    } = prepared;
    DashboardActivityAccountResponse {
        account_key,
        upstream_account_id,
        display_name,
        is_unassigned: upstream_account_id.is_none(),
        latest_conversation_created_at: aggregate.latest_conversation_created_at,
        last_invocation_at: aggregate.last_invocation_at,
        group_name,
        plan_type,
        enabled: status_fields.as_ref().map(|fields| fields.enabled),
        display_status: status_fields
            .as_ref()
            .map(|fields| fields.display_status.clone()),
        enable_status: status_fields
            .as_ref()
            .map(|fields| fields.enable_status.clone()),
        work_status: status_fields
            .as_ref()
            .map(|fields| fields.work_status.clone()),
        health_status: status_fields
            .as_ref()
            .map(|fields| fields.health_status.clone()),
        sync_state: status_fields
            .as_ref()
            .map(|fields| fields.sync_state.clone()),
        last_error: status_fields
            .as_ref()
            .and_then(|fields| fields.last_error.clone()),
        last_action_reason_message: status_fields
            .as_ref()
            .and_then(|fields| fields.last_action_reason_message.clone()),
        request_count: aggregate.request_count,
        success_count: aggregate.success_count,
        failure_count: aggregate.failure_count,
        non_success_count: aggregate.non_success_count,
        total_tokens: aggregate.total_tokens,
        success_tokens: aggregate.success_tokens,
        non_success_tokens: aggregate.non_success_tokens,
        failure_tokens: aggregate.failure_tokens,
        failure_cost: aggregate.failure_cost,
        non_success_cost: aggregate.non_success_cost,
        total_cost: aggregate.total_cost,
        usage_breakdown: aggregate.usage_breakdown.clone().into_response(),
        model_performance,
        cache_hit_rate: (aggregate.total_tokens > 0)
            .then_some(aggregate.cache_input_tokens as f64 / aggregate.total_tokens as f64),
        tokens_per_minute,
        spend_rate,
        first_byte_avg_ms: (aggregate.first_response_byte_total_sample_count > 0).then_some(
            aggregate.first_response_byte_total_sum_ms
                / aggregate.first_response_byte_total_sample_count as f64,
        ),
        first_response_byte_total_avg_ms: (aggregate.first_response_byte_total_sample_count > 0)
            .then_some(
                aggregate.first_response_byte_total_sum_ms
                    / aggregate.first_response_byte_total_sample_count as f64,
            ),
        first_token_avg_ms: (aggregate.first_token_sample_count > 0)
            .then_some(aggregate.first_token_sum_ms / aggregate.first_token_sample_count as f64),
        avg_total_ms: (aggregate.total_latency_sample_count > 0).then_some(
            aggregate.total_latency_sum_ms / aggregate.total_latency_sample_count as f64,
        ),
        current_first_response_byte_total_avg_ms,
        current_first_token_avg_ms,
        current_avg_total_ms,
        current_avg_response_ms,
        in_progress_invocation_count,
        in_progress_phase_counts,
        retry_invocation_count,
        upload_bytes_per_second: 0.0,
        download_bytes_per_second: 0.0,
        in_progress_wait_sum_ms: aggregate.in_progress_wait_sum_ms,
        in_progress_wait_sample_count: aggregate.in_progress_wait_sample_count,
        effective_routing_rule,
        recent_invocations: aggregate.recent_invocations,
    }
}
