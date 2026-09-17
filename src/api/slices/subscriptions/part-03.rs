#[derive(Debug)]
struct DashboardSummaryMaterializerState {
    response: StatsResponse,
    current_revision: Option<u64>,
    terminal_revision: Option<u64>,
    terminal_sequence: u64,
    range_start: Option<DateTime<Utc>>,
}

#[derive(Debug)]
struct DashboardParallelWorkMaterializerState {
    response: ParallelWorkStatsResponse,
    baseline_response: ParallelWorkStatsResponse,
    bucket_keys: BTreeMap<i64, HashSet<String>>,
    baseline_bucket_keys: BTreeMap<i64, HashSet<String>>,
    minute_keys: BTreeMap<i64, HashSet<String>>,
    baseline_minute_keys: BTreeMap<i64, HashSet<String>>,
    active_minute_stats: ParallelWorkActiveMinuteStats,
    baseline_active_minute_stats: ParallelWorkActiveMinuteStats,
    baseline_complete_minute_start_epoch: i64,
    baseline_complete_minute_end_epoch: i64,
    baseline_row_id: i64,
    range: String,
    reporting_tz: Tz,
    upstream_account_id: Option<i64>,
    conversations_enabled: bool,
    baseline_identities: HashSet<String>,
    applied_identities: HashSet<String>,
    runtime_mutations: BTreeMap<String, RuntimeInvocationMutation>,
    revision: u64,
}

#[derive(Debug, Default)]
struct ParallelWorkMutationOutcome {
    changed: bool,
    needs_account_reconcile: bool,
}

impl DashboardParallelWorkMaterializerState {
    fn apply_runtime_mutation(
        &mut self,
        mutation: &RuntimeInvocationMutation,
    ) -> ParallelWorkMutationOutcome {
        let identity = format!(
            "{}\0{}",
            mutation.identity.invoke_id, mutation.identity.occurred_at
        );
        if mutation.kind == RuntimeMutationKind::RuntimeRemoved {
            if self.runtime_mutations.remove(&identity).is_none() {
                return ParallelWorkMutationOutcome::default();
            }
            self.applied_identities.remove(&identity);
            self.rebuild_runtime_overlay();
            return ParallelWorkMutationOutcome {
                changed: true,
                needs_account_reconcile: false,
            };
        }
        if !matches!(
            mutation.kind,
            RuntimeMutationKind::RuntimeUpsert
                | RuntimeMutationKind::TerminalCommitted
                | RuntimeMutationKind::Recovery
        ) {
            return ParallelWorkMutationOutcome::default();
        }
        if let Some(account_id) = self.upstream_account_id
            && mutation.upstream_account_id != Some(account_id)
        {
            return ParallelWorkMutationOutcome {
                changed: false,
                needs_account_reconcile: mutation.upstream_account_id.is_none()
                    && mutation.prompt_cache_key.is_some(),
            };
        }
        if self.baseline_identities.contains(&identity) {
            return ParallelWorkMutationOutcome::default();
        }
        if mutation
            .row_id
            .is_some_and(|row_id| row_id <= self.baseline_row_id)
        {
            return ParallelWorkMutationOutcome::default();
        };
        if self.applied_identities.contains(&identity) {
            return ParallelWorkMutationOutcome::default();
        }

        let changed = self.apply_runtime_overlay(mutation);
        if changed {
            self.applied_identities.insert(identity.clone());
            self.runtime_mutations.insert(identity, mutation.clone());
            self.revision = self.revision.saturating_add(1);
        }
        ParallelWorkMutationOutcome {
            changed,
            needs_account_reconcile: false,
        }
    }

    fn replay_runtime_mutations(
        &mut self,
        mutations: &BTreeMap<String, RuntimeInvocationMutation>,
    ) -> bool {
        let mut changed = false;
        for mutation in mutations.values() {
            changed |= self.apply_runtime_mutation(mutation).changed;
        }
        changed
    }

    fn apply_runtime_overlay(&mut self, mutation: &RuntimeInvocationMutation) -> bool {
        self.apply_runtime_overlay_at(mutation, Utc::now())
    }

