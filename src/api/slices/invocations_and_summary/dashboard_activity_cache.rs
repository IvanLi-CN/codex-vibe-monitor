fn insert_dashboard_activity_applied_terminal_key(
    read_model: &mut DashboardActivityReadModel,
    key: (String, String),
) {
    let applied_at = Instant::now();
    read_model
        .applied_terminal_keys
        .insert(key.clone(), applied_at);
    read_model
        .applied_terminal_key_order
        .push_back((key, applied_at));
}

fn dashboard_activity_terminal_delta_hard_limit_reason(
    read_model: &DashboardActivityReadModel,
    delta: &DashboardActivityTerminalDelta,
) -> Option<&'static str> {
    if read_model.pending_terminal_deltas.len()
        >= DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_TERMINALS
    {
        Some("count_limit")
    } else if read_model
        .pending_delta_estimated_bytes
        .saturating_add(delta.estimated_bytes)
        > DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_BYTES
    {
        Some("byte_limit")
    } else {
        None
    }
}

fn dashboard_activity_terminal_delta_needs_persistence_ack(
    delta: &DashboardActivityTerminalDelta,
) -> bool {
    delta.persisted_row_id.is_none()
}

fn prune_dashboard_activity_terminal_deltas(cache: &mut DashboardActivitySnapshotCacheState) {
    let minimum_cursor = cache
        .entries
        .values()
        .map(|entry| entry.baseline_snapshot_cursor)
        .chain(
            cache
                .in_flight
                .values()
                .filter_map(|flight| flight.baseline_cursor),
        )
        .min();
    let can_prune_all_persisted = cache.entries.is_empty() && cache.in_flight.is_empty();
    let before = cache.read_model.pending_terminal_deltas.len();
    cache.read_model.pending_terminal_deltas.retain(|delta| {
        let Some(row_id) = delta.persisted_row_id else {
            return true;
        };
        !(can_prune_all_persisted || minimum_cursor.is_some_and(|cursor| cursor >= row_id))
    });
    let pruned = before.saturating_sub(cache.read_model.pending_terminal_deltas.len());
    cache.read_model.delta_pruned_count = cache
        .read_model
        .delta_pruned_count
        .saturating_add(pruned as u64);
    cache.read_model.pending_delta_estimated_bytes = cache
        .read_model
        .pending_terminal_deltas
        .iter()
        .map(|delta| delta.estimated_bytes)
        .sum();
    cache.read_model.persisted_ack_pending_count = cache
        .read_model
        .pending_terminal_deltas
        .iter()
        .filter(|delta| delta.persisted_row_id.is_some())
        .count();
}

fn prune_dashboard_activity_snapshot_entries(
    cache: &mut DashboardActivitySnapshotCacheState,
    protected_selection: Option<&DashboardActivitySnapshotSelection>,
) -> usize {
    let mut evicted = 0usize;
    loop {
        let expiry_bytes = cache
            .entries
            .values()
            .map(|entry| entry.expiry_delta_estimated_bytes)
            .sum::<usize>();
        if cache.entries.len() <= DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_MAX_ENTRIES
            && expiry_bytes <= DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_MAX_EXPIRY_BYTES
        {
            break;
        }

        let candidate = cache
            .entries
            .iter()
            .filter(|(selection, _)| protected_selection != Some(*selection))
            .min_by_key(|(_, entry)| entry.cached_at)
            .map(|(selection, _)| selection.clone())
            .or_else(|| protected_selection.cloned());
        let Some(candidate) = candidate else {
            break;
        };
        cache.entries.remove(&candidate);
        cache.invalidation_reasons.remove(&candidate);
        evicted += 1;
    }
    evicted
}

fn dashboard_activity_hard_limit_is_settled(read_model: &DashboardActivityReadModel) -> bool {
    read_model
        .hard_limit_sequence
        .is_none_or(|sequence| read_model.settled_terminal_sequence >= sequence)
}

