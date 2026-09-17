struct DashboardActivitySnapshotCacheRequest<'a> {
    state: &'a AppState,
    range_name: &'a str,
    range: ExactUtcRange,
    selection: DashboardActivitySnapshotSelection,
    recent_limit: usize,
    include_accounts: bool,
    include_recent: bool,
    in_progress_counts_override: Option<HashMap<Option<i64>, UpstreamAccountInProgressSummary>>,
    baseline_build_gate: Arc<Mutex<()>>,
    cache_ttl: Duration,
    cache_ttl_ms: u64,
    selection_fingerprint: u64,
}

#[derive(Default)]
struct DashboardActivitySnapshotCacheLoopState {
    waited_on_in_flight: bool,
    max_waiter_count: usize,
    saw_expired_entry: bool,
}

enum DashboardActivitySnapshotCacheProbe {
    Return(
        Box<(
            DashboardActivitySnapshot,
            DashboardActivitySnapshotCacheOutcome,
        )>,
    ),
    Wait(tokio::sync::watch::Receiver<bool>),
    Build {
        flight_guard: DashboardActivitySnapshotFlightGuard,
        routing_rules_generation: u64,
    },
}

enum DashboardActivitySnapshotCacheBuildAction {
    Return(
        Box<(
            DashboardActivitySnapshot,
            DashboardActivitySnapshotCacheOutcome,
        )>,
    ),
    Retry,
}

#[derive(Clone, Copy)]
struct DashboardActivitySnapshotCacheMetrics {
    cache_entry_count: usize,
    in_flight_count: usize,
    terminal_delta_count: u64,
    duplicate_delta_count: u64,
    pending_delta_count: usize,
    pending_delta_estimated_bytes: usize,
    persisted_ack_pending_count: usize,
    delta_pruned_count: u64,
    hard_limit_reason: &'static str,
    sequence_gap_count: u64,
}

fn dashboard_activity_snapshot_cache_metrics(
    cache: &DashboardActivitySnapshotCacheState,
) -> DashboardActivitySnapshotCacheMetrics {
    DashboardActivitySnapshotCacheMetrics {
        cache_entry_count: cache.entries.len(),
        in_flight_count: cache.in_flight.len(),
        terminal_delta_count: cache.read_model.terminal_delta_count,
        duplicate_delta_count: cache.read_model.duplicate_delta_count,
        pending_delta_count: cache.read_model.pending_terminal_deltas.len(),
        pending_delta_estimated_bytes: cache.read_model.pending_delta_estimated_bytes,
        persisted_ack_pending_count: cache.read_model.persisted_ack_pending_count,
        delta_pruned_count: cache.read_model.delta_pruned_count,
        hard_limit_reason: cache.read_model.hard_limit_reason.unwrap_or("none"),
        sequence_gap_count: cache.read_model.sequence_gap_count,
    }
}

async fn load_dashboard_activity_snapshot_cached(
    state: &AppState,
    range_name: &str,
    reporting_tz: Tz,
    recent_limit: usize,
    include_accounts: bool,
    include_recent: bool,
    in_progress_counts_override: Option<HashMap<Option<i64>, UpstreamAccountInProgressSummary>>,
) -> Result<
    (
        DashboardActivitySnapshot,
        DashboardActivitySnapshotCacheOutcome,
    ),
    ApiError,
> {
    if range_name == "yesterday" {
        return load_dashboard_activity_yesterday(
            state,
            range_name,
            reporting_tz,
            recent_limit,
            include_accounts,
            include_recent,
            in_progress_counts_override,
        )
        .await;
    }
    let request = build_dashboard_activity_snapshot_cache_request(
        state,
        range_name,
        reporting_tz,
        recent_limit,
        include_accounts,
        include_recent,
        in_progress_counts_override,
    )
    .await?;
    if !dashboard_activity_snapshot_cache_can_track_expiry(
        range_name,
        request.range,
        shanghai_retention_cutoff(state.config.invocation_max_days),
    ) {
        return load_dashboard_activity_snapshot_uncached(request).await;
    }
    let mut loop_state = DashboardActivitySnapshotCacheLoopState::default();
    loop {
        match probe_dashboard_activity_snapshot_cache(&request, &mut loop_state).await? {
            DashboardActivitySnapshotCacheProbe::Return(result) => return Ok(*result),
            DashboardActivitySnapshotCacheProbe::Wait(mut receiver) => {
                loop_state.waited_on_in_flight = true;
                if !*receiver.borrow() {
                    let _ = receiver.changed().await;
                }
            }
            DashboardActivitySnapshotCacheProbe::Build {
                flight_guard,
                routing_rules_generation,
            } => {
                match run_dashboard_activity_snapshot_cache_build(
                    &request,
                    loop_state.saw_expired_entry,
                    routing_rules_generation,
                    flight_guard,
                )
                .await?
                {
                    DashboardActivitySnapshotCacheBuildAction::Return(result) => {
                        return Ok(*result);
                    }
                    DashboardActivitySnapshotCacheBuildAction::Retry => continue,
                }
            }
        }
    }
}

