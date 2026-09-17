struct DashboardActivitySnapshotBuildDeltaContext<'a> {
    cache: &'a DashboardActivitySnapshotCacheState,
    selection: &'a DashboardActivitySnapshotSelection,
    snapshot_cursor_after_build: i64,
    persisted_terminal_ids_after_build: &'a HashMap<(String, String), i64>,
    expiry_terminal_deltas: &'a mut VecDeque<DashboardActivityTerminalDelta>,
    expiry_covered_until: &'a mut Option<DateTime<Utc>>,
    expiry_delta_estimated_bytes: &'a mut usize,
    expiry_tracking_failure_reason: &'a mut Option<&'static str>,
}

struct DashboardActivitySnapshotCacheFinalizationState<'a> {
    cache: &'a mut DashboardActivitySnapshotCacheState,
    in_flight: Option<DashboardActivitySnapshotInFlight>,
}

fn apply_dashboard_activity_snapshot_build_deltas(
    result: &mut Result<DashboardActivitySnapshot, ApiError>,
    context: DashboardActivitySnapshotBuildDeltaContext<'_>,
) {
    let Ok(snapshot) = result.as_mut() else {
        return;
    };
    for delta in &context.cache.read_model.pending_terminal_deltas {
        let Some(occurred_at) = parse_to_utc_datetime(&delta.occurred_at) else {
            continue;
        };
        if !dashboard_activity_selection_includes_compact_terminal(
            context.selection,
            delta,
            occurred_at,
        ) {
            continue;
        }
        if dashboard_activity_baseline_includes_pending_delta(
            delta,
            context.snapshot_cursor_after_build,
            context.persisted_terminal_ids_after_build,
        ) {
            snapshot.terminal_sequence = snapshot.terminal_sequence.max(delta.terminal_sequence);
            continue;
        }
        if context.expiry_tracking_failure_reason.is_none()
            && let Err(reason) = insert_dashboard_activity_expiry_delta(
                context.expiry_terminal_deltas,
                context.expiry_delta_estimated_bytes,
                *context.expiry_covered_until,
                delta,
                occurred_at,
            )
        {
            *context.expiry_tracking_failure_reason = Some(reason);
            context.expiry_terminal_deltas.clear();
            *context.expiry_delta_estimated_bytes = 0;
            *context.expiry_covered_until = None;
        }
        apply_dashboard_activity_compact_terminal_delta(snapshot, delta);
    }
}

fn dashboard_activity_snapshot_cache_success_outcome(
    request: &DashboardActivitySnapshotCacheRequest<'_>,
    metrics: DashboardActivitySnapshotCacheMetrics,
    coalesced_waiter_count: usize,
    db_build_elapsed_ms: u64,
    refresh_reason: &'static str,
    snapshot_cursor_after_build: i64,
    expiry_tracking_failure_reason: Option<&'static str>,
) -> DashboardActivitySnapshotCacheOutcome {
    DashboardActivitySnapshotCacheOutcome {
        cache_hit_or_miss: if expiry_tracking_failure_reason.is_some() {
            "uncached"
        } else {
            "cache_miss_build"
        },
        cache_bypass_reason: expiry_tracking_failure_reason.unwrap_or("none"),
        coalesced_waiter_count,
        db_build_elapsed_ms,
        cache_ttl_ms: request.cache_ttl_ms,
        cache_entry_age_ms: 0,
        cache_entry_count: metrics.cache_entry_count,
        in_flight_count: metrics.in_flight_count,
        refresh_reason,
        selection_fingerprint: request.selection_fingerprint,
        terminal_delta_count: metrics.terminal_delta_count,
        duplicate_delta_count: metrics.duplicate_delta_count,
        pending_delta_count: metrics.pending_delta_count,
        pending_delta_estimated_bytes: metrics.pending_delta_estimated_bytes,
        persisted_ack_pending_count: metrics.persisted_ack_pending_count,
        delta_pruned_count: metrics.delta_pruned_count,
        expiry_delta_count: 0,
        hard_limit_reason: metrics.hard_limit_reason,
        baseline_cursor: snapshot_cursor_after_build,
        sequence_gap_count: metrics.sequence_gap_count,
        build_attempted: true,
        snapshot_origin: if expiry_tracking_failure_reason.is_some() {
            "exact_db_expiry_untracked"
        } else {
            "db_reconciled"
        },
    }
}