    fn apply_runtime_overlay_at(
        &mut self,
        mutation: &RuntimeInvocationMutation,
        now: DateTime<Utc>,
    ) -> bool {
        let Some(prompt_cache_key) = mutation.prompt_cache_key.as_ref() else {
            return false;
        };
        let Some(occurred_at) = parse_to_utc_datetime(&mutation.identity.occurred_at) else {
            return false;
        };
        if parse_to_utc_datetime(&self.response.current.range_start)
            .is_none_or(|range_start| occurred_at < range_start)
        {
            return false;
        }

        let active_minute_stats_changed = self.refresh_active_minute_stats_at(now);

        let bucket_seconds = self.response.current.bucket_seconds;
        let Ok(bucket_start_epoch) = align_reporting_bucket_epoch(
            occurred_at.timestamp(),
            bucket_seconds,
            self.reporting_tz,
        ) else {
            return false;
        };
        let bucket_changed = self
            .bucket_keys
            .entry(bucket_start_epoch)
            .or_default()
            .insert(prompt_cache_key.clone());
        let minute_start_epoch = occurred_at.timestamp().div_euclid(60) * 60;
        let minute_keys = self.minute_keys.entry(minute_start_epoch).or_default();
        let minute_changed = minute_keys.insert(prompt_cache_key.clone());
        let minute_is_complete = minute_start_epoch >= self.baseline_complete_minute_start_epoch
            && minute_start_epoch < now.timestamp().div_euclid(60) * 60;
        if minute_changed
            && minute_is_complete
            && let Some(active_minute_count) = self.active_minute_stats.active_minute_count
        {
            let previous_minute_key_count = minute_keys.len() as i64 - 1;
            self.active_minute_stats.active_minute_count =
                Some(active_minute_count + i64::from(previous_minute_key_count == 0));
            self.active_minute_stats.parallel_count_sum += 1;
        }

        let overlay = ParallelWorkRuntimeOverlay {
            occurred_at,
            prompt_cache_key,
            bucket_start_epoch,
            bucket_changed,
            minute_changed,
            conversations_enabled: self.conversations_enabled,
            reporting_tz: self.reporting_tz,
            bucket_keys: &self.bucket_keys,
            active_minute_stats: self.active_minute_stats,
        };
        let mut changed = active_minute_stats_changed;
        for window in [
            &mut self.response.current,
            &mut self.response.minute7d,
            &mut self.response.hour30d,
            &mut self.response.day_all,
        ] {
            changed |= apply_parallel_work_runtime_overlay(window, &overlay);
        }
        changed
    }

    fn projected_active_minute_stats_at(
        &self,
        now: DateTime<Utc>,
    ) -> ParallelWorkActiveMinuteStats {
        let mut stats = self.baseline_active_minute_stats;
        let Some(mut active_minute_count) = stats.active_minute_count else {
            return stats;
        };
        let current_minute_start = now.timestamp().div_euclid(60) * 60;
        for (&minute_start_epoch, keys) in &self.minute_keys {
            if minute_start_epoch < self.baseline_complete_minute_start_epoch
                || minute_start_epoch >= current_minute_start
            {
                continue;
            }
            let baseline_keys = self.baseline_minute_keys.get(&minute_start_epoch);
            let baseline_key_count = baseline_keys.map_or(0, HashSet::len) as i64;
            let baseline_minute_was_complete =
                minute_start_epoch < self.baseline_complete_minute_end_epoch;
            if !baseline_minute_was_complete && baseline_key_count > 0 {
                active_minute_count += 1;
                stats.parallel_count_sum += baseline_key_count;
            }
            let runtime_key_count = keys
                .iter()
                .filter(|key| !baseline_keys.is_some_and(|keys| keys.contains(*key)))
                .count() as i64;
            if runtime_key_count == 0 {
                continue;
            }
            if baseline_key_count == 0 {
                active_minute_count += 1;
            }
            stats.parallel_count_sum += runtime_key_count;
        }
        stats.active_minute_count = Some(active_minute_count);
        stats
    }

