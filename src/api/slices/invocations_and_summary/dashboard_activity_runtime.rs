use super::*;
pub(crate) const DASHBOARD_ACTIVITY_RATE_WINDOW_MINUTES: i64 = 5;
/// DB-backed Dashboard baselines are reconciled at a bounded cadence. Terminal records are
/// layered onto the cached baseline synchronously, so this is not an owner-visible freshness
/// budget.
pub(crate) const DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS: u64 = 60;
pub(crate) const DASHBOARD_ACTIVITY_READ_MODEL_MAX_TERMINAL_KEYS: usize = 50_000;
pub(crate) const DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_TERMINALS: usize = 10_000;
pub(crate) const DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_MAX_ENTRIES: usize = 64;
pub(crate) const DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_MAX_EXPIRY_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const DASHBOARD_ACTIVITY_LAST_GOOD_MAX_AGE: Duration = Duration::from_secs(15 * 60);
pub(crate) const DASHBOARD_ACTIVITY_PRESSURE_RECONCILE_MAX_AGE: Duration =
    Duration::from_secs(5 * 60);

#[derive(Debug, Clone)]
pub(crate) struct DashboardActivitySnapshot {
    pub(crate) range: String,
    pub(crate) range_start: DateTime<Utc>,
    pub(crate) range_end: DateTime<Utc>,
    pub(crate) terminal_sequence: u64,
    pub(crate) accounts: Vec<DashboardActivityAccountResponse>,
    pub(crate) summary: DashboardActivitySummaryResponse,
    pub(crate) summary_model_performance_accumulator: ModelPerformanceAccumulator,
    pub(crate) account_model_performance_accumulators:
        HashMap<Option<i64>, ModelPerformanceAccumulator>,
    pub(crate) account_latency_accumulators:
        HashMap<Option<i64>, DashboardActivityAccountLatencyAccumulator>,
    pub(crate) model_performance_accumulator_ready: bool,
    pub(crate) materialized_archive_fallback_totals: StatsTotals,
    pub(crate) materialized_archive_details_limited: bool,
    pub(crate) build_telemetry: DashboardActivityBuildTelemetry,
}

/// Stateful typed base used only by Auto dashboard topic delivery. It retains the aggregate
/// inputs that are intentionally absent from the public response so terminal slices can preserve
/// the existing model-performance, latency, and recent-invocation response semantics.
#[derive(Debug)]
pub(crate) struct DashboardActivityTopicMaterializedBase {
    pub(crate) response: DashboardActivityResponse,
    pub(crate) summary_model_performance_accumulator: ModelPerformanceAccumulator,
    pub(crate) account_model_performance_accumulators:
        HashMap<Option<i64>, ModelPerformanceAccumulator>,
    pub(crate) account_latency_accumulators:
        HashMap<Option<i64>, DashboardActivityAccountLatencyAccumulator>,
    pub(crate) model_performance_accumulator_ready: bool,
    pub(crate) recent_limit: usize,
    pub(crate) include_recent: bool,
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
                    data_quality: None,
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
                    model_groups: Vec::new(),
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

#[cfg(test)]
mod dashboard_activity_routing_tests {
    use super::*;

    fn test_materialized_base() -> DashboardActivityTopicMaterializedBase {
        let snapshot = DashboardActivitySnapshot::test_stub("today");
        let range = ExactUtcRange {
            start: snapshot.range_start,
            end: snapshot.range_end,
        };
        let delta = dashboard_activity_terminal_delta(&summary_projection_test_invocation());
        let mut account = dashboard_activity_terminal_account_for_range(range, &delta);
        account.effective_routing_rule = Some(default_effective_routing_rule());
        account.recent_invocations = vec![PromptCacheConversationInvocationPreviewResponse {
            id: 7,
            invoke_id: "routing-rule-preview".to_string(),
            prompt_cache_key: Some("routing-rule-cache".to_string()),
            occurred_at: "2026-07-20 10:25:09".to_string(),
            status: "success".to_string(),
            live_phase: None,
            failure_class: None,
            route_mode: Some("pool".to_string()),
            model: Some("gpt-5.4".to_string()),
            request_model: Some("gpt-5.4".to_string()),
            response_model: Some("gpt-5.4".to_string()),
            total_tokens: 1_200,
            cost: Some(0.0099),
            proxy_display_name: Some("ciii".to_string()),
            upstream_account_id: Some(17),
            upstream_account_name: Some("Pool 17".to_string()),
            upstream_account_plan_type: None,
            endpoint: Some("/v1/responses".to_string()),
            compaction_request_kind: None,
            compaction_response_kind: None,
            image_intent: None,
            source: Some(SOURCE_PROXY.to_string()),
            input_tokens: Some(1_000),
            output_tokens: Some(200),
            cache_input_tokens: Some(400),
            reasoning_tokens: None,
            reasoning_effort: Some("medium".to_string()),
            error_message: None,
            downstream_status_code: Some(200),
            downstream_error_message: None,
            failure_kind: None,
            blocked_binding: None,
            is_actionable: Some(false),
            response_content_encoding: Some("identity".to_string()),
            request_compression_algorithm: Some("zstd".to_string()),
            transport: Some("http".to_string()),
            requested_service_tier: Some("default".to_string()),
            service_tier: Some("default".to_string()),
            billing_service_tier: Some("default".to_string()),
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: None,
            first_token_ms: Some(720.0),
            t_upstream_stream_ms: Some(500.0),
            t_resp_parse_ms: None,
            t_persist_ms: None,
            t_total_ms: Some(1_200.0),
        }];
        let version = RoutingStateVersion {
            epoch: "2026-09-05T12:00:00.000000000Z".to_string(),
            generation: "4".to_string(),
        };
        DashboardActivityTopicMaterializedBase {
            response: DashboardActivityResponse {
                range: snapshot.range,
                range_start: format_utc_iso_precise(snapshot.range_start),
                range_end: format_utc_iso_precise(snapshot.range_end),
                snapshot_id: snapshot.range_end.timestamp_millis(),
                routing_state_version: Some(version),
                terminal_sequence: snapshot.terminal_sequence,
                live_revision: 3,
                rate_window: DashboardActivityRateWindowResponse {
                    start: format_utc_iso_precise(snapshot.range_start),
                    end: format_utc_iso_precise(snapshot.range_end),
                    window_minutes: 1,
                    mode: "rolling_60s_live_mean".to_string(),
                },
                summary: snapshot.summary,
                network_live_bucket: None,
                network_realtime_rate: None,
                accounts: Some(vec![account]),
            },
            summary_model_performance_accumulator: snapshot.summary_model_performance_accumulator,
            account_model_performance_accumulators: snapshot.account_model_performance_accumulators,
            account_latency_accumulators: snapshot.account_latency_accumulators,
            model_performance_accumulator_ready: snapshot.model_performance_accumulator_ready,
            recent_limit: 16,
            include_recent: true,
        }
    }