fn dashboard_activity_read_model_requires_reconcile(
    read_model: &DashboardActivityReadModel,
) -> bool {
    read_model.hard_limit_reason.is_some() && dashboard_activity_hard_limit_is_settled(read_model)
}

fn settle_dashboard_activity_terminal_sequence(
    read_model: &mut DashboardActivityReadModel,
    terminal_sequence: u64,
) {
    if terminal_sequence <= read_model.settled_terminal_sequence {
        return;
    }
    read_model
        .settled_terminal_sequences
        .insert(terminal_sequence);
    while let Some(next_sequence) = read_model.settled_terminal_sequence.checked_add(1)
        && read_model.settled_terminal_sequences.remove(&next_sequence)
    {
        read_model.settled_terminal_sequence = next_sequence;
    }
}

fn clear_dashboard_activity_hard_limit_after_baseline(read_model: &mut DashboardActivityReadModel) {
    if dashboard_activity_hard_limit_is_settled(read_model)
        && read_model.pending_terminal_deltas.len()
            < DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_TERMINALS
        && read_model.pending_delta_estimated_bytes
            < DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_BYTES
    {
        read_model.hard_limit_reason = None;
        read_model.hard_limit_sequence = None;
    }
}

pub(crate) async fn acknowledge_dashboard_activity_terminal_record(
    cache: &Arc<tokio::sync::Mutex<DashboardActivitySnapshotCacheState>>,
    invoke_id: &str,
    occurred_at: &str,
    row_id: i64,
    terminal_sequence: Option<u64>,
) {
    let mut cache = cache.lock().await;
    // A rejected enqueue must not settle its speculative sequence.  The contiguous watermark is
    // advanced exclusively by acknowledge_dashboard_activity_terminal_record after commit.
    let mut ack_sequence_gap = false;
    for delta in &mut cache.read_model.pending_terminal_deltas {
        if delta.invoke_id == invoke_id && delta.occurred_at == occurred_at {
            ack_sequence_gap = delta
                .persisted_row_id
                .is_some_and(|persisted| persisted != row_id);
            delta.persisted_row_id = Some(row_id);
            break;
        }
    }
    if ack_sequence_gap {
        cache.read_model.sequence_gap_count += 1;
        cache.read_model.hard_limit_reason = Some("ack_sequence_gap");
    } else if let Some(terminal_sequence) = terminal_sequence {
        settle_dashboard_activity_terminal_sequence(&mut cache.read_model, terminal_sequence);
    }
    // A writer retry can acknowledge a delta after every warm cursor already allowed it to be
    // pruned. Matching row IDs are therefore idempotent; only a conflicting ACK is a gap.
    prune_dashboard_activity_terminal_deltas(&mut cache);
}

/// The write-side delta is registered before an asynchronous SQLite enqueue. If the queue
/// rejects that enqueue, discard the speculative delta and force future reads to rebuild from
/// persisted state rather than attempting an unsafe inverse for every aggregate field.
pub(crate) async fn rollback_dashboard_activity_terminal_record(
    state: &AppState,
    record: &ApiInvocation,
    _terminal_sequence: Option<u64>,
) {
    let key = (record.invoke_id.clone(), record.occurred_at.clone());
    let mut cache = state.dashboard_activity_snapshot_cache.lock().await;
    cache.read_model.applied_terminal_keys.remove(&key);
    cache
        .read_model
        .pending_terminal_deltas
        .retain(|pending| pending.key() != key);
    cache.read_model.pending_delta_estimated_bytes = cache
        .read_model
        .pending_terminal_deltas
        .iter()
        .map(|delta| delta.estimated_bytes)
        .sum();
    cache.read_model.persisted_ack_pending_count = cache
        .read_model
        .pending_terminal_deltas
        .iter()
        .filter(|delta| delta.persisted_row_id.is_some())
        .count();
    cache.entries.clear();
    cache.invalidation_reasons.clear();
    clear_dashboard_activity_hard_limit_after_baseline(&mut cache.read_model);
}

