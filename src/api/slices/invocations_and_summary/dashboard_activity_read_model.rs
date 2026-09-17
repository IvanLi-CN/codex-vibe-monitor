fn dashboard_live_snapshot_in_progress_counts(
    snapshot: &DashboardActivityLiveSnapshot,
) -> HashMap<Option<i64>, UpstreamAccountInProgressSummary> {
    snapshot
        .accounts
        .iter()
        .map(|account| {
            (
                account.upstream_account_id,
                UpstreamAccountInProgressSummary {
                    in_progress_count: account.in_progress_invocation_count,
                    retry_count: account.retry_invocation_count,
                    phase_counts: account.in_progress_phase_counts,
                    wait_sum_ms: account.in_progress_wait_sum_ms,
                    wait_sample_count: account.in_progress_wait_sample_count,
                },
            )
        })
        .collect()
}

pub(crate) const DASHBOARD_ACTIVITY_RATE_WINDOW_MINUTES: i64 = 5;
/// DB-backed Dashboard baselines are reconciled at a bounded cadence. Terminal records are
/// layered onto the cached baseline synchronously, so this is not an owner-visible freshness
/// budget.
pub(crate) const DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS: u64 = 60;
const DASHBOARD_ACTIVITY_READ_MODEL_MAX_TERMINAL_KEYS: usize = 50_000;
const DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_TERMINALS: usize = 10_000;
const DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;
const DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_MAX_ENTRIES: usize = 64;
const DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_MAX_EXPIRY_BYTES: usize = 64 * 1024 * 1024;
const DASHBOARD_ACTIVITY_LAST_GOOD_MAX_AGE: Duration = Duration::from_secs(15 * 60);
const DASHBOARD_ACTIVITY_PRESSURE_RECONCILE_MAX_AGE: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone)]
pub(crate) struct DashboardActivitySnapshot {
    range: String,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    terminal_sequence: u64,
    accounts: Vec<DashboardActivityAccountResponse>,
    summary: DashboardActivitySummaryResponse,
    summary_model_performance_accumulator: ModelPerformanceAccumulator,
    account_model_performance_accumulators: HashMap<Option<i64>, ModelPerformanceAccumulator>,
    account_latency_accumulators: HashMap<Option<i64>, DashboardActivityAccountLatencyAccumulator>,
    model_performance_accumulator_ready: bool,
    materialized_archive_fallback_totals: StatsTotals,
    materialized_archive_details_limited: bool,
    build_telemetry: DashboardActivityBuildTelemetry,
}

/// Stateful typed base used only by Auto dashboard topic delivery. It retains the aggregate
/// inputs that are intentionally absent from the public response so terminal slices can preserve
/// the existing model-performance, latency, and recent-invocation response semantics.
#[derive(Debug)]
pub(crate) struct DashboardActivityTopicMaterializedBase {
    response: DashboardActivityResponse,
    summary_model_performance_accumulator: ModelPerformanceAccumulator,
    account_model_performance_accumulators: HashMap<Option<i64>, ModelPerformanceAccumulator>,
    account_latency_accumulators: HashMap<Option<i64>, DashboardActivityAccountLatencyAccumulator>,
    model_performance_accumulator_ready: bool,
    recent_limit: usize,
    include_recent: bool,
}

impl DashboardActivityTopicMaterializedBase {
    pub(crate) fn response(&self) -> &DashboardActivityResponse {
        &self.response
    }

    pub(crate) fn response_mut(&mut self) -> &mut DashboardActivityResponse {
        &mut self.response
    }