async fn build_dashboard_activity_snapshot_cache_request<'a>(
    state: &'a AppState,
    range_name: &'a str,
    reporting_tz: Tz,
    recent_limit: usize,
    include_accounts: bool,
    include_recent: bool,
    in_progress_counts_override: Option<HashMap<Option<i64>, UpstreamAccountInProgressSummary>>,
) -> Result<DashboardActivitySnapshotCacheRequest<'a>, ApiError> {
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let range = resolve_dashboard_activity_cached_range(range_name, reporting_tz)?;
    let selection = build_dashboard_activity_snapshot_selection(
        range_name,
        range,
        reporting_tz,
        source_scope,
        recent_limit,
        include_accounts,
        include_recent,
    );
    let baseline_build_gate = {
        let cache = state.dashboard_activity_snapshot_cache.lock().await;
        cache.baseline_build_gate.clone()
    };
    let cache_ttl = dashboard_activity_snapshot_cache_ttl(range_name);
    Ok(DashboardActivitySnapshotCacheRequest {
        state,
        range_name,
        range,
        selection_fingerprint: dashboard_activity_selection_fingerprint(&selection),
        selection,
        recent_limit,
        include_accounts,
        include_recent,
        in_progress_counts_override,
        baseline_build_gate,
        cache_ttl,
        cache_ttl_ms: cache_ttl.as_millis() as u64,
    })
}

async fn load_dashboard_activity_yesterday(
    state: &AppState,
    range_name: &str,
    reporting_tz: Tz,
    recent_limit: usize,
    include_accounts: bool,
    include_recent: bool,
    in_progress_counts_override: Option<HashMap<Option<i64>, UpstreamAccountInProgressSummary>>,
) -> Result<
    (
        DashboardActivitySnapshot,
        DashboardActivitySnapshotCacheOutcome,
    ),
    ApiError,
> {
    let started_at = Instant::now();
    let range = resolve_dashboard_activity_exact_range(range_name, reporting_tz)?;
    let snapshot =
        load_dashboard_activity_snapshot_for_range_input(DashboardActivitySnapshotForRangeInput {
            state,
            range_name,
            range,
            recent_limit,
            include_accounts,
            include_recent,
            in_progress_counts_override,
        })
        .await?;
    Ok((
        snapshot,
        DashboardActivitySnapshotCacheOutcome {
            cache_hit_or_miss: "uncached",
            cache_bypass_reason: "yesterday_exact",
            coalesced_waiter_count: 0,
            db_build_elapsed_ms: started_at.elapsed().as_millis() as u64,
            cache_ttl_ms: 0,
            cache_entry_age_ms: 0,
            cache_entry_count: 0,
            in_flight_count: 0,
            refresh_reason: "exact_uncached",
            selection_fingerprint: 0,
            terminal_delta_count: 0,
            duplicate_delta_count: 0,
            pending_delta_count: 0,
            pending_delta_estimated_bytes: 0,
            persisted_ack_pending_count: 0,
            delta_pruned_count: 0,
            expiry_delta_count: 0,
            hard_limit_reason: "none",
            baseline_cursor: 0,
            sequence_gap_count: 0,
            build_attempted: true,
            snapshot_origin: "exact_db",
        },
    ))
}