/// Recovery can replace a previously persisted running row with its terminal form. Its positive
/// row ID is not proof that a warm baseline observed the terminal fields, so force the next read
/// to rebuild from persisted state before registering the recovered record as pending.
pub(crate) async fn invalidate_dashboard_activity_baselines_for_recovery(state: &AppState) {
    let mut cache = state.dashboard_activity_snapshot_cache.lock().await;
    cache.entries.clear();
    cache.invalidation_reasons.clear();
}

impl Default for DashboardActivityBuildTelemetry {
    fn default() -> Self {
        Self {
            preview_read_mode: "none",
            candidate_preview_id_count: 0,
            hydrated_preview_row_count: 0,
            account_aggregation: AccountActivityRangeBuildTelemetry::default(),
        }
    }
}

fn dashboard_activity_build_scope(include_accounts: bool, _include_recent: bool) -> &'static str {
    if !include_accounts {
        "summary_only"
    } else {
        "full"
    }
}

fn dashboard_activity_stats_totals_from_aggregate_rows(
    rows: &[UpstreamAccountActivityAggregateRow],
) -> StatsTotals {
    let mut totals = StatsTotals::default();
    for row in rows {
        totals.total_count += row.request_count;
        totals.success_count += row.success_count;
        totals.failure_count += row.failure_count;
        totals.total_cost += row.total_cost;
        totals.total_tokens += row.total_tokens;
        totals.non_success_cost += row.non_success_cost;
    }
    totals
}

fn dashboard_activity_stats_totals_subtract(left: StatsTotals, right: StatsTotals) -> StatsTotals {
    StatsTotals {
        total_count: left.total_count.saturating_sub(right.total_count).max(0),
        success_count: left
            .success_count
            .saturating_sub(right.success_count)
            .max(0),
        failure_count: left
            .failure_count
            .saturating_sub(right.failure_count)
            .max(0),
        total_cost: (left.total_cost - right.total_cost).max(0.0),
        total_tokens: left.total_tokens.saturating_sub(right.total_tokens).max(0),
        non_success_cost: (left.non_success_cost - right.non_success_cost).max(0.0),
    }
}

fn dashboard_activity_stats_totals_has_values(totals: StatsTotals) -> bool {
    totals.total_count > 0
        || totals.success_count > 0
        || totals.failure_count > 0
        || totals.total_cost > 0.0
        || totals.total_tokens > 0
        || totals.non_success_cost > 0.0
}

async fn dashboard_activity_materialized_archive_fallback_totals(
    state: &AppState,
    source_scope: InvocationSourceScope,
    skipped_materialized_ranges: Vec<ExactUtcRange>,
) -> Result<StatsTotals, ApiError> {
    let mut fallback_totals = StatsTotals::default();
    for skipped_range in skipped_materialized_ranges {
        let rollup_totals = query_hourly_backed_summary_range(
            state,
            skipped_range.start,
            skipped_range.end,
            source_scope,
        )
        .await?;
        let live_rows = query_live_upstream_account_activity_aggregate_rows(
            &state.pool,
            source_scope,
            skipped_range,
            true,
            DashboardActivityExcludedInvocationIdsFilter::None,
        )
        .await?;
        let live_totals = dashboard_activity_stats_totals_from_aggregate_rows(&live_rows);
        fallback_totals = fallback_totals.add(dashboard_activity_stats_totals_subtract(
            rollup_totals,
            live_totals,
        ));
    }
    Ok(fallback_totals)
}