    pub(crate) fn apply_routing_rule_change(
        &mut self,
        version: &crate::upstream_accounts::RoutingStateVersion,
        upserts: &[(i64, crate::upstream_accounts::EffectiveRoutingRule)],
        removed_account_ids: &[i64],
    ) -> bool {
        if let Some(current) = self.response.routing_state_version.as_ref()
            && (current.ordering() == version.ordering() || !version.is_same_or_newer_than(current))
        {
            return false;
        }
        if let Some(accounts) = self.response.accounts.as_mut() {
            accounts.retain(|account| {
                account
                    .upstream_account_id
                    .is_none_or(|account_id| !removed_account_ids.contains(&account_id))
            });
            for (account_id, rule) in upserts {
                if let Some(account) = accounts
                    .iter_mut()
                    .find(|account| account.upstream_account_id == Some(*account_id))
                {
                    account.effective_routing_rule = Some(rule.clone());
                }
            }
        }
        self.response.routing_state_version = Some(version.clone());
        true
    }

    pub(crate) fn apply_terminal_slice(
        &mut self,
        reporting_tz: Tz,
        source_scope: InvocationSourceScope,
        slice: &DashboardTerminalProjectionSlice,
    ) {
        let Ok(range) = resolve_dashboard_activity_cached_range(&self.response.range, reporting_tz)
        else {
            return;
        };
        let deltas = slice
            .deltas
            .iter()
            .filter(|delta| {
                dashboard_activity_terminal_delta_matches_source_scope(delta, source_scope)
                    && delta.terminal_sequence > self.response.terminal_sequence
                    && dashboard_activity_terminal_delta_is_within_range(delta, range)
            })
            .collect::<Vec<_>>();
        if deltas.is_empty() {
            return;
        }

        self.response.range_start = format_utc_iso_precise(range.start);
        self.response.range_end = format_utc_iso_precise(range.end);
        self.response.snapshot_id = range.end.timestamp_millis();
        let model_performance_available = self.response.summary.model_performance.available;
        for delta in &deltas {
            apply_dashboard_activity_terminal_delta_to_stats(
                &mut self.response.summary.stats,
                delta,
            );
            if self.model_performance_accumulator_ready {
                self.summary_model_performance_accumulator
                    .add_terminal_delta(delta);
            }
            self.response.terminal_sequence =
                self.response.terminal_sequence.max(delta.terminal_sequence);
        }
        if self.model_performance_accumulator_ready {
            self.response.summary.model_performance = self
                .summary_model_performance_accumulator
                .clone()
                .into_response(range, model_performance_available);
        }

        self.apply_terminal_slice_to_accounts(range, model_performance_available, &deltas);
    }

    fn apply_terminal_slice_to_accounts(
        &mut self,
        range: ExactUtcRange,
        model_performance_available: bool,
        deltas: &[&DashboardActivityTerminalDelta],
    ) {
        let Some(accounts) = self.response.accounts.as_mut() else {
            return;
        };
        let mut account_indexes = accounts
            .iter()
            .enumerate()
            .map(|(index, account)| (account.upstream_account_id, index))
            .collect::<HashMap<_, _>>();
        let mut updated_account_ids = HashSet::new();
        for delta in deltas {
            let account_index = if let Some(index) = account_indexes.get(&delta.upstream_account_id)
            {
                *index
            } else {
                accounts.push(dashboard_activity_terminal_account_for_range(range, delta));
                let index = accounts.len() - 1;
                account_indexes.insert(delta.upstream_account_id, index);
                index
            };

            if self.model_performance_accumulator_ready {
                let accumulator = self
                    .account_model_performance_accumulators
                    .entry(delta.upstream_account_id)
                    .or_default();
                accumulator.add_terminal_delta(delta);
            }
            self.account_latency_accumulators
                .entry(delta.upstream_account_id)
                .or_default()
                .add_terminal_delta(delta);
            let account = &mut accounts[account_index];
            apply_dashboard_activity_terminal_delta_to_account(account, delta);
            if self.include_recent {
                account.recent_invocations = merge_dashboard_activity_recent_invocations(
                    vec![delta.recent_invocation.clone()],
                    std::mem::take(&mut account.recent_invocations),
                    self.recent_limit,
                );
            }
            updated_account_ids.insert(delta.upstream_account_id);
        }
        for upstream_account_id in updated_account_ids {
            let account = &mut accounts[*account_indexes
                .get(&upstream_account_id)
                .expect("terminal account index inserted when absent")];
            self.account_latency_accumulators
                .get(&upstream_account_id)
                .expect("terminal latency accumulator inserted")
                .apply_to_account(account);
            if self.model_performance_accumulator_ready {
                account.model_performance = self
                    .account_model_performance_accumulators
                    .get(&upstream_account_id)
                    .expect("terminal model-performance accumulator inserted")
                    .clone()
                    .into_response(range, model_performance_available);
            }
        }
        sort_dashboard_activity_accounts(accounts);
    }
}