fn expire_dashboard_activity_snapshot_entry(
    entry: &mut DashboardActivitySnapshotCacheEntry,
    range: ExactUtcRange,
) -> usize {
    let mut expired_count = 0usize;
    while entry.expiry_terminal_deltas.front().is_some_and(|delta| {
        parse_to_utc_datetime(&delta.occurred_at)
            .is_some_and(|occurred_at| occurred_at < range.start)
    }) {
        if let Some(delta) = entry.expiry_terminal_deltas.pop_front() {
            entry.expiry_delta_estimated_bytes = entry
                .expiry_delta_estimated_bytes
                .saturating_sub(delta.estimated_bytes);
            subtract_dashboard_activity_compact_terminal_delta(&mut entry.response, &delta);
            restore_dashboard_activity_last_invocation_after_expiry(
                &mut entry.response,
                &delta,
                &entry.expiry_terminal_deltas,
            );
            expired_count += 1;
        }
    }
    expired_count
}

fn reuse_dashboard_activity_snapshot_entry(
    request: &DashboardActivitySnapshotCacheRequest<'_>,
    loop_state: &DashboardActivitySnapshotCacheLoopState,
    metrics: DashboardActivitySnapshotCacheMetrics,
    entry: &mut DashboardActivitySnapshotCacheEntry,
) -> Option<(
    DashboardActivitySnapshot,
    DashboardActivitySnapshotCacheOutcome,
)> {
    let reuse_mode =
        dashboard_activity_snapshot_reuse_mode(entry, request.range, request.cache_ttl)?;
    let expired_count = expire_dashboard_activity_snapshot_entry(entry, request.range);
    let is_last_good = reuse_mode == DashboardActivitySnapshotReuseMode::LastGoodReconcileBackoff;
    Some((
        entry.response.clone(),
        DashboardActivitySnapshotCacheOutcome {
            cache_hit_or_miss: if is_last_good {
                "last_good_fallback"
            } else if loop_state.waited_on_in_flight {
                "wait_on_in_flight"
            } else {
                "cache_hit"
            },
            cache_bypass_reason: if is_last_good {
                "expiry_reconcile_backoff"
            } else {
                "none"
            },
            coalesced_waiter_count: loop_state.max_waiter_count,
            db_build_elapsed_ms: 0,
            cache_ttl_ms: request.cache_ttl_ms,
            cache_entry_age_ms: entry.cached_at.elapsed().as_millis() as u64,
            cache_entry_count: metrics.cache_entry_count,
            in_flight_count: metrics.in_flight_count,
            refresh_reason: if is_last_good {
                "reconcile_backoff"
            } else {
                "within_ttl"
            },
            selection_fingerprint: request.selection_fingerprint,
            terminal_delta_count: metrics.terminal_delta_count,
            duplicate_delta_count: metrics.duplicate_delta_count,
            pending_delta_count: metrics.pending_delta_count,
            pending_delta_estimated_bytes: metrics.pending_delta_estimated_bytes,
            persisted_ack_pending_count: metrics.persisted_ack_pending_count,
            delta_pruned_count: metrics.delta_pruned_count,
            expiry_delta_count: expired_count,
            hard_limit_reason: metrics.hard_limit_reason,
            baseline_cursor: entry.baseline_snapshot_cursor,
            sequence_gap_count: metrics.sequence_gap_count,
            build_attempted: false,
            snapshot_origin: if is_last_good { "last_good" } else { "memory" },
        },
    ))
}

fn reuse_dashboard_activity_snapshot_under_pressure(
    request: &DashboardActivitySnapshotCacheRequest<'_>,
    loop_state: &DashboardActivitySnapshotCacheLoopState,
    metrics: DashboardActivitySnapshotCacheMetrics,
    entry: &mut DashboardActivitySnapshotCacheEntry,
) -> Option<(
    DashboardActivitySnapshot,
    DashboardActivitySnapshotCacheOutcome,
)> {
    if !dashboard_activity_entry_expiry_covers(entry, request.range)
        || !dashboard_activity_pressure_reconcile_deferred(entry)
    {
        return None;
    }
    let expired_count = expire_dashboard_activity_snapshot_entry(entry, request.range);
    entry.last_reconcile_attempted_at = Instant::now();
    entry.last_reconcile_failed = true;
    Some((
        entry.response.clone(),
        DashboardActivitySnapshotCacheOutcome {
            cache_hit_or_miss: "last_good_fallback",
            cache_bypass_reason: "writer_pressure",
            coalesced_waiter_count: loop_state.max_waiter_count,
            db_build_elapsed_ms: 0,
            cache_ttl_ms: request.cache_ttl_ms,
            cache_entry_age_ms: entry.cached_at.elapsed().as_millis() as u64,
            cache_entry_count: metrics.cache_entry_count,
            in_flight_count: metrics.in_flight_count,
            refresh_reason: "reconcile_deferred",
            selection_fingerprint: request.selection_fingerprint,
            terminal_delta_count: metrics.terminal_delta_count,
            duplicate_delta_count: metrics.duplicate_delta_count,
            pending_delta_count: metrics.pending_delta_count,
            pending_delta_estimated_bytes: metrics.pending_delta_estimated_bytes,
            persisted_ack_pending_count: metrics.persisted_ack_pending_count,
            delta_pruned_count: metrics.delta_pruned_count,
            expiry_delta_count: expired_count,
            hard_limit_reason: metrics.hard_limit_reason,
            baseline_cursor: entry.baseline_snapshot_cursor,
            sequence_gap_count: metrics.sequence_gap_count,
            build_attempted: false,
            snapshot_origin: "memory",
        },
    ))
}