    fn refresh_active_minute_stats_at(&mut self, now: DateTime<Utc>) -> bool {
        let stats = self.projected_active_minute_stats_at(now);
        if stats == self.active_minute_stats {
            return false;
        }
        self.active_minute_stats = stats;
        for window in [
            &mut self.response.current,
            &mut self.response.minute7d,
            &mut self.response.hour30d,
            &mut self.response.day_all,
        ] {
            window.active_minute_count = stats.active_minute_count;
            window.avg_count = stats.average();
        }
        true
    }

    fn rebuild_runtime_overlay(&mut self) {
        self.response = self.baseline_response.clone();
        self.bucket_keys = self.baseline_bucket_keys.clone();
        self.minute_keys = self.baseline_minute_keys.clone();
        self.active_minute_stats = self.baseline_active_minute_stats;
        self.applied_identities.clear();
        let runtime_mutations = self.runtime_mutations.clone();
        for (identity, mutation) in runtime_mutations {
            if self.apply_runtime_overlay(&mutation) {
                self.applied_identities.insert(identity);
            }
        }
        self.revision = self.revision.saturating_add(1);
    }

    fn requires_rolling_rebase(&self) -> bool {
        let Some(base_range_start) = parse_to_utc_datetime(&self.response.current.range_start)
        else {
            return true;
        };
        let Ok(current_range) = resolve_range_window(&self.range, self.reporting_tz) else {
            return true;
        };
        rolling_dashboard_window_requires_rebase(Some(base_range_start), Some(current_range.start))
            || self.projected_active_minute_stats_at(Utc::now()) != self.active_minute_stats
    }
}

#[derive(Debug)]
struct ParallelWorkRuntimeOverlay<'a> {
    occurred_at: DateTime<Utc>,
    prompt_cache_key: &'a str,
    bucket_start_epoch: i64,
    bucket_changed: bool,
    minute_changed: bool,
    conversations_enabled: bool,
    reporting_tz: Tz,
    bucket_keys: &'a BTreeMap<i64, HashSet<String>>,
    active_minute_stats: ParallelWorkActiveMinuteStats,
}

fn apply_parallel_work_runtime_overlay(
    window: &mut ParallelWorkWindowResponse,
    overlay: &ParallelWorkRuntimeOverlay<'_>,
) -> bool {
    let Some(range_start) = parse_to_utc_datetime(&window.range_start) else {
        return false;
    };
    if overlay.occurred_at < range_start {
        return false;
    }

    let mut changed = false;
    if overlay.bucket_changed {
        changed |= refresh_parallel_work_points(
            window,
            overlay.bucket_start_epoch,
            overlay.reporting_tz,
            overlay.bucket_keys,
        );
    }

    if overlay.conversations_enabled {
        let bucket_seconds = window.bucket_seconds;
        let effective_time_zone = window.effective_time_zone.clone();
        let conversation_changed = apply_parallel_work_conversation_overlay(
            &mut window.conversations,
            overlay.prompt_cache_key,
            overlay.occurred_at,
            bucket_seconds,
            effective_time_zone.as_str(),
        );
        changed |= conversation_changed;
    }

    if overlay.minute_changed {
        window.active_minute_count = overlay.active_minute_stats.active_minute_count;
        window.avg_count = overlay.active_minute_stats.average();
        changed = true;
    }
    changed
}

fn refresh_parallel_work_points(
    window: &mut ParallelWorkWindowResponse,
    bucket_start_epoch: i64,
    reporting_tz: Tz,
    bucket_keys: &BTreeMap<i64, HashSet<String>>,
) -> bool {
    let mut changed = false;
    if let Some(point) = window.points.iter_mut().find(|point| {
        parse_to_utc_datetime(&point.bucket_start)
            .is_some_and(|start| start.timestamp() == bucket_start_epoch)
    }) {
        let next_count = bucket_keys
            .get(&bucket_start_epoch)
            .map_or(0, |keys| keys.len() as i64);
        if point.parallel_count != next_count {
            point.parallel_count = next_count;
            changed = true;
        }
    } else {
        let Some(last_point) = window.points.last() else {
            return false;
        };
        let Some(mut cursor) = parse_to_utc_datetime(&last_point.bucket_end) else {
            return false;
        };
        while cursor.timestamp() <= bucket_start_epoch {
            let Ok(next_epoch) = next_reporting_bucket_epoch(
                cursor.timestamp(),
                window.bucket_seconds,
                reporting_tz,
            ) else {
                return changed;
            };
            let Some(next) = Utc.timestamp_opt(next_epoch, 0).single() else {
                return changed;
            };
            let parallel_count = bucket_keys
                .get(&cursor.timestamp())
                .map_or(0, |keys| keys.len() as i64);
            window.points.push(ParallelWorkPoint {
                bucket_start: format_utc_iso(cursor),
                bucket_end: format_utc_iso(next),
                parallel_count,
            });
            window.range_end = format_utc_iso(next);
            cursor = next;
            changed = true;
        }
    }
    if changed {
        refresh_parallel_work_point_totals(window);
    }
    changed
}