#[cfg(test)]
impl DashboardActivitySnapshot {
    pub(crate) fn exact_range(&self) -> ExactUtcRange {
        ExactUtcRange {
            start: self.range_start,
            end: self.range_end,
        }
    }

    pub(crate) fn accounts(&self) -> &[DashboardActivityAccountResponse] {
        &self.accounts
    }

    pub(crate) fn summary(&self) -> &DashboardActivitySummaryResponse {
        &self.summary
    }

    #[cfg(test)]
    pub(crate) fn test_stub(range: &str) -> Self {
        let now = Utc::now();
        Self {
            range: range.to_string(),
            range_start: now,
            range_end: now + ChronoDuration::minutes(1),
            terminal_sequence: 0,
            accounts: Vec::new(),
            summary: DashboardActivitySummaryResponse {
                stats: StatsResponse {
                    total_count: 0,
                    success_count: 0,
                    failure_count: 0,
                    total_cost: 0.0,
                    total_tokens: 0,
                    usage_breakdown: None,
                    in_progress_conversation_count: None,
                    in_progress_retry_conversation_count: None,
                    in_progress_avg_wait_ms: None,
                    in_progress_phase_counts: None,
                    non_success_cost: None,
                    non_success_tokens: None,
                    maintenance: None,
                },
                tokens_per_minute: None,
                spend_rate: None,
                current_first_response_byte_total_avg_ms: None,
                current_first_token_avg_ms: None,
                current_avg_total_ms: None,
                current_avg_response_ms: None,
                model_performance: ModelPerformanceResponse {
                    available: false,
                    total: ModelPerformanceMetricsResponse {
                        tokens_per_minute: 0.0,
                        streaming_response_rate: None,
                        avg_response_ms: None,
                        avg_first_response_byte_total_ms: None,
                        avg_first_token_ms: None,
                        wall_clock_usage_duration_ms: None,
                        cumulative_usage_duration_ms: None,
                        parallelism: None,
                    },
                    models: Vec::new(),
                },
            },
            summary_model_performance_accumulator: ModelPerformanceAccumulator::default(),
            account_model_performance_accumulators: HashMap::new(),
            account_latency_accumulators: HashMap::new(),
            model_performance_accumulator_ready: true,
            materialized_archive_fallback_totals: StatsTotals::default(),
            materialized_archive_details_limited: false,
            build_telemetry: DashboardActivityBuildTelemetry::default(),
        }
    }
}

fn option_string_capacity(value: &Option<String>) -> usize {
    value.as_ref().map_or(0, String::capacity)
}

fn usage_breakdown_memory_bytes(value: &UsageBreakdownResponse) -> usize {
    let model_bytes = value
        .models
        .iter()
        .map(|model| model.model.capacity() + option_string_capacity(&model.reasoning_effort))
        .sum::<usize>();
    value
        .models
        .capacity()
        .saturating_mul(std::mem::size_of::<UsageBreakdownModelResponse>())
        .saturating_add(model_bytes)
}

fn model_performance_memory_bytes(value: &ModelPerformanceResponse) -> usize {
    let model_bytes = value
        .models
        .iter()
        .map(|model| model.model.capacity() + option_string_capacity(&model.reasoning_effort))
        .sum::<usize>();
    value
        .models
        .capacity()
        .saturating_mul(std::mem::size_of::<ModelPerformanceModelResponse>())
        .saturating_add(model_bytes)
}