    #[test]
    fn routing_rule_live_projection_updates_only_rule_and_version() {
        let mut base = test_materialized_base();
        let before_summary = base.response.summary.stats.total_count;
        let before_recent = base
            .response
            .accounts
            .as_ref()
            .expect("account activity")
            .first()
            .expect("account")
            .recent_invocations
            .len();
        let mut primary = default_effective_routing_rule();
        primary.priority_tier = TagPriorityTier::Primary;
        let version = RoutingStateVersion {
            epoch: "2026-09-05T12:00:00.000000000Z".to_string(),
            generation: "5".to_string(),
        };

        assert!(base.apply_routing_rule_change(&version, &[(17, primary)], &[]));
        {
            let account = base
                .response
                .accounts
                .as_ref()
                .expect("account activity")
                .first()
                .expect("account");
            assert_eq!(
                account
                    .effective_routing_rule
                    .as_ref()
                    .unwrap()
                    .priority_tier,
                TagPriorityTier::Primary
            );
            assert_eq!(account.recent_invocations.len(), before_recent);
        }
        assert_eq!(
            base.response
                .routing_state_version
                .as_ref()
                .unwrap()
                .generation,
            "5"
        );
        assert_eq!(base.response.summary.stats.total_count, before_summary);

        let mut stale = default_effective_routing_rule();
        stale.priority_tier = TagPriorityTier::Fallback;
        assert!(!base.apply_routing_rule_change(&version, &[(17, stale)], &[]));
        assert_eq!(
            base.response
                .accounts
                .as_ref()
                .expect("account activity")
                .first()
                .expect("account")
                .effective_routing_rule
                .as_ref()
                .unwrap()
                .priority_tier,
            TagPriorityTier::Primary
        );
    }
}

pub(crate) fn option_string_capacity(value: &Option<String>) -> usize {
    value.as_ref().map_or(0, String::capacity)
}

pub(crate) fn usage_breakdown_memory_bytes(value: &UsageBreakdownResponse) -> usize {
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

pub(crate) fn model_performance_memory_bytes(value: &ModelPerformanceResponse) -> usize {
    let model_bytes = value
        .models
        .iter()
        .map(|model| model.model.capacity() + option_string_capacity(&model.reasoning_effort))
        .sum::<usize>();
    let model_group_bytes = value
        .model_groups
        .iter()
        .map(|model| model.model.capacity() + option_string_capacity(&model.reasoning_effort))
        .sum::<usize>();
    value
        .models
        .capacity()
        .saturating_mul(std::mem::size_of::<ModelPerformanceModelResponse>())
        .saturating_add(model_bytes)
        .saturating_add(
            value
                .model_groups
                .capacity()
                .saturating_mul(std::mem::size_of::<ModelPerformanceModelResponse>()),
        )
        .saturating_add(model_group_bytes)
}

pub(crate) fn dashboard_activity_account_memory_bytes(
    value: &DashboardActivityAccountResponse,
) -> usize {
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
pub(crate) struct DashboardActivityAccountLatencyAccumulator {
    pub(crate) first_response_byte_total_sample_count: i64,
    pub(crate) first_response_byte_total_sum_ms: f64,
    pub(crate) first_token_sample_count: i64,
    pub(crate) first_token_sum_ms: f64,
    pub(crate) total_latency_sample_count: i64,
    pub(crate) total_latency_sum_ms: f64,
}

impl DashboardActivityAccountLatencyAccumulator {
    pub(crate) fn from_aggregate(aggregate: &UpstreamAccountActivityAccumulator) -> Self {
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

    pub(crate) fn add_terminal_delta(&mut self, delta: &DashboardActivityTerminalDelta) {
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

    pub(crate) fn subtract_terminal_delta(&mut self, delta: &DashboardActivityTerminalDelta) {
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

    pub(crate) fn apply_to_account(self, account: &mut DashboardActivityAccountResponse) {
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
pub(crate) struct DashboardActivitySnapshotCacheOutcome {
    pub(crate) cache_hit_or_miss: &'static str,
    pub(crate) cache_bypass_reason: &'static str,
    pub(crate) coalesced_waiter_count: usize,
    pub(crate) db_build_elapsed_ms: u64,
    pub(crate) cache_ttl_ms: u64,
    pub(crate) cache_entry_age_ms: u64,
    pub(crate) cache_entry_count: usize,
    pub(crate) in_flight_count: usize,
    pub(crate) refresh_reason: &'static str,
    pub(crate) selection_fingerprint: u64,
    pub(crate) terminal_delta_count: u64,
    pub(crate) duplicate_delta_count: u64,
    pub(crate) pending_delta_count: usize,
    pub(crate) pending_delta_estimated_bytes: usize,
    pub(crate) persisted_ack_pending_count: usize,
    pub(crate) delta_pruned_count: u64,
    pub(crate) expiry_delta_count: usize,
    pub(crate) hard_limit_reason: &'static str,
    pub(crate) baseline_cursor: i64,
    pub(crate) sequence_gap_count: u64,
    pub(crate) build_attempted: bool,
    pub(crate) snapshot_origin: &'static str,
}

pub(crate) fn dashboard_activity_read_model_state(
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
pub(crate) struct DashboardActivityBuildTelemetry {
    pub(crate) preview_read_mode: &'static str,
    pub(crate) candidate_preview_id_count: usize,
    pub(crate) hydrated_preview_row_count: usize,
    pub(crate) account_aggregation: AccountActivityRangeBuildTelemetry,
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

pub(crate) fn dashboard_activity_selection_includes_compact_terminal(
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

pub(crate) fn dashboard_activity_selection_includes_terminal(
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

pub(crate) fn dashboard_activity_selection_source_scope(
    selection: &DashboardActivitySnapshotSelection,
) -> InvocationSourceScope {
    if selection.source_scope == "proxy_only" {
        InvocationSourceScope::ProxyOnly
    } else {
        InvocationSourceScope::All
    }
}

pub(crate) fn dashboard_activity_entry_includes_terminal(
    entry: &DashboardActivitySnapshotCacheEntry,
    delta: &DashboardActivityTerminalDelta,
) -> bool {
    delta
        .persisted_row_id
        .is_some_and(|row_id| row_id <= entry.baseline_snapshot_cursor)
}

pub(crate) fn dashboard_activity_baseline_includes_pending_delta(
    delta: &DashboardActivityTerminalDelta,
    baseline_cursor: i64,
    persisted_terminal_ids: &HashMap<(String, String), i64>,
) -> bool {
    delta
        .persisted_row_id
        .is_some_and(|row_id| row_id <= baseline_cursor)
        || persisted_terminal_ids.contains_key(&delta.key())
}

pub(crate) fn replay_dashboard_activity_pending_deltas_without_expiry(
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

pub(crate) fn dashboard_activity_terminal_delta(
    record: &ApiInvocation,
) -> DashboardActivityTerminalDelta {
    dashboard_activity_terminal_delta_with_first_token(
        record,
        runtime_record_first_token_ms(record),
    )
}

// Persisted dashboard rows come from build_invocation_select_query(), whose timing expression
// already establishes final-attempt ownership. Runtime records need the stricter retry guard.
pub(crate) fn persisted_dashboard_activity_terminal_delta(
    record: &ApiInvocation,
) -> DashboardActivityTerminalDelta {
    dashboard_activity_terminal_delta_with_first_token(
        record,
        finite_nonnegative_timing(record.first_token_ms),
    )
}

pub(crate) fn dashboard_activity_terminal_delta_with_first_token(
    record: &ApiInvocation,
    first_token_ms: Option<f64>,
) -> DashboardActivityTerminalDelta {
    let classification = resolve_failure_classification(
        record.status.as_deref(),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        record.failure_class.as_deref(),
        record.is_actionable.map(i64::from),
    );
    let success = runtime_record_is_success_for_summary(record)
        && classification.failure_class == FailureClass::None;
    let failure = matches!(
        classification.failure_class,
        FailureClass::ServiceFailure | FailureClass::ClientFailure | FailureClass::ClientAbort
    );
    let cache_read_tokens = record.cache_input_tokens.unwrap_or_default().max(0);
    let cache_write_tokens = record
        .cache_write_tokens
        .unwrap_or_else(|| {
            record
                .input_tokens
                .unwrap_or_default()
                .saturating_sub(cache_read_tokens)
        })
        .max(0);
    let model = record
        .response_model
        .as_deref()
        .or(record.model.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_string();
    let reasoning_effort = record
        .reasoning_effort
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let upstream_account_name = record
        .upstream_account_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let breakdown_complete = record.cost.is_some()
        && record.cost_input.is_some()
        && record.cost_cache_write.is_some()
        && record.cost_cache_read.is_some()
        && record.cost_output.is_some()
        && record.cost_reasoning.is_some();
    let total_cost = record.cost.unwrap_or_default().max(0.0);
    let string_bytes = record.invoke_id.len()
        + record.occurred_at.len()
        + record.source.len()
        + model.len()
        + reasoning_effort.as_ref().map_or(0, String::len)
        + upstream_account_name.as_ref().map_or(0, String::len);
    let recent_projection_string_bytes = record.invoke_id.len()
        + record.prompt_cache_key.as_ref().map_or(0, String::len)
        + record.occurred_at.len()
        + record.status.as_ref().map_or(0, String::len)
        + record.live_phase.as_ref().map_or(0, String::len)
        + record.failure_class.as_ref().map_or(0, String::len)
        + record.route_mode.as_ref().map_or(0, String::len)
        + record.model.as_ref().map_or(0, String::len)
        + record.request_model.as_ref().map_or(0, String::len)
        + record.response_model.as_ref().map_or(0, String::len)
        + record.proxy_display_name.as_ref().map_or(0, String::len)
        + record.upstream_account_name.as_ref().map_or(0, String::len)
        + record.endpoint.as_ref().map_or(0, String::len)
        + record
            .compaction_request_kind
            .as_ref()
            .map_or(0, String::len)
        + record
            .compaction_response_kind
            .as_ref()
            .map_or(0, String::len)
        + record.image_intent.as_ref().map_or(0, String::len)
        + record.reasoning_effort.as_ref().map_or(0, String::len)
        + record.error_message.as_ref().map_or(0, String::len)
        + record
            .downstream_error_message
            .as_ref()
            .map_or(0, String::len)
        + record.failure_kind.as_ref().map_or(0, String::len)
        + record
            .response_content_encoding
            .as_ref()
            .map_or(0, String::len)
        + record
            .request_compression_algorithm
            .as_ref()
            .map_or(0, String::len)
        + record.transport.as_ref().map_or(0, String::len)
        + record
            .requested_service_tier
            .as_ref()
            .map_or(0, String::len)
        + record.service_tier.as_ref().map_or(0, String::len)
        + record.billing_service_tier.as_ref().map_or(0, String::len);
    let mut timeseries = TimeseriesTerminalDelta::from(record);
    timeseries.first_token_ms = first_token_ms;
    DashboardActivityTerminalDelta {
        terminal_sequence: 0,
        timeseries,
        invoke_id: record.invoke_id.clone(),
        occurred_at: record.occurred_at.clone(),
        source: record.source.clone(),
        model,
        reasoning_effort,
        upstream_account_id: record.upstream_account_id,
        upstream_account_name,
        success,
        failure,
        total_tokens: record.total_tokens.unwrap_or_default().max(0),
        cache_write_tokens,
        cache_read_tokens,
        output_tokens: record.output_tokens.unwrap_or_default().max(0),
        has_cost: record.cost.is_some(),
        total_cost,
        cost_input: record
            .cost_input
            .filter(|_| breakdown_complete)
            .unwrap_or_default(),
        cost_cache_write: record
            .cost_cache_write
            .filter(|_| breakdown_complete)
            .unwrap_or_default(),
        cost_cache_read: record
            .cost_cache_read
            .filter(|_| breakdown_complete)
            .unwrap_or_default(),
        cost_output: record
            .cost_output
            .filter(|_| breakdown_complete)
            .unwrap_or_default(),
        cost_reasoning: record
            .cost_reasoning
            .filter(|_| breakdown_complete)
            .unwrap_or_default(),
        cost_unknown: if breakdown_complete { 0.0 } else { total_cost },
        t_total_ms: record.t_total_ms,
        t_req_read_ms: record.t_req_read_ms,
        t_req_parse_ms: record.t_req_parse_ms,
        t_upstream_connect_ms: record.t_upstream_connect_ms,
        t_upstream_ttfb_ms: record.t_upstream_ttfb_ms,
        first_token_ms,
        t_upstream_stream_ms: record.t_upstream_stream_ms,
        recent_invocation: invocation_preview_from_runtime_record(record),
        persisted_row_id: (record.id > 0).then_some(record.id),
        estimated_bytes: std::mem::size_of::<DashboardActivityTerminalDelta>()
            + string_bytes
            + recent_projection_string_bytes,
    }
}

pub(crate) fn dashboard_activity_terminal_delta_matches_source_scope(
    delta: &DashboardActivityTerminalDelta,
    source_scope: InvocationSourceScope,
) -> bool {
    source_scope != InvocationSourceScope::ProxyOnly || delta.source == SOURCE_PROXY
}

pub(crate) fn dashboard_activity_terminal_delta_is_within_range(
    delta: &DashboardActivityTerminalDelta,
    range: ExactUtcRange,
) -> bool {
    parse_to_utc_datetime(&delta.occurred_at)
        .is_some_and(|occurred_at| occurred_at >= range.start && occurred_at < range.end)
}

pub(crate) fn add_dashboard_activity_terminal_usage(
    usage: &mut UsageBreakdownResponse,
    delta: &DashboardActivityTerminalDelta,
) {
    let costs = delta.has_cost.then_some(UsageCostBreakdownResponse {
        input: delta.cost_input,
        cache_write: delta.cost_cache_write,
        cache_read: delta.cost_cache_read,
        output: delta.cost_output,
        reasoning: delta.cost_reasoning,
        unknown: delta.cost_unknown,
    });

    usage.cache_write_tokens += delta.cache_write_tokens;
    usage.cache_read_tokens += delta.cache_read_tokens;
    usage.output_tokens += delta.output_tokens;
    if let Some(costs) = &costs {
        let target = usage
            .costs
            .get_or_insert_with(UsageCostBreakdownResponse::default);
        target.input += costs.input;
        target.cache_write += costs.cache_write;
        target.cache_read += costs.cache_read;
        target.output += costs.output;
        target.reasoning += costs.reasoning;
        target.unknown += costs.unknown;
    }

    let model_usage = usage.models.iter_mut().find(|entry| {
        entry.model == delta.model
            && entry.reasoning_effort.as_deref() == delta.reasoning_effort.as_deref()
    });
    let entry = if let Some(entry) = model_usage {
        entry
    } else {
        usage.models.push(UsageBreakdownModelResponse {
            model: delta.model.clone(),
            reasoning_effort: delta.reasoning_effort.clone(),
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            output_tokens: 0,
            costs: None,
        });
        usage.models.last_mut().expect("just inserted usage model")
    };
    entry.cache_write_tokens += delta.cache_write_tokens;
    entry.cache_read_tokens += delta.cache_read_tokens;
    entry.output_tokens += delta.output_tokens;
    if let Some(costs) = costs {
        let target = entry
            .costs
            .get_or_insert_with(UsageCostBreakdownResponse::default);
        target.input += costs.input;
        target.cache_write += costs.cache_write;
        target.cache_read += costs.cache_read;
        target.output += costs.output;
        target.reasoning += costs.reasoning;
        target.unknown += costs.unknown;
    }
    usage.models.sort_by(|left, right| {
        left.model
            .cmp(&right.model)
            .then_with(|| left.reasoning_effort.cmp(&right.reasoning_effort))
    });
}

pub(crate) fn subtract_dashboard_activity_terminal_usage(
    usage: &mut UsageBreakdownResponse,
    delta: &DashboardActivityTerminalDelta,
) {
    usage.cache_write_tokens = usage
        .cache_write_tokens
        .saturating_sub(delta.cache_write_tokens);
    usage.cache_read_tokens = usage
        .cache_read_tokens
        .saturating_sub(delta.cache_read_tokens);
    usage.output_tokens = usage.output_tokens.saturating_sub(delta.output_tokens);
    if let Some(costs) = usage.costs.as_mut() {
        costs.input = (costs.input - delta.cost_input).max(0.0);
        costs.cache_write = (costs.cache_write - delta.cost_cache_write).max(0.0);
        costs.cache_read = (costs.cache_read - delta.cost_cache_read).max(0.0);
        costs.output = (costs.output - delta.cost_output).max(0.0);
        costs.reasoning = (costs.reasoning - delta.cost_reasoning).max(0.0);
        costs.unknown = (costs.unknown - delta.cost_unknown).max(0.0);
    }
    if let Some(entry) = usage.models.iter_mut().find(|entry| {
        entry.model == delta.model
            && entry.reasoning_effort.as_deref() == delta.reasoning_effort.as_deref()
    }) {
        entry.cache_write_tokens = entry
            .cache_write_tokens
            .saturating_sub(delta.cache_write_tokens);
        entry.cache_read_tokens = entry
            .cache_read_tokens
            .saturating_sub(delta.cache_read_tokens);
        entry.output_tokens = entry.output_tokens.saturating_sub(delta.output_tokens);
        if let Some(costs) = entry.costs.as_mut() {
            costs.input = (costs.input - delta.cost_input).max(0.0);
            costs.cache_write = (costs.cache_write - delta.cost_cache_write).max(0.0);
            costs.cache_read = (costs.cache_read - delta.cost_cache_read).max(0.0);
            costs.output = (costs.output - delta.cost_output).max(0.0);
            costs.reasoning = (costs.reasoning - delta.cost_reasoning).max(0.0);
            costs.unknown = (costs.unknown - delta.cost_unknown).max(0.0);
        }
    }
    usage.models.retain(|entry| {
        entry.cache_write_tokens != 0
            || entry.cache_read_tokens != 0
            || entry.output_tokens != 0
            || entry.costs.as_ref().is_some_and(|costs| {
                costs.input != 0.0
                    || costs.cache_write != 0.0
                    || costs.cache_read != 0.0
                    || costs.output != 0.0
                    || costs.reasoning != 0.0
                    || costs.unknown != 0.0
            })
    });
}

pub(crate) fn dashboard_activity_terminal_account(
    snapshot: &DashboardActivitySnapshot,
    delta: &DashboardActivityTerminalDelta,
) -> DashboardActivityAccountResponse {
    dashboard_activity_terminal_account_for_range(
        ExactUtcRange {
            start: snapshot.range_start,
            end: snapshot.range_end,
        },
        delta,
    )
}

pub(crate) fn dashboard_activity_terminal_account_for_range(
    range: ExactUtcRange,
    delta: &DashboardActivityTerminalDelta,
) -> DashboardActivityAccountResponse {
    let upstream_account_id = delta.upstream_account_id;
    DashboardActivityAccountResponse {
        account_key: upstream_account_id
            .map(|id| format!("upstream:{id}"))
            .unwrap_or_else(|| "unassigned".to_string()),
        upstream_account_id,
        display_name: upstream_account_id
            .and_then(|_| delta.upstream_account_name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| {
                upstream_account_id
                    .map(|id| format!("Upstream {id}"))
                    .unwrap_or_else(|| "Unassigned upstream account".to_string())
            }),
        is_unassigned: upstream_account_id.is_none(),
        latest_conversation_created_at: None,
        last_invocation_at: None,
        group_name: None,
        plan_type: None,
        enabled: None,
        display_status: None,
        enable_status: None,
        work_status: None,
        health_status: None,
        sync_state: None,
        last_error: None,
        last_action_reason_message: None,
        request_count: 0,
        success_count: 0,
        failure_count: 0,
        non_success_count: 0,
        total_tokens: 0,
        success_tokens: 0,
        non_success_tokens: 0,
        failure_tokens: 0,
        failure_cost: 0.0,
        non_success_cost: 0.0,
        total_cost: 0.0,
        usage_breakdown: UsageBreakdownResponse {
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            output_tokens: 0,
            costs: None,
            models: Vec::new(),
        },
        model_performance: ModelPerformanceAccumulator::default().into_response(range, false),
        cache_hit_rate: None,
        tokens_per_minute: None,
        spend_rate: None,
        first_byte_avg_ms: None,
        first_response_byte_total_avg_ms: None,
        first_token_avg_ms: None,
        avg_total_ms: None,
        current_first_response_byte_total_avg_ms: None,
        current_first_token_avg_ms: None,
        current_avg_total_ms: None,
        current_avg_response_ms: None,
        in_progress_invocation_count: Some(0),
        in_progress_phase_counts: Some(InvocationPhaseCountsResponse::default()),
        retry_invocation_count: Some(0),
        upload_bytes_per_second: 0.0,
        download_bytes_per_second: 0.0,
        in_progress_wait_sum_ms: 0.0,
        in_progress_wait_sample_count: 0,
        effective_routing_rule: None,
        recent_invocations: Vec::new(),
    }
}

pub(crate) fn apply_dashboard_activity_compact_terminal_delta(
    snapshot: &mut DashboardActivitySnapshot,
    delta: &DashboardActivityTerminalDelta,
) {
    snapshot.terminal_sequence = snapshot.terminal_sequence.max(delta.terminal_sequence);
    apply_dashboard_activity_terminal_delta_to_stats(&mut snapshot.summary.stats, delta);

    let model_performance_available = snapshot.summary.model_performance.available;
    let snapshot_range = ExactUtcRange {
        start: snapshot.range_start,
        end: snapshot.range_end,
    };
    if snapshot.model_performance_accumulator_ready {
        snapshot
            .summary_model_performance_accumulator
            .add_terminal_delta(delta);
        snapshot.summary.model_performance = snapshot
            .summary_model_performance_accumulator
            .clone()
            .into_response(snapshot_range, model_performance_available);
        snapshot
            .account_model_performance_accumulators
            .entry(delta.upstream_account_id)
            .or_default()
            .add_terminal_delta(delta);
    }
    let account_model_performance = snapshot
        .account_model_performance_accumulators
        .get(&delta.upstream_account_id)
        .cloned()
        .unwrap_or_default()
        .into_response(snapshot_range, model_performance_available);

    if !snapshot
        .accounts
        .iter()
        .any(|account| account.upstream_account_id == delta.upstream_account_id)
    {
        snapshot
            .accounts
            .push(dashboard_activity_terminal_account(snapshot, delta));
    }
    let account_latency = snapshot
        .account_latency_accumulators
        .entry(delta.upstream_account_id)
        .or_default();
    account_latency.add_terminal_delta(delta);
    let account_latency = *account_latency;
    let account = snapshot
        .accounts
        .iter_mut()
        .find(|account| account.upstream_account_id == delta.upstream_account_id)
        .expect("terminal account inserted when absent");
    apply_dashboard_activity_terminal_delta_to_account(account, delta);
    account_latency.apply_to_account(account);
    account.model_performance = account_model_performance;
    sort_dashboard_activity_accounts(&mut snapshot.accounts);
}

pub(crate) fn apply_dashboard_activity_terminal_delta_to_stats(
    stats: &mut StatsResponse,
    delta: &DashboardActivityTerminalDelta,
) {
    let non_success = !delta.success;
    stats.total_count += 1;
    stats.success_count += i64::from(delta.success);
    stats.failure_count += i64::from(delta.failure);
    stats.total_tokens += delta.total_tokens;
    stats.total_cost += delta.total_cost;
    if non_success {
        if let Some(non_success_cost) = stats.non_success_cost.as_mut() {
            *non_success_cost += delta.total_cost;
        }
        if let Some(non_success_tokens) = stats.non_success_tokens.as_mut() {
            *non_success_tokens += delta.total_tokens;
        }
    }
    if let Some(usage) = stats.usage_breakdown.as_mut() {
        add_dashboard_activity_terminal_usage(usage, delta);
    }
}

pub(crate) fn apply_dashboard_activity_terminal_delta_to_account(
    account: &mut DashboardActivityAccountResponse,
    delta: &DashboardActivityTerminalDelta,
) {
    let non_success = !delta.success;
    account.request_count += 1;
    account.success_count += i64::from(delta.success);
    account.failure_count += i64::from(delta.failure);
    account.non_success_count += i64::from(non_success);
    account.total_tokens += delta.total_tokens;
    account.total_cost += delta.total_cost;
    if delta.success {
        account.success_tokens += delta.total_tokens;
    } else {
        account.non_success_tokens += delta.total_tokens;
    }
    if delta.failure {
        account.failure_tokens += delta.total_tokens;
        account.failure_cost += delta.total_cost;
    }
    if non_success {
        account.non_success_cost += delta.total_cost;
    }
    if account
        .last_invocation_at
        .as_deref()
        .is_none_or(|current| delta.occurred_at.as_str() > current)
    {
        account.last_invocation_at = Some(delta.occurred_at.clone());
    }
    add_dashboard_activity_terminal_usage(&mut account.usage_breakdown, delta);
    account.cache_hit_rate = (account.total_tokens > 0)
        .then_some(account.usage_breakdown.cache_read_tokens as f64 / account.total_tokens as f64);
}

pub(crate) fn apply_dashboard_activity_terminal_delta(
    snapshot: &mut DashboardActivitySnapshot,
    record: &ApiInvocation,
) {
    apply_dashboard_activity_compact_terminal_delta(
        snapshot,
        &dashboard_activity_terminal_delta(record),
    );
}

pub(crate) fn subtract_dashboard_activity_compact_terminal_delta(
    snapshot: &mut DashboardActivitySnapshot,
    delta: &DashboardActivityTerminalDelta,
) {
    let non_success = !delta.success;
    let stats = &mut snapshot.summary.stats;
    stats.total_count = stats.total_count.saturating_sub(1);
    stats.success_count = stats.success_count.saturating_sub(i64::from(delta.success));
    stats.failure_count = stats.failure_count.saturating_sub(i64::from(delta.failure));
    stats.total_tokens = stats.total_tokens.saturating_sub(delta.total_tokens);
    stats.total_cost = (stats.total_cost - delta.total_cost).max(0.0);
    if non_success {
        if let Some(value) = stats.non_success_cost.as_mut() {
            *value = (*value - delta.total_cost).max(0.0);
        }
        if let Some(value) = stats.non_success_tokens.as_mut() {
            *value = value.saturating_sub(delta.total_tokens);
        }
    }
    if let Some(usage) = stats.usage_breakdown.as_mut() {
        subtract_dashboard_activity_terminal_usage(usage, delta);
    }
    let snapshot_range = ExactUtcRange {
        start: snapshot.range_start,
        end: snapshot.range_end,
    };
    let available = snapshot.summary.model_performance.available;
    if snapshot.model_performance_accumulator_ready {
        snapshot
            .summary_model_performance_accumulator
            .subtract_terminal_delta(delta);
        snapshot.summary.model_performance = snapshot
            .summary_model_performance_accumulator
            .clone()
            .into_response(snapshot_range, available);
        if let Some(accumulator) = snapshot
            .account_model_performance_accumulators
            .get_mut(&delta.upstream_account_id)
        {
            accumulator.subtract_terminal_delta(delta);
        }
    }
    if let Some(latency) = snapshot
        .account_latency_accumulators
        .get_mut(&delta.upstream_account_id)
    {
        latency.subtract_terminal_delta(delta);
    }
    let account_performance = snapshot
        .account_model_performance_accumulators
        .get(&delta.upstream_account_id)
        .cloned()
        .unwrap_or_default()
        .into_response(snapshot_range, available);
    if let Some(account) = snapshot
        .accounts
        .iter_mut()
        .find(|account| account.upstream_account_id == delta.upstream_account_id)
    {
        account.request_count = account.request_count.saturating_sub(1);
        account.success_count = account
            .success_count
            .saturating_sub(i64::from(delta.success));
        account.failure_count = account
            .failure_count
            .saturating_sub(i64::from(delta.failure));
        account.non_success_count = account
            .non_success_count
            .saturating_sub(i64::from(non_success));
        account.total_tokens = account.total_tokens.saturating_sub(delta.total_tokens);
        account.total_cost = (account.total_cost - delta.total_cost).max(0.0);
        if delta.success {
            account.success_tokens = account.success_tokens.saturating_sub(delta.total_tokens);
        } else {
            account.non_success_tokens = account
                .non_success_tokens
                .saturating_sub(delta.total_tokens);
        }
        if delta.failure {
            account.failure_tokens = account.failure_tokens.saturating_sub(delta.total_tokens);
            account.failure_cost = (account.failure_cost - delta.total_cost).max(0.0);
        }
        if non_success {
            account.non_success_cost = (account.non_success_cost - delta.total_cost).max(0.0);
        }
        if account.last_invocation_at.as_deref() == Some(delta.occurred_at.as_str()) {
            account.last_invocation_at = None;
        }
        subtract_dashboard_activity_terminal_usage(&mut account.usage_breakdown, delta);
        if let Some(latency) = snapshot
            .account_latency_accumulators
            .get(&delta.upstream_account_id)
            .copied()
        {
            latency.apply_to_account(account);
        }
        account.cache_hit_rate = (account.total_tokens > 0).then_some(
            account.usage_breakdown.cache_read_tokens as f64 / account.total_tokens as f64,
        );
        account.model_performance = account_performance;
    }
    snapshot
        .accounts
        .retain(|account| account.request_count > 0);
    sort_dashboard_activity_accounts(&mut snapshot.accounts);
}

pub(crate) fn restore_dashboard_activity_last_invocation_after_expiry(
    snapshot: &mut DashboardActivitySnapshot,
    expired: &DashboardActivityTerminalDelta,
    remaining_expiry_deltas: &VecDeque<DashboardActivityTerminalDelta>,
) {
    let Some(account) = snapshot
        .accounts
        .iter_mut()
        .find(|account| account.upstream_account_id == expired.upstream_account_id)
    else {
        return;
    };
    if account.last_invocation_at.is_some() || account.request_count <= 0 {
        return;
    }
    account.last_invocation_at = remaining_expiry_deltas
        .iter()
        .filter(|delta| delta.upstream_account_id == expired.upstream_account_id)
        .max_by_key(|delta| parse_to_utc_datetime(&delta.occurred_at))
        .map(|delta| delta.occurred_at.clone());
}

/// Applies an accepted terminal record to every warm open-range baseline. This is deliberately
/// independent from the subscription listener so a lagged broadcast cannot drop cumulative UI
/// totals before the next reconciliation build.
pub(crate) async fn apply_dashboard_activity_terminal_record(
    state: &AppState,
    record: &ApiInvocation,
) -> DashboardActivityTerminalDeltaOutcome {
    let mut delta = dashboard_activity_terminal_delta(record);
    let Some(occurred_at) = parse_to_utc_datetime(&delta.occurred_at) else {
        return DashboardActivityTerminalDeltaOutcome::default();
    };
    let key = delta.key();
    let mut outcome = DashboardActivityTerminalDeltaOutcome::default();
    let mut cache = state.dashboard_activity_snapshot_cache.lock().await;
    if cache.read_model.applied_terminal_keys.contains_key(&key) {
        cache.read_model.duplicate_delta_count += 1;
        outcome.duplicate = true;
        return outcome;
    }
    cache.read_model.next_terminal_sequence =
        cache.read_model.next_terminal_sequence.saturating_add(1);
    delta.terminal_sequence = cache.read_model.next_terminal_sequence;
    outcome.terminal_sequence = Some(delta.terminal_sequence);
    insert_dashboard_activity_applied_terminal_key(&mut cache.read_model, key);
    cache.read_model.terminal_delta_count += 1;
    prune_dashboard_activity_applied_terminal_keys(&mut cache.read_model);
    // Registration happens before the SQLite enqueue.  Even when the caller already carries a
    // positive row id, that id is not a durability proof until the writer ACKs the committed
    // transaction below; advancing the contiguous watermark here would let a failed enqueue be
    // published as covered source progress.
    let hard_limit_reason = dashboard_activity_terminal_delta_needs_persistence_ack(&delta)
        .then(|| dashboard_activity_terminal_delta_hard_limit_reason(&cache.read_model, &delta))
        .flatten();
    if let Some(reason) = hard_limit_reason {
        cache.read_model.hard_limit_reason = Some(reason);
        cache.read_model.hard_limit_sequence = Some(delta.terminal_sequence);
        cache.read_model.pending_terminal_overflow_count += 1;
        outcome.hard_limit_reason = Some(reason);
        tracing::warn!(
            hard_limit_reason = reason,
            pending_delta_count = cache.read_model.pending_terminal_deltas.len(),
            pending_delta_estimated_bytes = cache.read_model.pending_delta_estimated_bytes,
            persisted_ack_pending_count = cache.read_model.persisted_ack_pending_count,
            "dashboard activity read model entered dirty last-good protection"
        );
        return outcome;
    }
    let mut expiry_hard_limit = None;
    for (selection, entry) in &mut cache.entries {
        if !dashboard_activity_selection_includes_compact_terminal(selection, &delta, occurred_at) {
            outcome.skipped_out_of_range_count += 1;
            continue;
        }
        if dashboard_activity_entry_includes_terminal(entry, &delta) {
            continue;
        }
        if let Err(reason) = insert_dashboard_activity_expiry_delta(
            &mut entry.expiry_terminal_deltas,
            &mut entry.expiry_delta_estimated_bytes,
            entry.expiry_covered_until,
            &delta,
            occurred_at,
        ) {
            entry.expiry_covered_until = Some(DateTime::<Utc>::MIN_UTC);
            expiry_hard_limit = Some(reason);
            continue;
        }
        apply_dashboard_activity_compact_terminal_delta(&mut entry.response, &delta);
        outcome.applied_selection_count += 1;
    }
    let evicted_entry_count = prune_dashboard_activity_snapshot_entries(&mut cache, None);
    if evicted_entry_count > 0 {
        prune_dashboard_activity_terminal_deltas(&mut cache);
        tracing::debug!(
            evicted_entry_count,
            cache_entry_count = cache.entries.len(),
            "evicted dashboard activity snapshots to enforce the global cache budget"
        );
    }
    if let Some(reason) = expiry_hard_limit {
        cache.read_model.hard_limit_reason = Some(reason);
        cache.read_model.pending_terminal_overflow_count += 1;
        outcome.hard_limit_reason = Some(reason);
        return outcome;
    }
    outcome.terminal_delta = Some(delta.clone());
    if !dashboard_activity_terminal_delta_needs_persistence_ack(&delta) {
        return outcome;
    }
    cache.read_model.pending_delta_estimated_bytes = cache
        .read_model
        .pending_delta_estimated_bytes
        .saturating_add(delta.estimated_bytes);
    cache.read_model.pending_terminal_deltas.push_back(delta);
    outcome
}

pub(crate) fn prune_dashboard_activity_applied_terminal_keys(
    read_model: &mut DashboardActivityReadModel,
) {
    let max_age = Duration::from_secs(7 * 24 * 60 * 60);
    while let Some((key, applied_at)) = read_model.applied_terminal_key_order.front() {
        let is_live_entry = read_model.applied_terminal_keys.get(key) == Some(applied_at);
        if is_live_entry
            && read_model.applied_terminal_keys.len()
                <= DASHBOARD_ACTIVITY_READ_MODEL_MAX_TERMINAL_KEYS
            && applied_at.elapsed() <= max_age
        {
            break;
        }
        let key = key.clone();
        let applied_at = *applied_at;
        read_model.applied_terminal_key_order.pop_front();
        if read_model.applied_terminal_keys.get(&key) == Some(&applied_at) {
            read_model.applied_terminal_keys.remove(&key);
        }
    }
}

pub(crate) fn insert_dashboard_activity_applied_terminal_key(
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

pub(crate) fn dashboard_activity_terminal_delta_hard_limit_reason(
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

pub(crate) fn dashboard_activity_terminal_delta_needs_persistence_ack(
    delta: &DashboardActivityTerminalDelta,
) -> bool {
    delta.persisted_row_id.is_none()
}

pub(crate) fn prune_dashboard_activity_terminal_deltas(
    cache: &mut DashboardActivitySnapshotCacheState,
) {
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

pub(crate) fn prune_dashboard_activity_snapshot_entries(
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

pub(crate) fn dashboard_activity_hard_limit_is_settled(
    read_model: &DashboardActivityReadModel,
) -> bool {
    read_model
        .hard_limit_sequence
        .is_none_or(|sequence| read_model.settled_terminal_sequence >= sequence)
}

pub(crate) fn dashboard_activity_read_model_requires_reconcile(
    read_model: &DashboardActivityReadModel,
) -> bool {
    read_model.hard_limit_reason.is_some() && dashboard_activity_hard_limit_is_settled(read_model)
}

pub(crate) fn settle_dashboard_activity_terminal_sequence(
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

pub(crate) fn clear_dashboard_activity_hard_limit_after_baseline(
    read_model: &mut DashboardActivityReadModel,
) {
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

pub(crate) fn dashboard_activity_build_scope(
    include_accounts: bool,
    _include_recent: bool,
) -> &'static str {
    if !include_accounts {
        "summary_only"
    } else {
        "full"
    }
}

pub(crate) fn dashboard_activity_stats_totals_from_aggregate_rows(
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

pub(crate) fn dashboard_activity_stats_totals_subtract(
    left: StatsTotals,
    right: StatsTotals,
) -> StatsTotals {
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

pub(crate) fn dashboard_activity_stats_totals_has_values(totals: StatsTotals) -> bool {
    totals.total_count > 0
        || totals.success_count > 0
        || totals.failure_count > 0
        || totals.total_cost > 0.0
        || totals.total_tokens > 0
        || totals.non_success_cost > 0.0
}
