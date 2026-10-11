use super::*;

#[derive(Debug, Clone)]

pub(crate) struct ReplayableTopicEvent {
    pub(crate) frame: Arc<SerializedTopicFrame>,
    pub(crate) bytes: usize,
    pub(crate) emitted_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DashboardTopicRevision {
    pub(crate) base_revision: u64,
    pub(crate) current_revision: Option<u64>,
    pub(crate) network_revision: Option<u64>,
    pub(crate) terminal_revision: Option<u64>,
    pub(crate) routing_revision: u64,
}

#[derive(Debug)]
pub(crate) struct DashboardActivityMaterializerState {
    pub(crate) base: DashboardActivityTopicMaterializedBase,
    pub(crate) rebase_range_start: Option<DateTime<Utc>>,
    pub(crate) current_revision: Option<u64>,
    pub(crate) network_revision: Option<u64>,
    pub(crate) terminal_revision: Option<u64>,
    pub(crate) routing_revision: u64,
}

impl DashboardActivityMaterializerState {
    pub(crate) fn new(base: DashboardActivityTopicMaterializedBase) -> Self {
        let rebase_range_start = parse_to_utc_datetime(&base.response().range_start);
        Self {
            base,
            rebase_range_start,
            current_revision: None,
            network_revision: None,
            terminal_revision: None,
            routing_revision: 0,
        }
    }
}

#[derive(Debug)]
pub(crate) struct DashboardSummaryMaterializerState {
    pub(crate) response: StatsResponse,
    pub(crate) current_revision: Option<u64>,
    pub(crate) terminal_revision: Option<u64>,
    pub(crate) terminal_sequence: u64,
    pub(crate) range_start: Option<DateTime<Utc>>,
}

#[derive(Debug)]
pub(crate) struct DashboardParallelWorkMaterializerState {
    pub(crate) response: ParallelWorkStatsResponse,
    pub(crate) baseline_response: ParallelWorkStatsResponse,
    pub(crate) bucket_keys: BTreeMap<i64, HashSet<String>>,
    pub(crate) baseline_bucket_keys: BTreeMap<i64, HashSet<String>>,
    pub(crate) minute_keys: BTreeMap<i64, HashSet<String>>,
    pub(crate) baseline_minute_keys: BTreeMap<i64, HashSet<String>>,
    pub(crate) active_minute_stats: ParallelWorkActiveMinuteStats,
    pub(crate) baseline_active_minute_stats: ParallelWorkActiveMinuteStats,
    pub(crate) baseline_complete_minute_start_epoch: i64,
    pub(crate) baseline_complete_minute_end_epoch: i64,
    pub(crate) baseline_row_id: i64,
    pub(crate) range: String,
    pub(crate) reporting_tz: Tz,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) conversations_enabled: bool,
    pub(crate) baseline_identities: HashSet<String>,
    pub(crate) applied_identities: HashSet<String>,
    pub(crate) runtime_mutations: BTreeMap<String, RuntimeInvocationMutation>,
    pub(crate) revision: u64,
}

#[derive(Debug, Default)]
pub(crate) struct ParallelWorkMutationOutcome {
    pub(crate) changed: bool,
    pub(crate) needs_account_reconcile: bool,
}