fn dashboard_activity_account_memory_bytes(value: &DashboardActivityAccountResponse) -> usize {
    value.account_key.capacity()
        + value.display_name.capacity()
        + option_string_capacity(&value.latest_conversation_created_at)
        + option_string_capacity(&value.last_invocation_at)
        + option_string_capacity(&value.group_name)
        + option_string_capacity(&value.plan_type)
        + option_string_capacity(&value.display_status)
        + option_string_capacity(&value.enable_status)
        + option_string_capacity(&value.work_status)
        + option_string_capacity(&value.health_status)
        + option_string_capacity(&value.sync_state)
        + option_string_capacity(&value.last_error)
        + option_string_capacity(&value.last_action_reason_message)
        + value
            .recent_invocations
            .capacity()
            .saturating_mul(std::mem::size_of::<
                PromptCacheConversationInvocationPreviewResponse,
            >())
        + usage_breakdown_memory_bytes(&value.usage_breakdown)
        + model_performance_memory_bytes(&value.model_performance)
}

pub(crate) fn dashboard_activity_snapshot_memory_estimate(
    snapshot: &DashboardActivitySnapshot,
) -> usize {
    let account_bytes = snapshot
        .accounts
        .iter()
        .map(dashboard_activity_account_memory_bytes)
        .sum::<usize>();
    let summary = &snapshot.summary;
    let summary_usage_bytes = summary
        .stats
        .usage_breakdown
        .as_ref()
        .map(usage_breakdown_memory_bytes)
        .unwrap_or_default();
    let summary_bytes = std::mem::size_of_val(summary)
        + summary_usage_bytes
        + model_performance_memory_bytes(&summary.model_performance);
    std::mem::size_of_val(snapshot)
        .saturating_add(
            snapshot
                .accounts
                .capacity()
                .saturating_mul(std::mem::size_of::<DashboardActivityAccountResponse>()),
        )
        .saturating_add(account_bytes)
        .saturating_add(summary_bytes)
        .saturating_add(
            snapshot
                .account_model_performance_accumulators
                .capacity()
                .saturating_mul(std::mem::size_of::<(
                    Option<i64>,
                    ModelPerformanceAccumulator,
                )>()),
        )
        .saturating_add(
            snapshot
                .account_latency_accumulators
                .capacity()
                .saturating_mul(std::mem::size_of::<(
                    Option<i64>,
                    DashboardActivityAccountLatencyAccumulator,
                )>()),
        )
}

#[derive(Debug, Clone, Copy, Default)]
struct DashboardActivityAccountLatencyAccumulator {
    first_response_byte_total_sample_count: i64,
    first_response_byte_total_sum_ms: f64,
    first_token_sample_count: i64,
    first_token_sum_ms: f64,
    total_latency_sample_count: i64,
    total_latency_sum_ms: f64,
}

impl DashboardActivityAccountLatencyAccumulator {
    fn from_aggregate(aggregate: &UpstreamAccountActivityAccumulator) -> Self {
        Self {
            first_response_byte_total_sample_count: aggregate
                .first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms: aggregate.first_response_byte_total_sum_ms,
            first_token_sample_count: aggregate.first_token_sample_count,
            first_token_sum_ms: aggregate.first_token_sum_ms,
            total_latency_sample_count: aggregate.total_latency_sample_count,
            total_latency_sum_ms: aggregate.total_latency_sum_ms,
        }
    }