fn refresh_parallel_work_point_totals(window: &mut ParallelWorkWindowResponse) {
    let mut active_bucket_count = 0_i64;
    let mut min_count = None;
    let mut max_count = None;
    for point in &window.points {
        if point.parallel_count > 0 {
            active_bucket_count += 1;
        }
        min_count = Some(min_count.map_or(point.parallel_count, |value: i64| {
            value.min(point.parallel_count)
        }));
        max_count = Some(max_count.map_or(point.parallel_count, |value: i64| {
            value.max(point.parallel_count)
        }));
    }
    window.active_bucket_count = active_bucket_count;
    window.complete_bucket_count = window.points.len() as i64;
    window.min_count = min_count;
    window.max_count = max_count;
}

fn apply_parallel_work_conversation_overlay(
    conversations: &mut Vec<ParallelWorkConversation>,
    prompt_cache_key: &str,
    occurred_at: DateTime<Utc>,
    bucket_seconds: i64,
    effective_time_zone: &str,
) -> bool {
    let Ok(reporting_tz) = parse_reporting_tz(Some(effective_time_zone)) else {
        return false;
    };
    let Ok(start_epoch) =
        align_reporting_bucket_epoch(occurred_at.timestamp(), bucket_seconds, reporting_tz)
    else {
        return false;
    };
    let Ok(end_epoch) = next_reporting_bucket_epoch(start_epoch, bucket_seconds, reporting_tz)
    else {
        return false;
    };
    let Some(start) = Utc.timestamp_opt(start_epoch, 0).single() else {
        return false;
    };
    let Some(end) = Utc.timestamp_opt(end_epoch, 0).single() else {
        return false;
    };

    if let Some(conversation) = conversations
        .iter_mut()
        .find(|conversation| conversation.conversation_id == prompt_cache_key)
    {
        conversation.request_count = conversation.request_count.saturating_add(1);
        if parse_to_utc_datetime(&conversation.start).is_some_and(|current| start < current) {
            conversation.start = format_utc_iso(start);
        }
        if parse_to_utc_datetime(&conversation.end).is_some_and(|current| end > current) {
            conversation.end = format_utc_iso(end);
        }
    } else {
        conversations.push(ParallelWorkConversation {
            conversation_id: prompt_cache_key.to_string(),
            start: format_utc_iso(start),
            end: format_utc_iso(end),
            request_count: 1,
        });
    }
    conversations.sort_by(|left, right| {
        right
            .end
            .cmp(&left.end)
            .then_with(|| right.request_count.cmp(&left.request_count))
            .then_with(|| left.conversation_id.cmp(&right.conversation_id))
    });
    conversations.truncate(80);
    true
}

async fn build_dashboard_parallel_work_materializer_state(
    state: &Arc<AppState>,
    query: ParallelWorkStatsQuery,
) -> Result<DashboardParallelWorkMaterializerState, ApiError> {
    for _ in 0..3 {
        let mut observer = state.pool.acquire().await?;
        let version_before = sqlx::query_scalar::<_, i64>("PRAGMA data_version")
            .fetch_one(&mut *observer)
            .await?;
        let baseline_row_id =
            sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
                .fetch_one(&mut *observer)
                .await?;
        let materializer = build_dashboard_parallel_work_materializer_state_at_baseline(
            state,
            query.clone(),
            baseline_row_id,
        )
        .await?;
        let version_after = sqlx::query_scalar::<_, i64>("PRAGMA data_version")
            .fetch_one(&mut *observer)
            .await?;
        if version_before == version_after {
            return Ok(materializer);
        }
    }
    Err(ApiError::from(anyhow!(
        "parallel-work baseline changed during build"
    )))
}