async fn probe_dashboard_activity_snapshot_cache(
    request: &DashboardActivitySnapshotCacheRequest<'_>,
    loop_state: &mut DashboardActivitySnapshotCacheLoopState,
) -> Result<DashboardActivitySnapshotCacheProbe, ApiError> {
    let mut cache = request.state.dashboard_activity_snapshot_cache.lock().await;
    let routing_rules_generation = cache.routing_rules_generation;
    loop_state.saw_expired_entry |= cache
        .entries
        .get(&request.selection)
        .is_some_and(|entry| entry.cached_at.elapsed() > request.cache_ttl);
    cache
        .entries
        .retain(|_, entry| entry.cached_at.elapsed() <= DASHBOARD_ACTIVITY_LAST_GOOD_MAX_AGE);
    let metrics = dashboard_activity_snapshot_cache_metrics(&cache);
    let settled_dirty_state = dashboard_activity_read_model_requires_reconcile(&cache.read_model);
    if !settled_dirty_state
        && let Some(entry) = cache.entries.get_mut(&request.selection)
        && let Some(result) =
            reuse_dashboard_activity_snapshot_entry(request, loop_state, metrics, entry)
    {
        return Ok(DashboardActivitySnapshotCacheProbe::Return(Box::new(
            result,
        )));
    }
    let writer_pressure = crate::db_pressure::global_db_pressure_gate()
        .snapshot()
        .pressure_cooldown_remaining_ms
        > 0;
    if !settled_dirty_state
        && writer_pressure
        && let Some(entry) = cache.entries.get_mut(&request.selection)
        && let Some(result) =
            reuse_dashboard_activity_snapshot_under_pressure(request, loop_state, metrics, entry)
    {
        return Ok(DashboardActivitySnapshotCacheProbe::Return(Box::new(
            result,
        )));
    }
    if let Some(in_flight) = cache.in_flight.get_mut(&request.selection) {
        in_flight.waiter_count += 1;
        loop_state.max_waiter_count = loop_state.max_waiter_count.max(in_flight.waiter_count);
        return Ok(DashboardActivitySnapshotCacheProbe::Wait(
            in_flight.signal.subscribe(),
        ));
    }
    let (signal, _receiver) = tokio::sync::watch::channel(false);
    cache.in_flight.insert(
        request.selection.clone(),
        DashboardActivitySnapshotInFlight {
            signal,
            waiter_count: 0,
            baseline_cursor: None,
            routing_rules_generation,
        },
    );
    Ok(DashboardActivitySnapshotCacheProbe::Build {
        flight_guard: DashboardActivitySnapshotFlightGuard::new(
            request.state.dashboard_activity_snapshot_cache.clone(),
            request.selection.clone(),
            routing_rules_generation,
        ),
        routing_rules_generation,
    })
}

struct DashboardActivityUncachedBuild {
    result: Result<DashboardActivitySnapshot, ApiError>,
    baseline_cursor: i64,
    persisted_terminal_ids: HashMap<(String, String), i64>,
}

async fn dashboard_activity_pending_terminal_keys(
    state: &AppState,
    selection: &DashboardActivitySnapshotSelection,
) -> Vec<(String, String)> {
    let cache = state.dashboard_activity_snapshot_cache.lock().await;
    cache
        .read_model
        .pending_terminal_deltas
        .iter()
        .filter_map(|delta| {
            let occurred_at = parse_to_utc_datetime(&delta.occurred_at)?;
            dashboard_activity_selection_includes_compact_terminal(selection, delta, occurred_at)
                .then_some(delta.key())
        })
        .collect()
}