    fn add_terminal_delta(&mut self, delta: &DashboardActivityTerminalDelta) {
        if let Some(first_token_ms) = delta
            .first_token_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.first_token_sample_count += 1;
            self.first_token_sum_ms += first_token_ms;
        }
        if !delta.success {
            return;
        }
        if delta
            .t_upstream_ttfb_ms
            .filter(|value| value.is_finite() && *value > 0.0)
            .is_some()
            && let Some(first_response_byte_total_ms) =
                crate::stats::resolve_first_response_byte_total_ms(
                    delta.t_req_read_ms,
                    delta.t_req_parse_ms,
                    delta.t_upstream_connect_ms,
                    delta.t_upstream_ttfb_ms,
                )
        {
            self.first_response_byte_total_sample_count += 1;
            self.first_response_byte_total_sum_ms += first_response_byte_total_ms;
        }
        if let Some(total_latency_ms) = delta
            .t_total_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.total_latency_sample_count += 1;
            self.total_latency_sum_ms += total_latency_ms;
        }
    }

    fn subtract_terminal_delta(&mut self, delta: &DashboardActivityTerminalDelta) {
        if let Some(value) = delta
            .first_token_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.first_token_sample_count = self.first_token_sample_count.saturating_sub(1);
            self.first_token_sum_ms = (self.first_token_sum_ms - value).max(0.0);
        }
        if !delta.success {
            return;
        }
        if delta
            .t_upstream_ttfb_ms
            .is_some_and(|value| value.is_finite() && value > 0.0)
            && let Some(value) = crate::stats::resolve_first_response_byte_total_ms(
                delta.t_req_read_ms,
                delta.t_req_parse_ms,
                delta.t_upstream_connect_ms,
                delta.t_upstream_ttfb_ms,
            )
        {
            self.first_response_byte_total_sample_count = self
                .first_response_byte_total_sample_count
                .saturating_sub(1);
            self.first_response_byte_total_sum_ms =
                (self.first_response_byte_total_sum_ms - value).max(0.0);
        }
        if let Some(value) = delta
            .t_total_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.total_latency_sample_count = self.total_latency_sample_count.saturating_sub(1);
            self.total_latency_sum_ms = (self.total_latency_sum_ms - value).max(0.0);
        }
    }

    fn apply_to_account(self, account: &mut DashboardActivityAccountResponse) {
        let first_response_byte_total_avg_ms = (self.first_response_byte_total_sample_count > 0)
            .then_some(
                self.first_response_byte_total_sum_ms
                    / self.first_response_byte_total_sample_count as f64,
            );
        account.first_byte_avg_ms = first_response_byte_total_avg_ms;
        account.first_response_byte_total_avg_ms = first_response_byte_total_avg_ms;
        account.first_token_avg_ms = (self.first_token_sample_count > 0)
            .then_some(self.first_token_sum_ms / self.first_token_sample_count as f64);
        account.avg_total_ms = (self.total_latency_sample_count > 0)
            .then_some(self.total_latency_sum_ms / self.total_latency_sample_count as f64);
    }
}

#[derive(Debug, Clone, Copy)]
struct DashboardActivitySnapshotCacheOutcome {
    cache_hit_or_miss: &'static str,
    cache_bypass_reason: &'static str,
    coalesced_waiter_count: usize,
    db_build_elapsed_ms: u64,
    cache_ttl_ms: u64,
    cache_entry_age_ms: u64,
    cache_entry_count: usize,
    in_flight_count: usize,
    refresh_reason: &'static str,
    selection_fingerprint: u64,
    terminal_delta_count: u64,
    duplicate_delta_count: u64,
    pending_delta_count: usize,
    pending_delta_estimated_bytes: usize,
    persisted_ack_pending_count: usize,
    delta_pruned_count: u64,
    expiry_delta_count: usize,
    hard_limit_reason: &'static str,
    baseline_cursor: i64,
    sequence_gap_count: u64,
    build_attempted: bool,
    snapshot_origin: &'static str,
}

fn dashboard_activity_read_model_state(
    outcome: DashboardActivitySnapshotCacheOutcome,
) -> &'static str {
    if outcome.hard_limit_reason != "none" || outcome.sequence_gap_count > 0 {
        return "dirty_last_good";
    }
    match outcome.cache_hit_or_miss {
        "last_good_fallback" => "stale_last_good",
        "cache_miss_build" => "reconciled",
        "uncached" => "exact_closed_range",
        _ => "healthy",
    }
}