impl DashboardParallelWorkMaterializerState {
    pub(crate) fn apply_runtime_mutation(
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

    pub(crate) fn replay_runtime_mutations(
        &mut self,
        mutations: &BTreeMap<String, RuntimeInvocationMutation>,
    ) -> bool {
        let mut changed = false;
        for mutation in mutations.values() {
            changed |= self.apply_runtime_mutation(mutation).changed;
        }
        changed
    }

    pub(crate) fn apply_runtime_overlay(&mut self, mutation: &RuntimeInvocationMutation) -> bool {
        self.apply_runtime_overlay_at(mutation, Utc::now())
    }

    pub(crate) fn apply_runtime_overlay_at(
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

    pub(crate) fn projected_active_minute_stats_at(
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

    pub(crate) fn refresh_active_minute_stats_at(&mut self, now: DateTime<Utc>) -> bool {
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

    pub(crate) fn rebuild_runtime_overlay(&mut self) {
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

    pub(crate) fn requires_rolling_rebase(&self) -> bool {
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
pub(crate) struct ParallelWorkRuntimeOverlay<'a> {
    pub(crate) occurred_at: DateTime<Utc>,
    pub(crate) prompt_cache_key: &'a str,
    pub(crate) bucket_start_epoch: i64,
    pub(crate) bucket_changed: bool,
    pub(crate) minute_changed: bool,
    pub(crate) conversations_enabled: bool,
    pub(crate) reporting_tz: Tz,
    pub(crate) bucket_keys: &'a BTreeMap<i64, HashSet<String>>,
    pub(crate) active_minute_stats: ParallelWorkActiveMinuteStats,
}

pub(crate) fn apply_parallel_work_runtime_overlay(
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

pub(crate) fn refresh_parallel_work_points(
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

pub(crate) fn refresh_parallel_work_point_totals(window: &mut ParallelWorkWindowResponse) {
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

pub(crate) fn apply_parallel_work_conversation_overlay(
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

pub(crate) async fn build_dashboard_parallel_work_materializer_state(
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

pub(crate) async fn build_dashboard_parallel_work_materializer_state_at_baseline(
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
        range_start,
        range_end,
        60,
        chrono_tz::UTC,
        source_scope,
        query.upstream_account_id,
        None,
        None,
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
    pub(crate) fn new(
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

    pub(crate) fn from_summary_projection(
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
pub(crate) enum WorkingConversationsProjectionUpdate {
    Unchanged,
    Changed,
    NeedsBoundedKeyHydration(BTreeSet<String>),
    NeedsReconcile,
}

#[derive(Debug)]
pub(crate) struct DashboardWorkingConversationsMaterializerState {
    pub(crate) response: PromptCacheConversationsResponse,
    pub(crate) page_size: usize,
    pub(crate) recent_invocation_limit: usize,
    pub(crate) blocked_binding_filter: Option<PromptCacheConversationBlockedBindingFilter>,
    pub(crate) baseline_has_more: bool,
    pub(crate) in_flight_by_identity: HashMap<String, PromptCacheInFlightPhaseRecord>,
    pub(crate) in_flight_phase_counts_by_key: HashMap<String, InvocationPhaseCountsResponse>,
}

impl DashboardWorkingConversationsMaterializerState {
    pub(crate) fn new(
        response: PromptCacheConversationsResponse,
        page_size: i64,
        recent_invocation_limit: i64,
        blocked_binding_filter: Option<PromptCacheConversationBlockedBindingFilter>,
    ) -> Self {
        let baseline_has_more = response.has_more;
        let in_flight_phase_counts_by_key = response
            .conversations
            .iter()
            .map(|conversation| {
                (
                    conversation.prompt_cache_key.clone(),
                    conversation.in_flight_phase_counts,
                )
            })
            .collect();
        Self {
            response,
            page_size: page_size.max(0) as usize,
            recent_invocation_limit: recent_invocation_limit.max(0) as usize,
            blocked_binding_filter,
            baseline_has_more,
            in_flight_by_identity: HashMap::new(),
            in_flight_phase_counts_by_key,
        }
    }

    pub(crate) fn install_in_flight_phase_records(
        &mut self,
        records: impl IntoIterator<Item = PromptCacheInFlightPhaseRecord>,
    ) {
        self.in_flight_by_identity.clear();
        self.in_flight_phase_counts_by_key.clear();
        for record in records {
            self.in_flight_phase_counts_by_key
                .entry(record.prompt_cache_key.clone())
                .or_default()
                .increment_phase_name(record.phase.as_deref());
            self.in_flight_by_identity
                .insert(record.identity.clone(), record);
        }
        self.sync_in_flight_phase_counts_to_response();
    }

    pub(crate) fn sync_in_flight_phase_counts_to_response(&mut self) {
        for conversation in &mut self.response.conversations {
            conversation.in_flight_phase_counts = self
                .in_flight_phase_counts_by_key
                .get(&conversation.prompt_cache_key)
                .copied()
                .unwrap_or_default();
        }
    }

    pub(crate) fn apply_in_flight_phase_delta(&mut self, record: &PromptCacheTopicDelta) -> bool {
        let mut affected_keys = BTreeSet::new();
        if let Some(previous) = self.in_flight_by_identity.remove(&record.identity) {
            self.in_flight_phase_counts_by_key
                .entry(previous.prompt_cache_key.clone())
                .or_default()
                .decrement_phase_name(previous.phase.as_deref());
            affected_keys.insert(previous.prompt_cache_key);
        }
        let is_in_flight = !record.is_runtime_removed
            && !record.is_terminal
            && matches!(
                record.status.trim().to_ascii_lowercase().as_str(),
                "running" | "pending"
            );
        if is_in_flight
            && let Some(prompt_cache_key) = record
                .prompt_cache_key
                .as_deref()
                .filter(|key| !key.is_empty())
        {
            let phase = record
                .preview
                .as_ref()
                .and_then(|preview| preview.live_phase.clone());
            let entry = PromptCacheInFlightPhaseRecord {
                identity: record.identity.clone(),
                prompt_cache_key: prompt_cache_key.to_string(),
                phase,
            };
            self.in_flight_phase_counts_by_key
                .entry(entry.prompt_cache_key.clone())
                .or_default()
                .increment_phase_name(entry.phase.as_deref());
            affected_keys.insert(entry.prompt_cache_key.clone());
            self.in_flight_by_identity
                .insert(entry.identity.clone(), entry);
        }
        let mut changed = false;
        for key in affected_keys {
            if let Some(conversation) = self
                .response
                .conversations
                .iter_mut()
                .find(|conversation| conversation.prompt_cache_key == key)
            {
                let next = self
                    .in_flight_phase_counts_by_key
                    .get(&key)
                    .copied()
                    .unwrap_or_default();
                if conversation.in_flight_phase_counts != next {
                    conversation.in_flight_phase_counts = next;
                    changed = true;
                }
            }
        }
        changed
    }

    pub(crate) fn apply_deltas(
        &mut self,
        records: &[PromptCacheTopicDelta],
        applied_terminal_ids: &mut HashSet<String>,
        baseline_row_id: i64,
    ) -> Result<WorkingConversationsProjectionUpdate, ApiError> {
        let now = Utc::now();
        let mut bounded_hydration_keys = BTreeSet::new();
        let mut reconcile_required = false;
        for record in records {
            let Some(prompt_cache_key) = record.prompt_cache_key.as_deref() else {
                continue;
            };
            if record.is_runtime_removed {
                continue;
            }
            let conversation = self
                .response
                .conversations
                .iter()
                .find(|conversation| conversation.prompt_cache_key == prompt_cache_key);
            let conversation_is_visible = conversation.is_some();
            let Some(preview) = record.preview.as_ref() else {
                reconcile_required = true;
                continue;
            };
            if !self.matches_blocked_binding_filter(record, preview)
                || !Self::is_in_working_window(record, now)
            {
                if conversation_is_visible {
                    bounded_hydration_keys.insert(prompt_cache_key.to_string());
                }
                continue;
            }
            if !conversation_is_visible {
                // The compact delta carries only the changing invocation. Hydration owns all
                // historical totals, charts, account and owner metadata for a newly selected
                // key before it can be applied to the typed projection.
                bounded_hydration_keys.insert(prompt_cache_key.to_string());
            } else if conversation.is_some_and(|conversation| {
                Self::delta_requires_account_hydration(
                    conversation,
                    record,
                    applied_terminal_ids,
                    baseline_row_id,
                )
            }) {
                // The wire response retains only the top account summaries. Once it is full,
                // a newly observed account may have historical totals that were omitted from
                // the response, so only the bounded key hydrate can update it exactly.
                bounded_hydration_keys.insert(prompt_cache_key.to_string());
            }
        }
        if reconcile_required {
            return Ok(WorkingConversationsProjectionUpdate::NeedsReconcile);
        }
        if !bounded_hydration_keys.is_empty() {
            return Ok(
                WorkingConversationsProjectionUpdate::NeedsBoundedKeyHydration(
                    bounded_hydration_keys,
                ),
            );
        }

        let mut changed = false;

        for record in records {
            changed |= self.apply_in_flight_phase_delta(record);
        }

        for record in records {
            let Some(prompt_cache_key) = record.prompt_cache_key.as_deref() else {
                continue;
            };
            let conversation_index = self
                .response
                .conversations
                .iter()
                .position(|conversation| conversation.prompt_cache_key == prompt_cache_key);

            if record.is_runtime_removed {
                if let Some(index) = conversation_index {
                    changed |= self.remove_runtime_preview(index, record);
                }
                continue;
            }

            let Some(preview) = record.preview.as_ref() else {
                unreachable!("bounded hydration preflight must handle incomplete deltas");
            };
            if !self.matches_blocked_binding_filter(record, preview)
                || !Self::is_in_working_window(record, now)
            {
                continue;
            }
            let conversation_index = conversation_index.expect(
                "bounded hydration preflight must hydrate a missing working conversation key",
            );

            let conversation = &mut self.response.conversations[conversation_index];
            changed |= apply_working_conversation_delta(
                conversation,
                record,
                preview,
                self.recent_invocation_limit,
                applied_terminal_ids,
                baseline_row_id,
            );
        }

        if changed {
            self.refresh_pagination();
        }

        Ok(if changed {
            WorkingConversationsProjectionUpdate::Changed
        } else {
            WorkingConversationsProjectionUpdate::Unchanged
        })
    }

    pub(crate) fn apply_binding(
        &mut self,
        prompt_cache_key: &str,
        binding: &PromptCacheConversationBindingResponse,
    ) -> Option<bool> {
        let conversation = self
            .response
            .conversations
            .iter_mut()
            .find(|conversation| conversation.prompt_cache_key == prompt_cache_key)?;
        let manual_binding = (binding.binding_kind != "none").then(|| {
            PromptCacheConversationManualBindingResponse {
                binding_kind: binding.binding_kind.clone(),
                group_name: binding.group_name.clone(),
                upstream_account_id: binding.upstream_account_id,
                upstream_account_name: binding.upstream_account_name.clone(),
            }
        });
        let changed = conversation.has_encrypted_session_owner
            != binding.has_encrypted_session_owner
            || conversation.encrypted_owner_account_id != binding.encrypted_owner_account_id
            || conversation.encrypted_owner_account_name != binding.encrypted_owner_account_name
            || conversation.encrypted_owner_group_name != binding.encrypted_owner_group_name
            || conversation.manual_binding != manual_binding;
        if changed {
            conversation.has_encrypted_session_owner = binding.has_encrypted_session_owner;
            conversation.encrypted_owner_account_id = binding.encrypted_owner_account_id;
            conversation.encrypted_owner_account_name =
                binding.encrypted_owner_account_name.clone();
            conversation.encrypted_owner_group_name = binding.encrypted_owner_group_name.clone();
            conversation.manual_binding = manual_binding;
        }
        Some(changed)
    }

    pub(crate) fn serialize(&self) -> Result<Vec<u8>, ApiError> {
        serde_json::to_vec(&self.response).map_err(ApiError::from)
    }

    pub(crate) fn matches_blocked_binding_filter(
        &self,
        record: &PromptCacheTopicDelta,
        preview: &PromptCacheConversationInvocationPreviewResponse,
    ) -> bool {
        let Some(filter) = self.blocked_binding_filter.as_ref() else {
            return true;
        };
        if !filter.is_active() || !record.is_terminal {
            return !filter.is_active();
        }
        let Some(blocked_binding) = preview.blocked_binding.as_ref() else {
            return false;
        };
        filter
            .upstream_account_id
            .is_none_or(|account_id| account_id == blocked_binding.upstream_account_id)
            && filter
                .constraint_source
                .is_none_or(|source| source == blocked_binding.constraint_source)
    }

    pub(crate) fn is_in_working_window(record: &PromptCacheTopicDelta, now: DateTime<Utc>) -> bool {
        !record.is_terminal
            || parse_to_utc_datetime(&record.occurred_at).is_some_and(|occurred_at| {
                occurred_at
                    >= now
                        - ChronoDuration::minutes(
                            SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                        )
            })
    }

    pub(crate) fn delta_requires_account_hydration(
        conversation: &PromptCacheConversationResponse,
        record: &PromptCacheTopicDelta,
        applied_terminal_ids: &HashSet<String>,
        baseline_row_id: i64,
    ) -> bool {
        let already_terminal = applied_terminal_ids.contains(&record.identity)
            || (record.row_id > 0 && record.row_id <= baseline_row_id);
        if !record.is_terminal
            || already_terminal
            || conversation.upstream_accounts.len()
                < PROMPT_CACHE_CONVERSATION_UPSTREAM_ACCOUNT_LIMIT
        {
            return false;
        }
        let account_group_key = resolve_prompt_cache_upstream_account_group_key(
            record.upstream_account_id,
            record.upstream_account_name.as_deref(),
        );
        !conversation.upstream_accounts.iter().any(|account| {
            resolve_prompt_cache_upstream_account_group_key(
                account.upstream_account_id,
                account.upstream_account_name.as_deref(),
            ) == account_group_key
        })
    }

    pub(crate) fn delta_is_eligible(
        &self,
        record: &PromptCacheTopicDelta,
        now: DateTime<Utc>,
    ) -> bool {
        !record.is_runtime_removed
            && record.preview.as_ref().is_some_and(|preview| {
                self.matches_blocked_binding_filter(record, preview)
                    && Self::is_in_working_window(record, now)
            })
    }

    pub(crate) fn replace_hydrated_conversation(
        &mut self,
        prompt_cache_key: &str,
        hydrated: Option<PromptCacheConversationResponse>,
    ) -> bool {
        let existing_index = self
            .response
            .conversations
            .iter()
            .position(|conversation| conversation.prompt_cache_key == prompt_cache_key);
        let changed = match (existing_index, hydrated) {
            (Some(index), Some(hydrated)) => {
                if self.response.conversations[index] == hydrated {
                    false
                } else {
                    self.response.conversations[index] = hydrated;
                    true
                }
            }
            (Some(index), None) => {
                self.response.conversations.remove(index);
                true
            }
            (None, Some(hydrated)) => {
                self.response.conversations.push(hydrated);
                true
            }
            (None, None) => false,
        };
        if changed {
            self.refresh_pagination();
        }
        changed
    }

    pub(crate) fn set_total_matched(&mut self, total_matched: i64) -> bool {
        if self.response.total_matched == Some(total_matched) {
            return false;
        }
        self.response.total_matched = Some(total_matched);
        self.refresh_pagination();
        true
    }

    pub(crate) fn visible_keys(&self) -> HashSet<String> {
        self.response
            .conversations
            .iter()
            .map(|conversation| conversation.prompt_cache_key.clone())
            .collect()
    }

    pub(crate) fn refresh_pagination(&mut self) {
        // The cold baseline owns continuation cursor semantics. Live deltas may reorder the
        // active display page, but they never manufacture a cursor for that changed ordering:
        // such a cursor would be applied to the original database snapshot and skip or repeat
        // rows. Fresh page-size subscriptions establish a new baseline when needed.
        sort_working_conversation_responses(&mut self.response.conversations);
        let overflowed = self.response.conversations.len() > self.page_size;
        self.response.conversations.truncate(self.page_size);
        self.response.has_more = self.baseline_has_more
            || overflowed
            || self.response.total_matched.is_some_and(|total_matched| {
                total_matched > self.response.conversations.len() as i64
            });
        for conversation in &mut self.response.conversations {
            conversation.cursor = None;
        }
        self.response.next_cursor = None;
    }

    pub(crate) fn remove_runtime_preview(
        &mut self,
        conversation_index: usize,
        record: &PromptCacheTopicDelta,
    ) -> bool {
        let conversation = &mut self.response.conversations[conversation_index];
        let before = conversation.recent_invocations.len();
        conversation.recent_invocations.retain(|preview| {
            preview.invoke_id != record.invoke_id
                || !working_timestamp_cmp(&preview.occurred_at, &record.occurred_at).is_eq()
        });
        if conversation.recent_invocations.len() == before {
            return false;
        }

        conversation.last_in_flight_at = working_conversation_latest_in_flight_at(conversation);
        conversation.blocked_binding = conversation
            .recent_invocations
            .iter()
            .find_map(|preview| preview.blocked_binding.clone());
        if let Some(last_activity_at) = working_conversation_latest_activity_at(conversation) {
            conversation.last_activity_at = last_activity_at;
        }

        if conversation.request_count == 0
            && let Some(first_preview) = conversation.recent_invocations.last()
        {
            conversation.created_at = first_preview.occurred_at.clone();
        }

        if conversation.request_count == 0 && conversation.recent_invocations.is_empty() {
            self.response.conversations.remove(conversation_index);
            if let Some(total_matched) = self.response.total_matched.as_mut() {
                *total_matched = total_matched.saturating_sub(1);
            }
        }
        true
    }

    pub(crate) fn expire(&mut self, now: DateTime<Utc>) -> bool {
        let activity_cutoff = now
            - ChronoDuration::minutes(SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES);
        let request_cutoff = now - ChronoDuration::hours(24);
        let before = self.response.conversations.len();
        self.response.conversations.retain(|conversation| {
            conversation.last_in_flight_at.is_some()
                || parse_to_utc_datetime(&conversation.last_activity_at)
                    .is_some_and(|last_activity_at| last_activity_at >= activity_cutoff)
        });
        let removed = before.saturating_sub(self.response.conversations.len());
        if removed > 0
            && let Some(total_matched) = self.response.total_matched.as_mut()
        {
            *total_matched = total_matched.saturating_sub(removed as i64);
        }
        let mut changed = removed > 0;
        for conversation in &mut self.response.conversations {
            let before = conversation.last24h_requests.len();
            conversation.last24h_requests.retain(|point| {
                parse_to_utc_datetime(&point.occurred_at)
                    .is_some_and(|occurred_at| occurred_at >= request_cutoff)
            });
            changed |= conversation.last24h_requests.len() != before;
            let mut cumulative_tokens = 0_i64;
            for point in &mut conversation.last24h_requests {
                cumulative_tokens = cumulative_tokens.saturating_add(point.request_tokens);
                point.cumulative_tokens = cumulative_tokens;
            }
        }
        if changed {
            self.refresh_pagination();
        }
        changed
    }
}

pub(crate) fn apply_working_conversation_delta(
    conversation: &mut PromptCacheConversationResponse,
    record: &PromptCacheTopicDelta,
    preview: &PromptCacheConversationInvocationPreviewResponse,
    recent_invocation_limit: usize,
    applied_terminal_ids: &mut HashSet<String>,
    baseline_row_id: i64,
) -> bool {
    let mut changed = false;
    if let Some(index) = conversation.recent_invocations.iter().position(|existing| {
        existing.invoke_id == preview.invoke_id && existing.occurred_at == preview.occurred_at
    }) {
        if conversation.recent_invocations[index] != *preview {
            conversation.recent_invocations[index] = preview.clone();
            changed = true;
        }
    } else {
        conversation.recent_invocations.push(preview.clone());
        changed = true;
    }
    conversation.recent_invocations.sort_by(|left, right| {
        working_timestamp_cmp(&right.occurred_at, &left.occurred_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    if conversation.recent_invocations.len() > recent_invocation_limit {
        conversation
            .recent_invocations
            .truncate(recent_invocation_limit);
    }

    changed |= update_string_if_newer(&mut conversation.last_activity_at, &record.occurred_at);
    changed |= update_string_if_earlier(&mut conversation.created_at, &record.occurred_at);

    let already_terminal = applied_terminal_ids.contains(&record.identity)
        || (record.row_id > 0 && record.row_id <= baseline_row_id);
    if record.is_terminal && !already_terminal {
        applied_terminal_ids.insert(record.identity.clone());
        apply_working_conversation_terminal_delta(conversation, record, preview);
        changed = true;
    }
    if !record.is_terminal {
        changed |= update_option_if_newer(&mut conversation.last_in_flight_at, &record.occurred_at);
    } else {
        let latest_in_flight = working_conversation_latest_in_flight_at(conversation);
        if conversation.last_in_flight_at != latest_in_flight {
            conversation.last_in_flight_at = latest_in_flight;
            changed = true;
        }
    }

    let blocked_binding = conversation
        .recent_invocations
        .iter()
        .find_map(|candidate| candidate.blocked_binding.clone());
    if conversation.blocked_binding != blocked_binding {
        conversation.blocked_binding = blocked_binding;
        changed = true;
    }
    changed
}

pub(crate) fn working_conversation_latest_in_flight_at(
    conversation: &PromptCacheConversationResponse,
) -> Option<String> {
    conversation
        .recent_invocations
        .iter()
        .filter(|preview| {
            !prompt_invocation_status_counts_toward_terminal_totals(Some(&preview.status))
        })
        .map(|preview| preview.occurred_at.clone())
        .max_by(|left, right| working_timestamp_cmp(left, right))
}

pub(crate) fn working_conversation_latest_activity_at(
    conversation: &PromptCacheConversationResponse,
) -> Option<String> {
    conversation.recent_invocations.iter().fold(
        conversation.last_terminal_at.clone(),
        |latest, preview| match latest {
            Some(latest) if !working_timestamp_cmp(&preview.occurred_at, &latest).is_gt() => {
                Some(latest)
            }
            _ => Some(preview.occurred_at.clone()),
        },
    )
}

pub(crate) fn apply_working_conversation_terminal_delta(
    conversation: &mut PromptCacheConversationResponse,
    record: &PromptCacheTopicDelta,
    preview: &PromptCacheConversationInvocationPreviewResponse,
) {
    let statistics_baseline_is_known_empty =
        conversation.request_count == 0 && conversation.last_terminal_at.is_none();
    conversation.request_count = conversation.request_count.saturating_add(1);
    conversation.total_tokens = conversation
        .total_tokens
        .saturating_add(record.request_tokens.max(0));
    conversation.total_cost += record.cost;
    increment_optional_i64(
        &mut conversation.success_count,
        Some(i64::from(record.is_success)),
        statistics_baseline_is_known_empty,
    );
    increment_optional_i64(
        &mut conversation.failure_count,
        Some(i64::from(!record.is_success)),
        statistics_baseline_is_known_empty,
    );
    increment_optional_i64(
        &mut conversation.input_tokens,
        record.input_tokens,
        statistics_baseline_is_known_empty,
    );
    increment_optional_i64(
        &mut conversation.output_tokens,
        record.output_tokens,
        statistics_baseline_is_known_empty,
    );
    increment_optional_i64(
        &mut conversation.cache_input_tokens,
        record.cache_input_tokens,
        statistics_baseline_is_known_empty,
    );
    increment_optional_i64(
        &mut conversation.reported_cache_write_tokens,
        record.reported_cache_write_tokens,
        statistics_baseline_is_known_empty,
    );
    increment_optional_i64(
        &mut conversation.reasoning_tokens,
        record.reasoning_tokens,
        statistics_baseline_is_known_empty,
    );
    increment_optional_f64(
        &mut conversation.cost_input,
        record.cost_input,
        statistics_baseline_is_known_empty,
    );
    increment_optional_f64(
        &mut conversation.cost_cache_write,
        record.cost_cache_write,
        statistics_baseline_is_known_empty,
    );
    increment_optional_f64(
        &mut conversation.cost_cache_read,
        record.cost_cache_read,
        statistics_baseline_is_known_empty,
    );
    increment_optional_f64(
        &mut conversation.cost_output,
        record.cost_output,
        statistics_baseline_is_known_empty,
    );
    increment_optional_f64(
        &mut conversation.cost_reasoning,
        record.cost_reasoning,
        statistics_baseline_is_known_empty,
    );
    if statistics_baseline_is_known_empty || conversation.first_invocation_at.is_some() {
        update_option_if_earlier(&mut conversation.first_invocation_at, &record.occurred_at);
    }
    if statistics_baseline_is_known_empty || conversation.last_invocation_at.is_some() {
        update_option_if_newer(&mut conversation.last_invocation_at, &record.occurred_at);
    }
    update_option_if_newer(&mut conversation.last_terminal_at, &record.occurred_at);

    let account_group_key = resolve_prompt_cache_upstream_account_group_key(
        record.upstream_account_id,
        record.upstream_account_name.as_deref(),
    );
    let account_index = conversation
        .upstream_accounts
        .iter()
        .position(|account| {
            resolve_prompt_cache_upstream_account_group_key(
                account.upstream_account_id,
                account.upstream_account_name.as_deref(),
            ) == account_group_key
        })
        .unwrap_or_else(|| {
            conversation
                .upstream_accounts
                .push(PromptCacheConversationUpstreamAccountResponse {
                    upstream_account_id: record.upstream_account_id,
                    upstream_account_name: record.upstream_account_name.clone(),
                    request_count: 0,
                    total_tokens: 0,
                    total_cost: 0.0,
                    last_activity_at: record.occurred_at.clone(),
                });
            conversation.upstream_accounts.len() - 1
        });
    let account = &mut conversation.upstream_accounts[account_index];
    if account.upstream_account_id.is_none() && record.upstream_account_id.is_some() {
        account.upstream_account_id = record.upstream_account_id;
    }
    if account.upstream_account_name.is_none() && record.upstream_account_name.is_some() {
        account.upstream_account_name = record.upstream_account_name.clone();
    }
    account.request_count = account.request_count.saturating_add(1);
    account.total_tokens = account
        .total_tokens
        .saturating_add(record.request_tokens.max(0));
    account.total_cost += record.cost;
    update_string_if_newer(&mut account.last_activity_at, &record.occurred_at);
    conversation.upstream_accounts.sort_by(|left, right| {
        working_timestamp_cmp(&right.last_activity_at, &left.last_activity_at)
            .then_with(|| {
                resolve_prompt_cache_upstream_account_label(
                    right.upstream_account_name.as_deref(),
                    right.upstream_account_id,
                )
                .cmp(&resolve_prompt_cache_upstream_account_label(
                    left.upstream_account_name.as_deref(),
                    left.upstream_account_id,
                ))
            })
            .then_with(|| {
                right
                    .upstream_account_id
                    .unwrap_or(i64::MIN)
                    .cmp(&left.upstream_account_id.unwrap_or(i64::MIN))
            })
            .then_with(|| right.total_tokens.cmp(&left.total_tokens))
            .then_with(|| right.request_count.cmp(&left.request_count))
    });
    conversation
        .upstream_accounts
        .truncate(PROMPT_CACHE_CONVERSATION_UPSTREAM_ACCOUNT_LIMIT);

    let outcome = invocation_point_outcome(
        Some(&record.status),
        preview.error_message.as_deref(),
        preview.downstream_error_message.as_deref(),
        preview.failure_kind.as_deref(),
        preview.failure_class.as_deref(),
    )
    .to_string();
    conversation
        .last24h_requests
        .push(PromptCacheConversationRequestPointResponse {
            occurred_at: record.occurred_at.clone(),
            status: if record.status.trim().is_empty() {
                "unknown".to_string()
            } else {
                record.status.clone()
            },
            is_success: outcome == "success",
            outcome,
            request_tokens: record.request_tokens.max(0),
            cumulative_tokens: 0,
        });
    conversation
        .last24h_requests
        .sort_by(|left, right| working_timestamp_cmp(&left.occurred_at, &right.occurred_at));
    let mut cumulative_tokens = 0_i64;
    for point in &mut conversation.last24h_requests {
        cumulative_tokens = cumulative_tokens.saturating_add(point.request_tokens);
        point.cumulative_tokens = cumulative_tokens;
    }
}

pub(crate) fn sort_working_conversation_responses(
    conversations: &mut [PromptCacheConversationResponse],
) {
    conversations.sort_by(|left, right| {
        working_timestamp_cmp(
            working_conversation_sort_anchor(right),
            working_conversation_sort_anchor(left),
        )
        .then_with(|| working_timestamp_cmp(&right.created_at, &left.created_at))
        .then_with(|| right.prompt_cache_key.cmp(&left.prompt_cache_key))
    });
}

pub(crate) fn working_conversation_sort_anchor(
    conversation: &PromptCacheConversationResponse,
) -> &str {
    resolve_working_conversation_sort_anchor(
        conversation.last_terminal_at.as_deref(),
        conversation.last_in_flight_at.as_deref(),
        &conversation.created_at,
    )
}

pub(crate) fn working_timestamp_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    match (parse_to_utc_datetime(left), parse_to_utc_datetime(right)) {
        (Some(left), Some(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

pub(crate) fn update_string_if_newer(current: &mut String, candidate: &str) -> bool {
    if working_timestamp_cmp(candidate, current).is_gt() {
        *current = candidate.to_string();
        true
    } else {
        false
    }
}

pub(crate) fn update_string_if_earlier(current: &mut String, candidate: &str) -> bool {
    if working_timestamp_cmp(candidate, current).is_lt() {
        *current = candidate.to_string();
        true
    } else {
        false
    }
}

pub(crate) fn update_option_if_newer(current: &mut Option<String>, candidate: &str) -> bool {
    if current
        .as_deref()
        .is_none_or(|existing| working_timestamp_cmp(candidate, existing).is_gt())
    {
        *current = Some(candidate.to_string());
        true
    } else {
        false
    }
}

pub(crate) fn update_option_if_earlier(current: &mut Option<String>, candidate: &str) -> bool {
    if current
        .as_deref()
        .is_none_or(|existing| working_timestamp_cmp(candidate, existing).is_lt())
    {
        *current = Some(candidate.to_string());
        true
    } else {
        false
    }
}

pub(crate) fn increment_optional_i64(
    current: &mut Option<i64>,
    delta: Option<i64>,
    allow_initialize: bool,
) {
    let Some(delta) = delta else {
        return;
    };
    if current.is_none() && !allow_initialize {
        return;
    }
    *current = Some(current.unwrap_or_default().saturating_add(delta));
}

pub(crate) fn increment_optional_f64(
    current: &mut Option<f64>,
    delta: Option<f64>,
    allow_initialize: bool,
) {
    let Some(delta) = delta else {
        return;
    };
    if current.is_none() && !allow_initialize {
        return;
    }
    *current = Some(current.unwrap_or_default() + delta);
}

#[derive(Debug, Clone)]
pub(crate) enum DashboardTopicMaterializer {
    Activity {
        base: Arc<StdMutex<DashboardActivityMaterializerState>>,
        reporting_tz: Tz,
        source_scope: InvocationSourceScope,
    },
    Summary {
        base: Arc<StdMutex<DashboardSummaryMaterializerState>>,
        window: SummaryWindow,
        reporting_tz: Tz,
        source_scope: InvocationSourceScope,
        upstream_account_id: Option<i64>,
    },
    NetworkTimeseries {
        base: Arc<DashboardNetworkTimeseriesResponse>,
        upstream_account_id: Option<i64>,
    },
    NetworkRecent {
        base: Arc<DashboardRecentNetworkWindowResponse>,
    },
    Timeseries {
        base: Arc<StdMutex<TimeseriesTopicMaterializedBase>>,
        runtime: Arc<RuntimeProjectionHub>,
    },
    ParallelWork {
        base: Arc<StdMutex<DashboardParallelWorkMaterializerState>>,
    },
    WorkingConversations {
        state: Arc<StdMutex<DashboardWorkingConversationsMaterializerState>>,
    },
}

impl DashboardTopicMaterializer {
    pub(crate) fn revision(
        &self,
        base_revision: u64,
        current: Option<&DashboardCurrentProjectionSlice>,
        network: Option<&DashboardNetworkProjectionSlice>,
        terminal: Option<&DashboardTerminalProjectionSlice>,
    ) -> Option<DashboardTopicRevision> {
        match self {
            Self::Activity { base, .. } => {
                let routing_revision = base
                    .lock()
                    .expect("activity materializer state lock")
                    .routing_revision;
                if current.is_none()
                    && network.is_none()
                    && terminal.is_none()
                    && routing_revision == 0
                {
                    return None;
                }
                Some(DashboardTopicRevision {
                    base_revision,
                    current_revision: current.map(|slice| slice.revision),
                    network_revision: network.map(|slice| slice.revision),
                    terminal_revision: terminal.map(|slice| slice.revision),
                    routing_revision,
                })
            }
            Self::Summary { .. } if current.is_some() || terminal.is_some() => {
                Some(DashboardTopicRevision {
                    base_revision,
                    current_revision: current.map(|slice| slice.revision),
                    network_revision: None,
                    terminal_revision: terminal.map(|slice| slice.revision),
                    routing_revision: 0,
                })
            }
            Self::NetworkTimeseries {
                base,
                upstream_account_id,
            } => network
                .filter(|slice| {
                    dashboard_network_timeseries_live_point(
                        base.as_ref(),
                        *upstream_account_id,
                        Some(slice),
                    )
                    .is_some()
                })
                .map(|slice| DashboardTopicRevision {
                    base_revision,
                    current_revision: None,
                    network_revision: Some(slice.revision),
                    terminal_revision: None,
                    routing_revision: 0,
                }),
            Self::NetworkRecent { .. } => network.map(|slice| DashboardTopicRevision {
                base_revision,
                current_revision: None,
                network_revision: Some(slice.revision),
                terminal_revision: None,
                routing_revision: 0,
            }),
            Self::Timeseries { .. } if current.is_some() || terminal.is_some() => {
                Some(DashboardTopicRevision {
                    base_revision,
                    current_revision: current.map(|slice| slice.revision),
                    network_revision: None,
                    terminal_revision: terminal.map(|slice| slice.revision),
                    routing_revision: 0,
                })
            }
            Self::ParallelWork { base } => Some(DashboardTopicRevision {
                base_revision,
                current_revision: Some(
                    base.lock()
                        .expect("parallel-work materializer state lock")
                        .revision,
                ),
                network_revision: None,
                terminal_revision: None,
                routing_revision: 0,
            }),
            Self::WorkingConversations { .. } => None,
            _ => None,
        }
    }

    pub(crate) fn requires_terminal_window_rebase(&self) -> bool {
        match self {
            Self::Activity {
                base, reporting_tz, ..
            } => {
                let base = base.lock().expect("activity materializer state lock");
                let Ok(range) = resolve_dashboard_activity_cached_range(
                    &base.base.response().range,
                    *reporting_tz,
                ) else {
                    return true;
                };
                if parse_duration_spec(&base.base.response().range).is_ok() {
                    return rolling_dashboard_window_requires_rebase(
                        base.rebase_range_start,
                        Some(range.start),
                    );
                }
                base.rebase_range_start != Some(range.start)
            }
            Self::Summary {
                base,
                window,
                reporting_tz,
                ..
            } => {
                let base = base.lock().expect("summary materializer state lock");
                let current_range_start = summary_window_range(window, *reporting_tz, Utc::now())
                    .ok()
                    .flatten()
                    .map(|(start, _)| start);
                match (window, base.range_start, current_range_start) {
                    (SummaryWindow::Duration(_), base_start, current_start) => {
                        rolling_dashboard_window_requires_rebase(base_start, current_start)
                    }
                    (_, base_start, current_start) => base_start != current_start,
                }
            }
            Self::Timeseries { base, .. } => base
                .lock()
                .expect("timeseries materializer state lock")
                .requires_window_rebase(),
            Self::ParallelWork { base } => base
                .lock()
                .expect("parallel-work materializer state lock")
                .requires_rolling_rebase(),
            Self::NetworkTimeseries { .. }
            | Self::NetworkRecent { .. }
            | Self::WorkingConversations { .. } => false,
        }
    }

    pub(crate) fn serialize(
        &self,
        current: Option<&DashboardCurrentProjectionSlice>,
        network: Option<&DashboardNetworkProjectionSlice>,
        terminal: Option<&DashboardTerminalProjectionSlice>,
    ) -> Result<Vec<u8>, ApiError> {
        match self {
            Self::Activity {
                base,
                reporting_tz,
                source_scope,
            } => {
                let mut base = base.lock().expect("activity materializer state lock");
                if current.is_some_and(|slice| base.current_revision < Some(slice.revision)) {
                    apply_dashboard_activity_slices(base.base.response_mut(), current, None);
                    base.current_revision = current.map(|slice| slice.revision);
                }
                if network.is_some_and(|slice| base.network_revision < Some(slice.revision)) {
                    apply_dashboard_activity_slices(base.base.response_mut(), None, network);
                    base.network_revision = network.map(|slice| slice.revision);
                }
                if terminal.is_some_and(|slice| base.terminal_revision < Some(slice.revision)) {
                    base.base.apply_terminal_slice(
                        *reporting_tz,
                        *source_scope,
                        terminal.expect("terminal slice checked above"),
                    );
                    base.terminal_revision = terminal.map(|slice| slice.revision);
                }
                serde_json::to_vec(base.base.response()).map_err(ApiError::from)
            }
            Self::Summary {
                base,
                window: _,
                reporting_tz: _,
                source_scope: _,
                upstream_account_id,
            } => {
                let mut base = base.lock().expect("summary materializer state lock");
                if current.is_some_and(|slice| base.current_revision < Some(slice.revision)) {
                    apply_dashboard_current_slice_to_summary_response(
                        &mut base.response,
                        *upstream_account_id,
                        current,
                    );
                    base.current_revision = current.map(|slice| slice.revision);
                }
                if terminal.is_some_and(|slice| base.terminal_revision < Some(slice.revision)) {
                    // Terminal slices can be emitted before the SQLite transaction commits.
                    // Summary only incorporates the writer-ACKed journal during its dedicated
                    // refresh, so this delivery path never publishes a speculative total.
                    base.terminal_revision = terminal.map(|slice| slice.revision);
                }
                serde_json::to_vec(&base.response).map_err(ApiError::from)
            }
            Self::NetworkTimeseries {
                base,
                upstream_account_id,
            } => serde_json::to_vec(&DashboardNetworkTimeseriesPayload {
                base,
                upstream_account_id: *upstream_account_id,
                network,
            })
            .map_err(ApiError::from),
            Self::NetworkRecent { base } => {
                serde_json::to_vec(&DashboardNetworkRecentPayload { base, network })
                    .map_err(ApiError::from)
            }
            Self::Timeseries { base, runtime } => {
                let mut base = base.lock().expect("timeseries materializer state lock");
                base.apply_terminal_slice(terminal);
                base.serialize(&runtime.snapshot())
            }
            Self::ParallelWork { base } => serde_json::to_vec(
                &base
                    .lock()
                    .expect("parallel-work materializer state lock")
                    .response,
            )
            .map_err(ApiError::from),
            Self::WorkingConversations { state } => state
                .lock()
                .expect("working conversations materializer state lock")
                .serialize(),
        }
    }
}

pub(crate) struct DashboardNetworkTimeseriesPayload<'a> {
    pub(crate) base: &'a DashboardNetworkTimeseriesResponse,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) network: Option<&'a DashboardNetworkProjectionSlice>,
}

impl Serialize for DashboardNetworkTimeseriesPayload<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let live_point = dashboard_network_timeseries_live_point(
            self.base,
            self.upstream_account_id,
            self.network,
        );
        let mut response = serializer.serialize_struct("DashboardNetworkTimeseriesResponse", 6)?;
        response.serialize_field("range", &self.base.range)?;
        response.serialize_field("rangeStart", &self.base.range_start)?;
        if let Some((_, _)) = live_point {
            let now = Utc::now();
            response.serialize_field("rangeEnd", &format_utc_iso_precise(now))?;
            response.serialize_field("snapshotId", &now.timestamp_millis())?;
        } else {
            response.serialize_field("rangeEnd", &self.base.range_end)?;
            response.serialize_field("snapshotId", &self.base.snapshot_id)?;
        }
        response.serialize_field("bucketSeconds", &self.base.bucket_seconds)?;
        response.serialize_field(
            "points",
            &DashboardNetworkTimeseriesPoints {
                points: &self.base.points,
                live_point,
            },
        )?;
        response.end()
    }
}

pub(crate) struct DashboardNetworkTimeseriesPoints<'a> {
    pub(crate) points: &'a [DashboardNetworkTimeseriesPointResponse],
    pub(crate) live_point: Option<(usize, &'a DashboardNetworkTimeseriesPointResponse)>,
}

impl Serialize for DashboardNetworkTimeseriesPoints<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut points = serializer.serialize_seq(Some(self.points.len()))?;
        for (index, point) in self.points.iter().enumerate() {
            let point = self
                .live_point
                .filter(|(live_index, _)| *live_index == index)
                .map_or(point, |(_, live_point)| live_point);
            points.serialize_element(point)?;
        }
        points.end()
    }
}

pub(crate) struct DashboardNetworkRecentPayload<'a> {
    pub(crate) base: &'a DashboardRecentNetworkWindowResponse,
    pub(crate) network: Option<&'a DashboardNetworkProjectionSlice>,
}

impl Serialize for DashboardNetworkRecentPayload<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.network
            .map_or(self.base, |slice| &slice.recent)
            .serialize(serializer)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum BuiltSubscriptionTopicPayload {
    Json(Value),
    JsonDelta {
        event_payload: Value,
        snapshot_payload: Value,
    },
    Dashboard(DashboardTopicMaterializer),
}

impl BuiltSubscriptionTopicPayload {
    pub(crate) fn dashboard_materializer(&self) -> Option<DashboardTopicMaterializer> {
        match self {
            Self::Json(_) | Self::JsonDelta { .. } => None,
            Self::Dashboard(materializer) => Some(materializer.clone()),
        }
    }

    pub(crate) fn serialize(
        &self,
        current: Option<&DashboardCurrentProjectionSlice>,
        network: Option<&DashboardNetworkProjectionSlice>,
        terminal: Option<&DashboardTerminalProjectionSlice>,
    ) -> Result<Vec<u8>, ApiError> {
        match self {
            Self::Json(payload) => serde_json::to_vec(payload).map_err(ApiError::from),
            Self::JsonDelta { event_payload, .. } => {
                serde_json::to_vec(event_payload).map_err(ApiError::from)
            }
            Self::Dashboard(materializer) => materializer.serialize(current, network, terminal),
        }
    }

    pub(crate) fn serialize_snapshot(
        &self,
        current: Option<&DashboardCurrentProjectionSlice>,
        network: Option<&DashboardNetworkProjectionSlice>,
        terminal: Option<&DashboardTerminalProjectionSlice>,
    ) -> Result<Vec<u8>, ApiError> {
        match self {
            Self::JsonDelta {
                snapshot_payload, ..
            } => serde_json::to_vec(snapshot_payload).map_err(ApiError::from),
            _ => self.serialize(current, network, terminal),
        }
    }

    pub(crate) fn snapshot_payload(&self) -> Value {
        match self {
            Self::Json(payload) => payload.clone(),
            Self::JsonDelta {
                snapshot_payload, ..
            } => snapshot_payload.clone(),
            Self::Dashboard(_) => Value::Null,
        }
    }
}