async fn dashboard_activity_materialized_archive_first_token_fallback_totals(
    state: &AppState,
    source_scope: InvocationSourceScope,
    skipped_materialized_ranges: &[ExactUtcRange],
) -> Result<(i64, f64), ApiError> {
    let mut sample_count = 0_i64;
    let mut sum_ms = 0.0_f64;
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    for skipped_range in skipped_materialized_ranges {
        let range_plan = build_hourly_rollup_exact_range_plan(
            skipped_range.start,
            skipped_range.end,
            retention_cutoff,
        )?;
        let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range else {
            continue;
        };
        let full_hour_range = dashboard_activity_full_hour_exact_range(range_plan.full_hour_range)?
            .expect("non-empty full-hour epoch range should produce an exact range");
        let rollup_rows = crate::stats::query_invocation_hourly_rollup_range(
            &state.pool,
            range_start_epoch,
            range_end_epoch,
            source_scope,
        )
        .await?;
        let rollup_count = rollup_rows
            .iter()
            .map(|row| row.first_token_sample_count.max(0))
            .sum::<i64>();
        let rollup_sum = rollup_rows
            .iter()
            .map(|row| row.first_token_sum_ms.max(0.0))
            .sum::<f64>();
        let live_rows = query_live_upstream_account_activity_aggregate_rows(
            &state.pool,
            source_scope,
            full_hour_range,
            true,
            DashboardActivityExcludedInvocationIdsFilter::None,
        )
        .await?;
        let live_count = live_rows
            .iter()
            .map(|row| row.first_token_sample_count.max(0))
            .sum::<i64>();
        let live_sum = live_rows
            .iter()
            .map(|row| row.first_token_sum_ms.max(0.0))
            .sum::<f64>();
        sample_count += rollup_count.saturating_sub(live_count).max(0);
        sum_ms += (rollup_sum - live_sum).max(0.0);
    }
    Ok((sample_count, sum_ms))
}