async fn load_dashboard_activity_uncached_build(
    request: &DashboardActivitySnapshotCacheRequest<'_>,
    consistency_connection: &mut Option<sqlx::Transaction<'static, Sqlite>>,
) -> DashboardActivityUncachedBuild {
    let baseline_cursor = match consistency_connection.as_mut() {
        Some(connection) => {
            match query_live_dashboard_activity_snapshot_cursor_tx(connection.as_mut()).await {
                Ok(cursor) => cursor,
                Err(error) => {
                    return DashboardActivityUncachedBuild {
                        result: Err(error),
                        baseline_cursor: 0,
                        persisted_terminal_ids: HashMap::new(),
                    };
                }
            }
        }
        None => match query_live_dashboard_activity_snapshot_cursor(&request.state.pool).await {
            Ok(cursor) => cursor,
            Err(error) => {
                return DashboardActivityUncachedBuild {
                    result: Err(error),
                    baseline_cursor: 0,
                    persisted_terminal_ids: HashMap::new(),
                };
            }
        },
    };
    let mut result =
        load_dashboard_activity_snapshot_for_range_input(DashboardActivitySnapshotForRangeInput {
            state: request.state,
            range_name: request.range_name,
            range: request.range,
            recent_limit: request.recent_limit,
            include_accounts: request.include_accounts,
            include_recent: request.include_recent,
            in_progress_counts_override: request.in_progress_counts_override.clone(),
        })
        .await;
    let persisted_terminal_ids = if result.is_ok() {
        let keys =
            dashboard_activity_pending_terminal_keys(request.state, &request.selection).await;
        match consistency_connection.as_mut() {
            Some(connection) => query_live_dashboard_activity_persisted_terminal_ids_tx(
                connection.as_mut(),
                dashboard_activity_selection_source_scope(&request.selection),
                &keys,
            )
            .await
            .unwrap_or_else(|error| {
                result = Err(error);
                HashMap::new()
            }),
            None => HashMap::new(),
        }
    } else {
        HashMap::new()
    };
    DashboardActivityUncachedBuild {
        result,
        baseline_cursor,
        persisted_terminal_ids,
    }
}

async fn load_dashboard_activity_snapshot_uncached(
    request: DashboardActivitySnapshotCacheRequest<'_>,
) -> Result<
    (
        DashboardActivitySnapshot,
        DashboardActivitySnapshotCacheOutcome,
    ),
    ApiError,
> {
    let started_at = Instant::now();
    let _baseline_build_guard = request.baseline_build_gate.lock().await;
    let reconcile_gate = request.state.sqlite_batch_writer.dashboard_reconcile_gate();
    let reconcile_guard = reconcile_gate.lock().await;
    let mut consistency_connection =
        Some(begin_dashboard_activity_consistency_barrier(&request.state.pool).await?);
    drop(reconcile_guard);
    let build = load_dashboard_activity_uncached_build(&request, &mut consistency_connection).await;
    let cache = request.state.dashboard_activity_snapshot_cache.lock().await;
    if let Some(connection) = consistency_connection.take() {
        finish_dashboard_activity_consistency_barrier(connection, build.result.is_ok()).await?;
    }
    let mut snapshot = build.result?;
    let baseline_cursor = build.baseline_cursor;
    if !dashboard_activity_hard_limit_is_settled(&cache.read_model) {
        return Err(ApiError::from(anyhow!(
            "dashboard activity hard-limit terminal writes are not persisted yet"
        )));
    }
    let replayed_pending_delta_count = replay_dashboard_activity_pending_deltas_without_expiry(
        &mut snapshot,
        &request.selection,
        &cache.read_model.pending_terminal_deltas,
        baseline_cursor,
        &build.persisted_terminal_ids,
    );
    tracing::debug!(
        selection_fingerprint = request.selection_fingerprint,
        replayed_pending_delta_count,
        "replayed pending dashboard deltas over archive-prefix exact snapshot"
    );
    let metrics = dashboard_activity_snapshot_cache_metrics(&cache);
    Ok((
        snapshot,
        DashboardActivitySnapshotCacheOutcome {
            cache_hit_or_miss: "uncached",
            cache_bypass_reason: "archive_expiry_unavailable",
            coalesced_waiter_count: 0,
            db_build_elapsed_ms: started_at.elapsed().as_millis() as u64,
            cache_ttl_ms: 0,
            cache_entry_age_ms: 0,
            cache_entry_count: 0,
            in_flight_count: 0,
            refresh_reason: "exact_uncached",
            selection_fingerprint: 0,
            terminal_delta_count: metrics.terminal_delta_count,
            duplicate_delta_count: metrics.duplicate_delta_count,
            pending_delta_count: metrics.pending_delta_count,
            pending_delta_estimated_bytes: metrics.pending_delta_estimated_bytes,
            persisted_ack_pending_count: metrics.persisted_ack_pending_count,
            delta_pruned_count: metrics.delta_pruned_count,
            expiry_delta_count: 0,
            hard_limit_reason: metrics.hard_limit_reason,
            baseline_cursor,
            sequence_gap_count: metrics.sequence_gap_count,
            build_attempted: true,
            snapshot_origin: "exact_db_with_pending_overlay",
        },
    ))
}