#[derive(Debug, Clone, Copy)]
struct DashboardActivityBuildTelemetry {
    preview_read_mode: &'static str,
    candidate_preview_id_count: usize,
    hydrated_preview_row_count: usize,
    account_aggregation: AccountActivityRangeBuildTelemetry,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct DashboardActivityTerminalDeltaOutcome {
    pub(crate) applied_selection_count: usize,
    pub(crate) duplicate: bool,
    pub(crate) skipped_out_of_range_count: usize,
    pub(crate) hard_limit_reason: Option<&'static str>,
    pub(crate) terminal_sequence: Option<u64>,
    pub(crate) terminal_delta: Option<DashboardActivityTerminalDelta>,
}

fn dashboard_activity_selection_includes_compact_terminal(
    selection: &DashboardActivitySnapshotSelection,
    delta: &DashboardActivityTerminalDelta,
    occurred_at: DateTime<Utc>,
) -> bool {
    if !matches!(selection.range.as_str(), "today" | "1d" | "7d")
        || (selection.source_scope == "proxy_only" && delta.source != SOURCE_PROXY)
    {
        return false;
    }
    let Ok(reporting_tz) = selection.time_zone.parse::<Tz>() else {
        return false;
    };
    let Ok(range) = resolve_dashboard_activity_exact_range(&selection.range, reporting_tz) else {
        return false;
    };
    occurred_at >= range.start && occurred_at < range.end
}

fn dashboard_activity_selection_includes_terminal(
    selection: &DashboardActivitySnapshotSelection,
    record: &ApiInvocation,
    occurred_at: DateTime<Utc>,
) -> bool {
    dashboard_activity_selection_includes_compact_terminal(
        selection,
        &dashboard_activity_terminal_delta(record),
        occurred_at,
    )
}

fn dashboard_activity_selection_source_scope(
    selection: &DashboardActivitySnapshotSelection,
) -> InvocationSourceScope {
    if selection.source_scope == "proxy_only" {
        InvocationSourceScope::ProxyOnly
    } else {
        InvocationSourceScope::All
    }
}

fn dashboard_activity_entry_includes_terminal(
    entry: &DashboardActivitySnapshotCacheEntry,
    delta: &DashboardActivityTerminalDelta,
) -> bool {
    delta
        .persisted_row_id
        .is_some_and(|row_id| row_id <= entry.baseline_snapshot_cursor)
}

fn dashboard_activity_baseline_includes_pending_delta(
    delta: &DashboardActivityTerminalDelta,
    baseline_cursor: i64,
    persisted_terminal_ids: &HashMap<(String, String), i64>,
) -> bool {
    delta
        .persisted_row_id
        .is_some_and(|row_id| row_id <= baseline_cursor)
        || persisted_terminal_ids.contains_key(&delta.key())
}

fn replay_dashboard_activity_pending_deltas_without_expiry(
    snapshot: &mut DashboardActivitySnapshot,
    selection: &DashboardActivitySnapshotSelection,
    pending_terminal_deltas: &VecDeque<DashboardActivityTerminalDelta>,
    baseline_cursor: i64,
    persisted_terminal_ids: &HashMap<(String, String), i64>,
) -> usize {
    let mut replayed = 0usize;
    for delta in pending_terminal_deltas {
        let Some(occurred_at) = parse_to_utc_datetime(&delta.occurred_at) else {
            continue;
        };
        if dashboard_activity_selection_includes_compact_terminal(selection, delta, occurred_at) {
            if dashboard_activity_baseline_includes_pending_delta(
                delta,
                baseline_cursor,
                persisted_terminal_ids,
            ) {
                snapshot.terminal_sequence =
                    snapshot.terminal_sequence.max(delta.terminal_sequence);
            } else {
                apply_dashboard_activity_compact_terminal_delta(snapshot, delta);
                replayed += 1;
            }
        }
    }
    replayed
}