fn dashboard_activity_historical_live_gap_ranges(
    range: ExactUtcRange,
    retention_cutoff: DateTime<Utc>,
) -> Result<Vec<ExactUtcRange>, ApiError> {
    if range.start >= retention_cutoff {
        return Ok(Vec::new());
    }

    let full_hour_start_epoch = ceil_hour_epoch(range.start.timestamp());
    let full_hour_end_epoch = crate::stats::align_bucket_epoch(range.end.timestamp(), 3_600, 0);
    let full_hour_start = Utc
        .timestamp_opt(full_hour_start_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid dashboard activity gap start epoch")))?;
    let full_hour_end = Utc
        .timestamp_opt(full_hour_end_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid dashboard activity gap end epoch")))?;

    let mut gap_ranges = Vec::new();
    if let Some(gap) = exact_utc_range(
        range.start,
        range.end.min(full_hour_start).min(retention_cutoff),
    )? {
        gap_ranges.push(gap);
    }
    if let Some(gap) = exact_utc_range(
        range.start.max(full_hour_end),
        range.end.min(retention_cutoff),
    )? {
        gap_ranges.push(gap);
    }
    Ok(gap_ranges)
}

async fn dashboard_activity_historical_live_gap_totals(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
    retention_cutoff: DateTime<Utc>,
) -> Result<StatsTotals, ApiError> {
    let mut totals = StatsTotals::default();
    for gap_range in dashboard_activity_historical_live_gap_ranges(range, retention_cutoff)? {
        totals = totals.add(dashboard_activity_stats_totals_from_aggregate_rows(
            &query_live_upstream_account_activity_aggregate_rows(
                &state.pool,
                source_scope,
                gap_range,
                true,
                DashboardActivityExcludedInvocationIdsFilter::None,
            )
            .await?,
        ));
    }
    Ok(totals)
}

fn dashboard_activity_apply_materialized_archive_fallback_to_stats(
    stats: &mut StatsResponse,
    fallback_totals: StatsTotals,
) {
    if !dashboard_activity_stats_totals_has_values(fallback_totals) {
        return;
    }

    stats.total_count += fallback_totals.total_count;
    stats.success_count += fallback_totals.success_count;
    stats.failure_count += fallback_totals.failure_count;
    stats.total_cost += fallback_totals.total_cost;
    stats.total_tokens += fallback_totals.total_tokens;
    stats.non_success_cost =
        Some(stats.non_success_cost.unwrap_or_default() + fallback_totals.non_success_cost);
    // Materialized invocation rollups do not retain model/cost-breakdown or non-success-token
    // detail, so omit those partial fields when they would no longer align with top-level totals.
    stats.usage_breakdown = None;
    stats.non_success_tokens = None;
}

fn dashboard_activity_clear_materialized_archive_detail_fields(stats: &mut StatsResponse) {
    stats.usage_breakdown = None;
    stats.non_success_tokens = None;
}

#[derive(Debug, FromRow)]
struct DashboardActivityAccountStatsRollupAggregateRow {
    upstream_account_id: i64,
    request_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_input_tokens: i64,
    total_cost: f64,
    non_success_cost: f64,
    first_response_byte_total_sample_count: i64,
    first_response_byte_total_sum_ms: f64,
    first_token_sample_count: i64,
    first_token_sum_ms: f64,
    total_latency_sample_count: i64,
    total_latency_sum_ms: f64,
}

#[derive(Debug, Default, Clone, Copy)]
struct DashboardActivityUsageFallbackTotals {
    cache_write_tokens: i64,
    cache_read_tokens: i64,
    output_tokens: i64,
}

#[derive(Debug, Default, Clone, Copy)]
struct DashboardActivityAccountFallbackTotals {
    request_count: i64,
    success_count: i64,
    failure_count: i64,
    non_success_count: i64,
    total_tokens: i64,
    success_tokens: i64,
    non_success_tokens: i64,
    failure_tokens: i64,
    failure_cost: f64,
    non_success_cost: f64,
    cache_input_tokens: i64,
    total_cost: f64,
    cache_write_tokens: i64,
    cache_read_tokens: i64,
    output_tokens: i64,
    first_response_byte_total_sample_count: i64,
    first_response_byte_total_sum_ms: f64,
    first_token_sample_count: i64,
    first_token_sum_ms: f64,
    total_latency_sample_count: i64,
    total_latency_sum_ms: f64,
}

impl DashboardActivityAccountFallbackTotals {
    fn from_stats_totals(totals: StatsTotals) -> Self {
        let non_success_count = totals
            .total_count
            .saturating_sub(totals.success_count)
            .max(0);
        Self {
            request_count: totals.total_count,
            success_count: totals.success_count,
            failure_count: totals.failure_count,
            non_success_count: non_success_count.max(totals.failure_count),
            total_tokens: totals.total_tokens,
            success_tokens: if non_success_count == 0 {
                totals.total_tokens
            } else {
                0
            },
            non_success_tokens: if totals.success_count == 0 {
                totals.total_tokens
            } else {
                0
            },
            failure_tokens: if totals.success_count == 0 && totals.failure_count > 0 {
                totals.total_tokens
            } else {
                0
            },
            failure_cost: if totals.success_count == 0 && totals.failure_count > 0 {
                totals.non_success_cost
            } else {
                0.0
            },
            non_success_cost: totals.non_success_cost,
            total_cost: totals.total_cost,
            ..Self::default()
        }
    }

    fn from_rollup_minus_live(
        row: &DashboardActivityAccountStatsRollupAggregateRow,
        live: Option<&UpstreamAccountActivityAggregateRow>,
        live_usage: DashboardActivityUsageFallbackTotals,
    ) -> Self {
        let live_request_count = live.map_or(0, |row| row.request_count);
        let live_success_count = live.map_or(0, |row| row.success_count);
        let live_failure_count = live.map_or(0, |row| row.failure_count);
        let live_total_tokens = live.map_or(0, |row| row.total_tokens);
        let live_cache_input_tokens = live.map_or(0, |row| row.cache_input_tokens);
        let live_total_cost = live.map_or(0.0, |row| row.total_cost);
        let live_non_success_cost = live.map_or(0.0, |row| row.non_success_cost);
        let live_first_response_count =
            live.map_or(0, |row| row.first_response_byte_total_sample_count);
        let live_first_response_sum = live.map_or(0.0, |row| row.first_response_byte_total_sum_ms);
        let live_first_token_count = live.map_or(0, |row| row.first_token_sample_count);
        let live_first_token_sum = live.map_or(0.0, |row| row.first_token_sum_ms);
        let live_total_latency_count = live.map_or(0, |row| row.total_latency_sample_count);
        let live_total_latency_sum = live.map_or(0.0, |row| row.total_latency_sum_ms);

        let request_count = row.request_count.saturating_sub(live_request_count).max(0);
        let success_count = row.success_count.saturating_sub(live_success_count).max(0);
        let failure_count = row.failure_count.saturating_sub(live_failure_count).max(0);
        let non_success_count = request_count
            .saturating_sub(success_count)
            .max(failure_count);
        let total_tokens = row.total_tokens.saturating_sub(live_total_tokens).max(0);
        let non_success_cost = (row.non_success_cost - live_non_success_cost).max(0.0);
        let total_cost = (row.total_cost - live_total_cost).max(0.0);
        let cache_input_tokens = row
            .cache_input_tokens
            .saturating_sub(live_cache_input_tokens)
            .max(0);
        let rollup_cache_write_tokens = row
            .input_tokens
            .max(0)
            .saturating_sub(row.cache_input_tokens.max(0))
            .max(0);
        let cache_write_tokens = rollup_cache_write_tokens
            .saturating_sub(live_usage.cache_write_tokens)
            .max(0);
        let cache_read_tokens = row
            .cache_input_tokens
            .max(0)
            .saturating_sub(live_usage.cache_read_tokens)
            .max(0);
        let output_tokens = row
            .output_tokens
            .max(0)
            .saturating_sub(live_usage.output_tokens)
            .max(0);

        Self {
            request_count,
            success_count,
            failure_count,
            non_success_count,
            total_tokens,
            success_tokens: if non_success_count == 0 {
                total_tokens
            } else {
                0
            },
            non_success_tokens: if success_count == 0 { total_tokens } else { 0 },
            failure_tokens: if success_count == 0 && failure_count > 0 {
                total_tokens
            } else {
                0
            },
            failure_cost: if success_count == 0 && failure_count > 0 {
                non_success_cost
            } else {
                0.0
            },
            non_success_cost,
            cache_input_tokens,
            total_cost,
            cache_write_tokens,
            cache_read_tokens,
            output_tokens,
            first_response_byte_total_sample_count: row
                .first_response_byte_total_sample_count
                .saturating_sub(live_first_response_count)
                .max(0),
            first_response_byte_total_sum_ms: (row.first_response_byte_total_sum_ms
                - live_first_response_sum)
                .max(0.0),
            first_token_sample_count: row
                .first_token_sample_count
                .saturating_sub(live_first_token_count)
                .max(0),
            first_token_sum_ms: (row.first_token_sum_ms - live_first_token_sum).max(0.0),
            total_latency_sample_count: row
                .total_latency_sample_count
                .saturating_sub(live_total_latency_count)
                .max(0),
            total_latency_sum_ms: (row.total_latency_sum_ms - live_total_latency_sum).max(0.0),
        }
    }

    fn add_assign(&mut self, other: Self) {
        self.request_count += other.request_count;
        self.success_count += other.success_count;
        self.failure_count += other.failure_count;
        self.non_success_count += other.non_success_count;
        self.total_tokens += other.total_tokens;
        self.success_tokens += other.success_tokens;
        self.non_success_tokens += other.non_success_tokens;
        self.failure_tokens += other.failure_tokens;
        self.failure_cost += other.failure_cost;
        self.non_success_cost += other.non_success_cost;
        self.cache_input_tokens += other.cache_input_tokens;
        self.total_cost += other.total_cost;
        self.cache_write_tokens += other.cache_write_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.output_tokens += other.output_tokens;
        self.first_response_byte_total_sample_count += other.first_response_byte_total_sample_count;
        self.first_response_byte_total_sum_ms += other.first_response_byte_total_sum_ms;
        self.first_token_sample_count += other.first_token_sample_count;
        self.first_token_sum_ms += other.first_token_sum_ms;
        self.total_latency_sample_count += other.total_latency_sample_count;
        self.total_latency_sum_ms += other.total_latency_sum_ms;
    }

    fn stats_totals(self) -> StatsTotals {
        StatsTotals {
            total_count: self.request_count,
            success_count: self.success_count,
            failure_count: self.failure_count,
            total_cost: self.total_cost,
            total_tokens: self.total_tokens,
            non_success_cost: self.non_success_cost,
        }
    }
}

fn dashboard_activity_residual_first_token_totals(
    global_sample_count: i64,
    global_sum_ms: f64,
    account_totals: &HashMap<Option<i64>, DashboardActivityAccountFallbackTotals>,
) -> (i64, f64) {
    let account_sample_count = account_totals
        .values()
        .map(|totals| totals.first_token_sample_count)
        .sum::<i64>();
    let account_sum_ms = account_totals
        .values()
        .map(|totals| totals.first_token_sum_ms)
        .sum::<f64>();
    (
        global_sample_count
            .saturating_sub(account_sample_count)
            .max(0),
        (global_sum_ms - account_sum_ms).max(0.0),
    )
}

async fn query_dashboard_activity_account_stats_rollup_aggregate_rows(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: ExactUtcRange,
) -> Result<Vec<DashboardActivityAccountStatsRollupAggregateRow>, ApiError> {
    let range_start_epoch = ceil_hour_epoch(range.start.timestamp());
    let range_end_epoch = crate::stats::align_bucket_epoch(range.end.timestamp(), 3_600, 0);
    if range_start_epoch >= range_end_epoch {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            upstream_account_id,
            COALESCE(SUM(total_count), 0) AS request_count,
            COALESCE(SUM(success_count), 0) AS success_count,
            COALESCE(SUM(failure_count), 0) AS failure_count,
            COALESCE(SUM(total_tokens), 0) AS total_tokens,
            COALESCE(SUM(input_tokens), 0) AS input_tokens,
            COALESCE(SUM(output_tokens), 0) AS output_tokens,
            COALESCE(SUM(cache_input_tokens), 0) AS cache_input_tokens,
            CAST(COALESCE(SUM(total_cost), 0.0) AS REAL) AS total_cost,
            CAST(COALESCE(SUM(non_success_cost), 0.0) AS REAL) AS non_success_cost,
            COALESCE(SUM(first_response_byte_total_sample_count), 0) AS first_response_byte_total_sample_count,
            CAST(COALESCE(SUM(first_response_byte_total_sum_ms), 0.0) AS REAL) AS first_response_byte_total_sum_ms,
            COALESCE(SUM(first_token_sample_count), 0) AS first_token_sample_count,
            CAST(COALESCE(SUM(first_token_sum_ms), 0.0) AS REAL) AS first_token_sum_ms,
            COALESCE(SUM(total_latency_sample_count), 0) AS total_latency_sample_count,
            CAST(COALESCE(SUM(total_latency_sum_ms), 0.0) AS REAL) AS total_latency_sum_ms
        FROM upstream_account_stats_hourly
        WHERE bucket_start_epoch >=
        "#,
    );
    query
        .push_bind(range_start_epoch)
        .push(" AND bucket_start_epoch < ")
        .push_bind(range_end_epoch);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" GROUP BY upstream_account_id");
    Ok(query
        .build_query_as::<DashboardActivityAccountStatsRollupAggregateRow>()
        .fetch_all(pool)
        .await?)
}

async fn dashboard_activity_materialized_archive_account_fallback_totals(
    state: &AppState,
    source_scope: InvocationSourceScope,
    skipped_materialized_ranges: &[ExactUtcRange],
) -> Result<HashMap<Option<i64>, DashboardActivityAccountFallbackTotals>, ApiError> {
    let mut fallback_by_account =
        HashMap::<Option<i64>, DashboardActivityAccountFallbackTotals>::new();
    let retention_cutoff = shanghai_retention_cutoff(state.config.invocation_max_days);
    for skipped_range in skipped_materialized_ranges {
        let range_plan = build_hourly_rollup_exact_range_plan(
            skipped_range.start,
            skipped_range.end,
            retention_cutoff,
        )?;
        if let Some(full_hour_range) =
            dashboard_activity_full_hour_exact_range(range_plan.full_hour_range)?
        {
            let rollup_rows = query_dashboard_activity_account_stats_rollup_aggregate_rows(
                &state.pool,
                source_scope,
                full_hour_range,
            )
            .await?;
            let live_full_hour_rows = query_live_upstream_account_activity_aggregate_rows(
                &state.pool,
                source_scope,
                full_hour_range,
                true,
                DashboardActivityExcludedInvocationIdsFilter::None,
            )
            .await?;
            let live_full_hour_by_account = live_full_hour_rows
                .into_iter()
                .map(|row| (row.upstream_account_id, row))
                .collect::<HashMap<_, _>>();
            let mut live_full_hour_usage_by_account =
                HashMap::<Option<i64>, DashboardActivityUsageFallbackTotals>::new();
            for row in query_live_upstream_account_usage_breakdown_rows(
                &state.pool,
                source_scope,
                full_hour_range,
                true,
                true,
                DashboardActivityExcludedInvocationIdsFilter::None,
            )
            .await?
            {
                let entry = live_full_hour_usage_by_account
                    .entry(row.upstream_account_id)
                    .or_default();
                entry.cache_write_tokens += row.cache_write_tokens;
                entry.cache_read_tokens += row.cache_read_tokens;
                entry.output_tokens += row.output_tokens;
            }

            for row in rollup_rows {
                let account_id = Some(row.upstream_account_id);
                let totals = DashboardActivityAccountFallbackTotals::from_rollup_minus_live(
                    &row,
                    live_full_hour_by_account.get(&account_id),
                    live_full_hour_usage_by_account
                        .get(&account_id)
                        .copied()
                        .unwrap_or_default(),
                );
                if !dashboard_activity_stats_totals_has_values(totals.stats_totals()) {
                    continue;
                }
                fallback_by_account
                    .entry(account_id)
                    .or_default()
                    .add_assign(totals);
            }
        }
    }
    Ok(fallback_by_account)
}

fn dashboard_activity_merge_account_fallback_totals(
    entry: &mut UpstreamAccountActivityAccumulator,
    totals: DashboardActivityAccountFallbackTotals,
) {
    entry.request_count += totals.request_count;
    entry.success_count += totals.success_count;
    entry.failure_count += totals.failure_count;
    entry.non_success_count += totals.non_success_count;
    entry.total_tokens += totals.total_tokens;
    entry.success_tokens += totals.success_tokens;
    entry.non_success_tokens += totals.non_success_tokens;
    entry.failure_tokens += totals.failure_tokens;
    entry.failure_cost += totals.failure_cost;
    entry.non_success_cost += totals.non_success_cost;
    entry.cache_input_tokens += totals.cache_input_tokens;
    entry.total_cost += totals.total_cost;
    entry.first_response_byte_total_sample_count += totals.first_response_byte_total_sample_count;
    entry.first_response_byte_total_sum_ms += totals.first_response_byte_total_sum_ms;
    entry.first_token_sample_count += totals.first_token_sample_count;
    entry.first_token_sum_ms += totals.first_token_sum_ms;
    entry.total_latency_sample_count += totals.total_latency_sample_count;
    entry.total_latency_sum_ms += totals.total_latency_sum_ms;
    entry.usage_breakdown.add_coarse_rollup_totals(
        totals.cache_write_tokens,
        totals.cache_read_tokens,
        totals.output_tokens,
        totals.total_cost,
    );
}

fn dashboard_activity_source_scope_cache_key(source_scope: InvocationSourceScope) -> &'static str {
    match source_scope {
        InvocationSourceScope::ProxyOnly => "proxy_only",
        InvocationSourceScope::All => "all",
    }
}