fn finalize_dashboard_activity_snapshot_cache_in_flight(
    request: &DashboardActivitySnapshotCacheRequest<'_>,
    data: &mut DashboardActivitySnapshotBuildData,
    routing_rules_changed: bool,
    cache: &mut DashboardActivitySnapshotCacheState,
    in_flight: Option<DashboardActivitySnapshotInFlight>,
) {
    let Some(in_flight) = in_flight else {
        return;
    };
    if let Ok(snapshot) = &data.result
        && !routing_rules_changed
        && data.expiry_tracking_failure_reason.is_none()
    {
        let response = snapshot.clone();
        let expiry_terminal_deltas = std::mem::take(&mut data.expiry_terminal_deltas);
        cache.entries.insert(
            request.selection.clone(),
            DashboardActivitySnapshotCacheEntry {
                cached_at: Instant::now(),
                last_reconcile_attempted_at: Instant::now(),
                last_reconcile_failed: false,
                baseline_snapshot_cursor: data.snapshot_cursor_after_build,
                expiry_covered_until: data.expiry_covered_until,
                expiry_terminal_deltas,
                expiry_delta_estimated_bytes: data.expiry_delta_estimated_bytes,
                response,
            },
        );
        prune_dashboard_activity_snapshot_entries(cache, Some(&request.selection));
        prune_dashboard_activity_terminal_deltas(cache);
        clear_dashboard_activity_hard_limit_after_baseline(&mut cache.read_model);
    } else if data.expiry_tracking_failure_reason.is_some() {
        cache.entries.remove(&request.selection);
    }
    let _ = in_flight.signal.send(true);
}

fn finalize_dashboard_activity_snapshot_cache_last_good_fallback(
    request: &DashboardActivitySnapshotCacheRequest<'_>,
    metrics: DashboardActivitySnapshotCacheMetrics,
    coalesced_waiter_count: usize,
    db_build_elapsed_ms: u64,
    refresh_reason: &'static str,
    cache: &mut DashboardActivitySnapshotCacheState,
    error: ApiError,
) -> Result<DashboardActivitySnapshotCacheBuildAction, ApiError> {
    let Some(entry) = cache.entries.get_mut(&request.selection) else {
        return Err(error);
    };
    mark_dashboard_activity_reconcile_failed(entry);
    let cache_entry_age_ms = entry.cached_at.elapsed().as_millis() as u64;
    tracing::warn!(
        selection_fingerprint = request.selection_fingerprint,
        refresh_reason,
        cache_entry_age_ms,
        db_build_elapsed_ms,
        error = ?error,
        "dashboard activity reconciliation failed; retained last-good snapshot"
    );
    Ok(DashboardActivitySnapshotCacheBuildAction::Return(Box::new(
        (
            entry.response.clone(),
            DashboardActivitySnapshotCacheOutcome {
                cache_hit_or_miss: "last_good_fallback",
                cache_bypass_reason: "reconcile_failed",
                coalesced_waiter_count,
                db_build_elapsed_ms,
                cache_ttl_ms: request.cache_ttl_ms,
                cache_entry_age_ms,
                cache_entry_count: metrics.cache_entry_count,
                in_flight_count: metrics.in_flight_count,
                refresh_reason: "reconcile_failed",
                selection_fingerprint: request.selection_fingerprint,
                terminal_delta_count: metrics.terminal_delta_count,
                duplicate_delta_count: metrics.duplicate_delta_count,
                pending_delta_count: metrics.pending_delta_count,
                pending_delta_estimated_bytes: metrics.pending_delta_estimated_bytes,
                persisted_ack_pending_count: metrics.persisted_ack_pending_count,
                delta_pruned_count: metrics.delta_pruned_count,
                expiry_delta_count: 0,
                hard_limit_reason: metrics.hard_limit_reason,
                baseline_cursor: entry.baseline_snapshot_cursor,
                sequence_gap_count: metrics.sequence_gap_count,
                build_attempted: true,
                snapshot_origin: "last_good",
            },
        ),
    )))
}

fn finalize_dashboard_activity_snapshot_cache_build_result(
    request: &DashboardActivitySnapshotCacheRequest<'_>,
    build_started_at: Instant,
    routing_rules_changed: bool,
    coalesced_waiter_count: usize,
    refresh_reason: &'static str,
    data: DashboardActivitySnapshotBuildData,
    state: DashboardActivitySnapshotCacheFinalizationState<'_>,
) -> Result<DashboardActivitySnapshotCacheBuildAction, ApiError> {
    let db_build_elapsed_ms = build_started_at.elapsed().as_millis() as u64;
    let DashboardActivitySnapshotCacheFinalizationState { cache, in_flight } = state;
    let mut data = data;
    finalize_dashboard_activity_snapshot_cache_in_flight(
        request,
        &mut data,
        routing_rules_changed,
        cache,
        in_flight,
    );
    if routing_rules_changed {
        return Ok(DashboardActivitySnapshotCacheBuildAction::Retry);
    }
    let metrics = dashboard_activity_snapshot_cache_metrics(cache);
    match data.result {
        Ok(snapshot) => Ok(DashboardActivitySnapshotCacheBuildAction::Return(Box::new(
            (
                snapshot,
                dashboard_activity_snapshot_cache_success_outcome(
                    request,
                    metrics,
                    coalesced_waiter_count,
                    db_build_elapsed_ms,
                    refresh_reason,
                    data.snapshot_cursor_after_build,
                    data.expiry_tracking_failure_reason,
                ),
            ),
        ))),
        Err(error) => finalize_dashboard_activity_snapshot_cache_last_good_fallback(
            request,
            metrics,
            coalesced_waiter_count,
            db_build_elapsed_ms,
            refresh_reason,
            cache,
            error,
        ),
    }
}