async fn build_dashboard_parallel_work_materializer_state_at_baseline(
    state: &Arc<AppState>,
    query: ParallelWorkStatsQuery,
    baseline_row_id: i64,
) -> Result<DashboardParallelWorkMaterializerState, ApiError> {
    let ParallelWorkProjectionBaseline {
        response,
        bucket_keys,
        active_minute_stats,
    } = load_parallel_work_projection_baseline(state, query.clone()).await?;
    let current = &response.current;
    let range_start = parse_to_utc_datetime(&current.range_start)
        .ok_or_else(|| ApiError::from(anyhow!("invalid parallel-work range start")))?;
    let range_end = parse_to_utc_datetime(&current.range_end)
        .ok_or_else(|| ApiError::from(anyhow!("invalid parallel-work range end")))?;
    let reporting_tz = parse_reporting_tz(Some(current.effective_time_zone.as_str()))?;
    let requested_reporting_tz = parse_reporting_tz(query.time_zone.as_deref())?;
    let conversations_enabled = resolve_range_window(&query.range, requested_reporting_tz)?
        .duration
        <= ChronoDuration::hours(24);
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let range_start_epoch = range_start.timestamp();
    let minute_keys = query_parallel_work_exact_key_sets(
        &state.pool,
        ParallelWorkExactKeySetsQuery {
            range_start,
            range_end,
            bucket_seconds: 60,
            reporting_tz: chrono_tz::UTC,
            source_scope,
            upstream_account_id: query.upstream_account_id,
            start_after_id: None,
            snapshot_id: None,
        },
    )
    .await?;
    Ok(DashboardParallelWorkMaterializerState {
        baseline_response: response.clone(),
        response,
        baseline_bucket_keys: bucket_keys.clone(),
        bucket_keys,
        baseline_minute_keys: minute_keys.clone(),
        minute_keys,
        baseline_active_minute_stats: active_minute_stats,
        active_minute_stats,
        baseline_complete_minute_start_epoch: if range_start_epoch.rem_euclid(60) == 0 {
            range_start_epoch
        } else {
            range_start_epoch.div_euclid(60) * 60 + 60
        },
        baseline_complete_minute_end_epoch: range_end.timestamp().div_euclid(60) * 60,
        baseline_row_id,
        range: query.range,
        reporting_tz,
        upstream_account_id: query.upstream_account_id,
        conversations_enabled,
        baseline_identities: HashSet::new(),
        applied_identities: HashSet::new(),
        runtime_mutations: BTreeMap::new(),
        revision: 0,
    })
}

impl DashboardSummaryMaterializerState {
    fn new(
        response: StatsResponse,
        terminal_sequence: u64,
        range_start: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            response,
            current_revision: None,
            terminal_revision: None,
            terminal_sequence,
            range_start,
        }
    }

    fn from_summary_projection(
        response: StatsResponse,
        _initial_terminal_slice_suppressions: HashSet<u64>,
        range_start: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            response,
            current_revision: None,
            terminal_revision: None,
            terminal_sequence: 0,
            range_start,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkingConversationsProjectionUpdate {
    Unchanged,
    Changed,
    NeedsBoundedKeyHydration(BTreeSet<String>),
    NeedsReconcile,
}

#[derive(Debug)]
struct DashboardWorkingConversationsMaterializerState {
    response: PromptCacheConversationsResponse,
    page_size: usize,
    recent_invocation_limit: usize,
    blocked_binding_filter: Option<PromptCacheConversationBlockedBindingFilter>,
    baseline_has_more: bool,
    in_flight_by_identity: HashMap<String, PromptCacheInFlightPhaseRecord>,
    in_flight_phase_counts_by_key: HashMap<String, InvocationPhaseCountsResponse>,
}