struct DashboardActivitySnapshotBuildData {
    result: Result<DashboardActivitySnapshot, ApiError>,
    snapshot_cursor_before_build: i64,
    snapshot_cursor_after_build: i64,
    persisted_terminal_ids_after_build: HashMap<(String, String), i64>,
    expiry_terminal_deltas: VecDeque<DashboardActivityTerminalDelta>,
    expiry_covered_until: Option<DateTime<Utc>>,
    expiry_delta_estimated_bytes: usize,
    expiry_tracking_failure_reason: Option<&'static str>,
}

async fn load_dashboard_activity_snapshot_build_data(
    request: &DashboardActivitySnapshotCacheRequest<'_>,
    consistency_connection: &mut Option<sqlx::Transaction<'static, Sqlite>>,
    snapshot_cursor_before_build_result: Result<i64, ApiError>,
    consistency_barrier_unavailable: bool,
) -> DashboardActivitySnapshotBuildData {
    let (snapshot_cursor_before_build, mut result) = match snapshot_cursor_before_build_result {
        Ok(snapshot_cursor_before_build) => (
            snapshot_cursor_before_build,
            if consistency_barrier_unavailable {
                Err(ApiError::from(anyhow!(
                    "dashboard activity consistency barrier unavailable"
                )))
            } else {
                load_dashboard_activity_snapshot_for_range_input(
                    DashboardActivitySnapshotForRangeInput {
                        state: request.state,
                        range_name: request.range_name,
                        range: request.range,
                        recent_limit: request.recent_limit,
                        include_accounts: request.include_accounts,
                        include_recent: request.include_recent,
                        in_progress_counts_override: request.in_progress_counts_override.clone(),
                    },
                )
                .await
            },
        ),
        Err(error) => (0, Err(error)),
    };
    let (snapshot_cursor_after_build, persisted_terminal_ids_after_build) = if result.is_ok() {
        let keys =
            dashboard_activity_pending_terminal_keys(request.state, &request.selection).await;
        let probe = match consistency_connection.as_mut() {
            Some(connection) => query_live_dashboard_activity_persisted_terminal_ids_tx(
                connection.as_mut(),
                dashboard_activity_selection_source_scope(&request.selection),
                &keys,
            )
            .await
            .map(|ids| (snapshot_cursor_before_build, ids)),
            None => Ok((snapshot_cursor_before_build, HashMap::new())),
        };
        match probe {
            Ok(probe) => probe,
            Err(error) => {
                result = Err(error);
                (snapshot_cursor_before_build, HashMap::new())
            }
        }
    } else {
        (snapshot_cursor_before_build, HashMap::new())
    };
    let (
        mut expiry_terminal_deltas,
        mut expiry_covered_until,
        mut expiry_delta_estimated_bytes,
        mut expiry_tracking_failure_reason,
    ) = if result.is_ok() {
        match load_dashboard_activity_expiry_deltas(
            &request.state.pool,
            &request.selection,
            request.range,
            snapshot_cursor_after_build,
        )
        .await
        {
            Ok(expiry) => expiry,
            Err(error) => {
                result = Err(error);
                (VecDeque::new(), None, 0, None)
            }
        }
    } else {
        (VecDeque::new(), None, 0, None)
    };
    if consistency_barrier_unavailable {
        expiry_tracking_failure_reason = Some("consistency_barrier_unavailable");
        expiry_terminal_deltas.clear();
        expiry_delta_estimated_bytes = 0;
        expiry_covered_until = None;
    }
    DashboardActivitySnapshotBuildData {
        result,
        snapshot_cursor_before_build,
        snapshot_cursor_after_build,
        persisted_terminal_ids_after_build,
        expiry_terminal_deltas,
        expiry_covered_until,
        expiry_delta_estimated_bytes,
        expiry_tracking_failure_reason,
    }
}

async fn handle_dashboard_activity_barrier_unavailable(
    request: &DashboardActivitySnapshotCacheRequest<'_>,
    mut flight_guard: DashboardActivitySnapshotFlightGuard,
    build_started_at: Instant,
) -> Result<DashboardActivitySnapshotCacheBuildAction, ApiError> {
    let mut cache = request.state.dashboard_activity_snapshot_cache.lock().await;
    if let Some(entry) = cache.entries.get_mut(&request.selection) {
        mark_dashboard_activity_reconcile_failed(entry);
        let response = entry.response.clone();
        let cache_entry_age_ms = entry.cached_at.elapsed().as_millis() as u64;
        let baseline_cursor = entry.baseline_snapshot_cursor;
        let in_flight = cache.in_flight.remove(&request.selection);
        flight_guard.disarm();
        let coalesced_waiter_count = in_flight.as_ref().map_or(0, |flight| flight.waiter_count);
        if let Some(in_flight) = in_flight {
            let _ = in_flight.signal.send(true);
        }
        let metrics = dashboard_activity_snapshot_cache_metrics(&cache);
        return Ok(DashboardActivitySnapshotCacheBuildAction::Return(Box::new(
            (
                response,
                DashboardActivitySnapshotCacheOutcome {
                    cache_hit_or_miss: "last_good_fallback",
                    cache_bypass_reason: "consistency_barrier_unavailable",
                    coalesced_waiter_count,
                    db_build_elapsed_ms: build_started_at.elapsed().as_millis() as u64,
                    cache_ttl_ms: request.cache_ttl_ms,
                    cache_entry_age_ms,
                    cache_entry_count: metrics.cache_entry_count,
                    in_flight_count: metrics.in_flight_count,
                    refresh_reason: "reconcile_lock_contended",
                    selection_fingerprint: request.selection_fingerprint,
                    terminal_delta_count: metrics.terminal_delta_count,
                    duplicate_delta_count: metrics.duplicate_delta_count,
                    pending_delta_count: metrics.pending_delta_count,
                    pending_delta_estimated_bytes: metrics.pending_delta_estimated_bytes,
                    persisted_ack_pending_count: metrics.persisted_ack_pending_count,
                    delta_pruned_count: metrics.delta_pruned_count,
                    expiry_delta_count: 0,
                    hard_limit_reason: metrics.hard_limit_reason,
                    baseline_cursor,
                    sequence_gap_count: metrics.sequence_gap_count,
                    build_attempted: false,
                    snapshot_origin: "last_good",
                },
            ),
        )));
    }
    let in_flight = cache.in_flight.remove(&request.selection);
    flight_guard.disarm();
    if let Some(in_flight) = in_flight {
        let _ = in_flight.signal.send(true);
    }
    Err(ApiError::from(anyhow!(
        "dashboard activity consistency barrier unavailable for initial snapshot"
    )))
}

async fn finalize_dashboard_activity_snapshot_cache_build(
    request: &DashboardActivitySnapshotCacheRequest<'_>,
    saw_expired_entry: bool,
    routing_rules_generation: u64,
    build_started_at: Instant,
    flight_guard: &mut DashboardActivitySnapshotFlightGuard,
    mut data: DashboardActivitySnapshotBuildData,
) -> Result<DashboardActivitySnapshotCacheBuildAction, ApiError> {
    let mut cache = request.state.dashboard_activity_snapshot_cache.lock().await;
    let refresh_reason = cache
        .invalidation_reasons
        .remove(&request.selection)
        .unwrap_or(if saw_expired_entry {
            "ttl_expired"
        } else {
            "initial_build"
        });
    let in_flight = cache.in_flight.remove(&request.selection);
    flight_guard.disarm();
    let coalesced_waiter_count = in_flight.as_ref().map_or(0, |flight| flight.waiter_count);
    let routing_rules_changed = cache.routing_rules_generation != routing_rules_generation
        || in_flight
            .as_ref()
            .is_some_and(|flight| flight.routing_rules_generation != routing_rules_generation);
    if routing_rules_changed && data.result.is_ok() {
        data.result = Err(ApiError::from(anyhow!(
            "dashboard activity routing rules changed during build"
        )));
    }
    if data.result.is_ok() && !dashboard_activity_hard_limit_is_settled(&cache.read_model) {
        data.result = Err(ApiError::from(anyhow!(
            "dashboard activity hard-limit terminal writes are not persisted yet"
        )));
    }
    apply_dashboard_activity_snapshot_build_deltas(
        &mut data.result,
        DashboardActivitySnapshotBuildDeltaContext {
            cache: &cache,
            selection: &request.selection,
            snapshot_cursor_after_build: data.snapshot_cursor_after_build,
            persisted_terminal_ids_after_build: &data.persisted_terminal_ids_after_build,
            expiry_terminal_deltas: &mut data.expiry_terminal_deltas,
            expiry_covered_until: &mut data.expiry_covered_until,
            expiry_delta_estimated_bytes: &mut data.expiry_delta_estimated_bytes,
            expiry_tracking_failure_reason: &mut data.expiry_tracking_failure_reason,
        },
    );
    finalize_dashboard_activity_snapshot_cache_build_result(
        request,
        build_started_at,
        routing_rules_changed,
        coalesced_waiter_count,
        refresh_reason,
        data,
        DashboardActivitySnapshotCacheFinalizationState {
            cache: &mut cache,
            in_flight,
        },
    )
}

async fn run_dashboard_activity_snapshot_cache_build(
    request: &DashboardActivitySnapshotCacheRequest<'_>,
    saw_expired_entry: bool,
    routing_rules_generation: u64,
    mut flight_guard: DashboardActivitySnapshotFlightGuard,
) -> Result<DashboardActivitySnapshotCacheBuildAction, ApiError> {
    let build_started_at = Instant::now();
    let _baseline_build_guard = request.baseline_build_gate.lock().await;
    let reconcile_gate = request.state.sqlite_batch_writer.dashboard_reconcile_gate();
    let reconcile_guard = reconcile_gate.lock().await;
    let has_last_good = {
        let cache = request.state.dashboard_activity_snapshot_cache.lock().await;
        cache.entries.contains_key(&request.selection)
    };
    let mut consistency_connection = if has_last_good {
        try_begin_dashboard_activity_consistency_barrier(&request.state.pool).await?
    } else {
        Some(begin_dashboard_activity_consistency_barrier(&request.state.pool).await?)
    };
    drop(reconcile_guard);
    let consistency_barrier_unavailable = consistency_connection.is_none();
    if consistency_barrier_unavailable {
        return handle_dashboard_activity_barrier_unavailable(
            request,
            flight_guard,
            build_started_at,
        )
        .await;
    }
    let snapshot_cursor_before_build = match consistency_connection.as_mut() {
        Some(connection) => {
            query_live_dashboard_activity_snapshot_cursor_tx(connection.as_mut()).await
        }
        None => query_live_dashboard_activity_snapshot_cursor(&request.state.pool).await,
    };
    {
        let mut cache = request.state.dashboard_activity_snapshot_cache.lock().await;
        if let Ok(snapshot_cursor_before_build) = snapshot_cursor_before_build.as_ref()
            && let Some(flight) = cache.in_flight.get_mut(&request.selection)
        {
            flight.baseline_cursor = Some(*snapshot_cursor_before_build);
        }
    }
    let mut data = load_dashboard_activity_snapshot_build_data(
        request,
        &mut consistency_connection,
        snapshot_cursor_before_build,
        consistency_barrier_unavailable,
    )
    .await;
    if let Some(connection) = consistency_connection.take()
        && let Err(error) =
            finish_dashboard_activity_consistency_barrier(connection, data.result.is_ok()).await
    {
        data.result = Err(error);
    }
    finalize_dashboard_activity_snapshot_cache_build(
        request,
        saw_expired_entry,
        routing_rules_generation,
        build_started_at,
        &mut flight_guard,
        data,
    )
    .await
}
