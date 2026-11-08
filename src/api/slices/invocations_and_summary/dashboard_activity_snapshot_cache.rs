use super::*;
pub(crate) async fn load_dashboard_activity_snapshot_cached(
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
        let started_at = Instant::now();
        let range = resolve_dashboard_activity_exact_range(range_name, reporting_tz)?;
        let snapshot = load_dashboard_activity_snapshot_for_range(
            state,
            range_name,
            reporting_tz,
            range,
            recent_limit,
            include_accounts,
            include_recent,
            in_progress_counts_override.clone(),
        )
        .await?;
        return Ok((
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
        ));
    }

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
    if !dashboard_activity_snapshot_cache_can_track_expiry(
        range_name,
        range,
        shanghai_retention_cutoff(state.config.invocation_max_days),
    ) {
        let started_at = Instant::now();
        let _baseline_build_guard = baseline_build_gate.lock().await;
        let reconcile_gate = state.sqlite_batch_writer.dashboard_reconcile_gate();
        let reconcile_guard = reconcile_gate.lock().await;
        let mut consistency_connection =
            Some(begin_dashboard_activity_consistency_barrier(&state.pool).await?);
        // Once BEGIN IMMEDIATE owns the SQLite write barrier, the batch writer can drain its
        // channel into its retained batch instead of backing up behind this process-local gate.
        drop(reconcile_guard);
        let build_result: Result<_, ApiError> = async {
            let baseline_cursor = if let Some(connection) = consistency_connection.as_mut() {
                query_live_dashboard_activity_snapshot_cursor_tx(connection.as_mut()).await?
            } else {
                query_live_dashboard_activity_snapshot_cursor(&state.pool).await?
            };
            let snapshot = load_dashboard_activity_snapshot_for_range(
                state,
                range_name,
                reporting_tz,
                range,
                recent_limit,
                include_accounts,
                include_recent,
                in_progress_counts_override,
            )
            .await?;
            let pending_terminal_keys_after_build = {
                let cache = state.dashboard_activity_snapshot_cache.lock().await;
                cache
                    .read_model
                    .pending_terminal_deltas
                    .iter()
                    .filter_map(|delta| {
                        let occurred_at = parse_to_utc_datetime(&delta.occurred_at)?;
                        dashboard_activity_selection_includes_compact_terminal(
                            &selection,
                            delta,
                            occurred_at,
                        )
                        .then_some(delta.key())
                    })
                    .collect::<Vec<_>>()
            };
            let persisted_terminal_ids = if let Some(connection) = consistency_connection.as_mut() {
                query_live_dashboard_activity_persisted_terminal_ids_tx(
                    connection.as_mut(),
                    dashboard_activity_selection_source_scope(&selection),
                    &pending_terminal_keys_after_build,
                )
                .await?
            } else {
                HashMap::new()
            };
            Ok((snapshot, baseline_cursor, persisted_terminal_ids))
        }
        .await;
        // Keep ACK pruning behind the exact snapshot replay. Once the consistency transaction
        // is released, the writer may persist and acknowledge a terminal that arrived during the
        // build; without this guard it could remove the pending delta before this older baseline
        // has incorporated it.
        let cache = state.dashboard_activity_snapshot_cache.lock().await;
        if let Some(connection) = consistency_connection.take() {
            finish_dashboard_activity_consistency_barrier(connection, build_result.is_ok()).await?;
        }
        let (mut snapshot, baseline_cursor, persisted_terminal_ids) = build_result?;
        if !dashboard_activity_hard_limit_is_settled(&cache.read_model) {
            return Err(ApiError::from(anyhow!(
                "dashboard activity hard-limit terminal writes are not persisted yet"
            )));
        }
        let replayed_pending_delta_count = replay_dashboard_activity_pending_deltas_without_expiry(
            &mut snapshot,
            &selection,
            &cache.read_model.pending_terminal_deltas,
            baseline_cursor,
            &persisted_terminal_ids,
        );
        tracing::debug!(
            selection_fingerprint = dashboard_activity_selection_fingerprint(&selection),
            replayed_pending_delta_count,
            "replayed pending dashboard deltas over archive-prefix exact snapshot"
        );
        let terminal_delta_count = cache.read_model.terminal_delta_count;
        let duplicate_delta_count = cache.read_model.duplicate_delta_count;
        let pending_delta_count = cache.read_model.pending_terminal_deltas.len();
        let pending_delta_estimated_bytes = cache.read_model.pending_delta_estimated_bytes;
        let persisted_ack_pending_count = cache.read_model.persisted_ack_pending_count;
        let delta_pruned_count = cache.read_model.delta_pruned_count;
        let hard_limit_reason = cache.read_model.hard_limit_reason.unwrap_or("none");
        let sequence_gap_count = cache.read_model.sequence_gap_count;
        return Ok((
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
                terminal_delta_count,
                duplicate_delta_count,
                pending_delta_count,
                pending_delta_estimated_bytes,
                persisted_ack_pending_count,
                delta_pruned_count,
                expiry_delta_count: 0,
                hard_limit_reason,
                baseline_cursor,
                sequence_gap_count,
                build_attempted: true,
                snapshot_origin: "exact_db_with_pending_overlay",
            },
        ));
    }
    let mut waited_on_in_flight = false;
    let mut max_waiter_count = 0_usize;
    let mut saw_expired_entry = false;
    let cache_ttl = dashboard_activity_snapshot_cache_ttl(range_name);
    let cache_ttl_ms = cache_ttl.as_millis() as u64;
    let selection_fingerprint = dashboard_activity_selection_fingerprint(&selection);

    loop {
        let mut wait_on: Option<tokio::sync::watch::Receiver<bool>> = None;
        let mut flight_guard: Option<DashboardActivitySnapshotFlightGuard> = None;
        let routing_rules_generation;
        {
            let mut cache = state.dashboard_activity_snapshot_cache.lock().await;
            routing_rules_generation = cache.routing_rules_generation;
            saw_expired_entry |= cache
                .entries
                .get(&selection)
                .is_some_and(|entry| entry.cached_at.elapsed() > cache_ttl);
            cache.entries.retain(|_, entry| {
                entry.cached_at.elapsed() <= DASHBOARD_ACTIVITY_LAST_GOOD_MAX_AGE
            });
            let cache_entry_count = cache.entries.len();
            let in_flight_count = cache.in_flight.len();
            let terminal_delta_count = cache.read_model.terminal_delta_count;
            let duplicate_delta_count = cache.read_model.duplicate_delta_count;
            let pending_delta_count = cache.read_model.pending_terminal_deltas.len();
            let pending_delta_estimated_bytes = cache.read_model.pending_delta_estimated_bytes;
            let persisted_ack_pending_count = cache.read_model.persisted_ack_pending_count;
            let delta_pruned_count = cache.read_model.delta_pruned_count;
            let hard_limit_reason = cache.read_model.hard_limit_reason.unwrap_or("none");
            let sequence_gap_count = cache.read_model.sequence_gap_count;
            let settled_dirty_state =
                dashboard_activity_read_model_requires_reconcile(&cache.read_model);
            if !settled_dirty_state
                && let Some(entry) = cache.entries.get_mut(&selection)
                && let Some(reuse_mode) =
                    dashboard_activity_snapshot_reuse_mode(entry, range, cache_ttl)
            {
                let mut expired_count = 0usize;
                while entry.expiry_terminal_deltas.front().is_some_and(|delta| {
                    parse_to_utc_datetime(&delta.occurred_at)
                        .is_some_and(|occurred_at| occurred_at < range.start)
                }) {
                    if let Some(delta) = entry.expiry_terminal_deltas.pop_front() {
                        entry.expiry_delta_estimated_bytes = entry
                            .expiry_delta_estimated_bytes
                            .saturating_sub(delta.estimated_bytes);
                        subtract_dashboard_activity_compact_terminal_delta(
                            &mut entry.response,
                            &delta,
                        );
                        restore_dashboard_activity_last_invocation_after_expiry(
                            &mut entry.response,
                            &delta,
                            &entry.expiry_terminal_deltas,
                        );
                        expired_count += 1;
                    }
                }
                let cache_entry_age_ms = entry.cached_at.elapsed().as_millis() as u64;
                let is_last_good =
                    reuse_mode == DashboardActivitySnapshotReuseMode::LastGoodReconcileBackoff;
                return Ok((
                    entry.response.clone(),
                    DashboardActivitySnapshotCacheOutcome {
                        cache_hit_or_miss: if is_last_good {
                            "last_good_fallback"
                        } else if waited_on_in_flight {
                            "wait_on_in_flight"
                        } else {
                            "cache_hit"
                        },
                        cache_bypass_reason: if is_last_good {
                            "expiry_reconcile_backoff"
                        } else {
                            "none"
                        },
                        coalesced_waiter_count: max_waiter_count,
                        db_build_elapsed_ms: 0,
                        cache_ttl_ms,
                        cache_entry_age_ms,
                        cache_entry_count,
                        in_flight_count,
                        refresh_reason: if is_last_good {
                            "reconcile_backoff"
                        } else {
                            "within_ttl"
                        },
                        selection_fingerprint,
                        terminal_delta_count,
                        duplicate_delta_count,
                        pending_delta_count,
                        pending_delta_estimated_bytes,
                        persisted_ack_pending_count,
                        delta_pruned_count,
                        expiry_delta_count: expired_count,
                        hard_limit_reason,
                        baseline_cursor: entry.baseline_snapshot_cursor,
                        sequence_gap_count,
                        build_attempted: false,
                        snapshot_origin: if is_last_good { "last_good" } else { "memory" },
                    },
                ));
            }

            let writer_pressure = crate::db_pressure::global_db_pressure_gate()
                .snapshot()
                .pressure_cooldown_remaining_ms
                > 0;
            if !settled_dirty_state
                && writer_pressure
                && let Some(entry) = cache.entries.get_mut(&selection)
                && dashboard_activity_entry_expiry_covers(entry, range)
                && dashboard_activity_pressure_reconcile_deferred(entry)
            {
                let mut expired_count = 0usize;
                while entry.expiry_terminal_deltas.front().is_some_and(|delta| {
                    parse_to_utc_datetime(&delta.occurred_at)
                        .is_some_and(|occurred_at| occurred_at < range.start)
                }) {
                    if let Some(delta) = entry.expiry_terminal_deltas.pop_front() {
                        entry.expiry_delta_estimated_bytes = entry
                            .expiry_delta_estimated_bytes
                            .saturating_sub(delta.estimated_bytes);
                        subtract_dashboard_activity_compact_terminal_delta(
                            &mut entry.response,
                            &delta,
                        );
                        restore_dashboard_activity_last_invocation_after_expiry(
                            &mut entry.response,
                            &delta,
                            &entry.expiry_terminal_deltas,
                        );
                        expired_count += 1;
                    }
                }
                entry.last_reconcile_attempted_at = Instant::now();
                entry.last_reconcile_failed = true;
                let cache_entry_age_ms = entry.cached_at.elapsed().as_millis() as u64;
                return Ok((
                    entry.response.clone(),
                    DashboardActivitySnapshotCacheOutcome {
                        cache_hit_or_miss: "last_good_fallback",
                        cache_bypass_reason: "writer_pressure",
                        coalesced_waiter_count: max_waiter_count,
                        db_build_elapsed_ms: 0,
                        cache_ttl_ms,
                        cache_entry_age_ms,
                        cache_entry_count,
                        in_flight_count,
                        refresh_reason: "reconcile_deferred",
                        selection_fingerprint,
                        terminal_delta_count,
                        duplicate_delta_count,
                        pending_delta_count,
                        pending_delta_estimated_bytes,
                        persisted_ack_pending_count,
                        delta_pruned_count,
                        expiry_delta_count: expired_count,
                        hard_limit_reason,
                        baseline_cursor: entry.baseline_snapshot_cursor,
                        sequence_gap_count,
                        build_attempted: false,
                        snapshot_origin: "memory",
                    },
                ));
            }

            if let Some(in_flight) = cache.in_flight.get_mut(&selection) {
                in_flight.waiter_count += 1;
                max_waiter_count = max_waiter_count.max(in_flight.waiter_count);
                wait_on = Some(in_flight.signal.subscribe());
            } else {
                let (signal, _receiver) = tokio::sync::watch::channel(false);
                cache.in_flight.insert(
                    selection.clone(),
                    DashboardActivitySnapshotInFlight {
                        signal,
                        waiter_count: 0,
                        baseline_cursor: None,
                        routing_rules_generation,
                    },
                );
                flight_guard = Some(DashboardActivitySnapshotFlightGuard::new(
                    state.dashboard_activity_snapshot_cache.clone(),
                    selection.clone(),
                    routing_rules_generation,
                ));
            }
        }

        if let Some(mut receiver) = wait_on {
            waited_on_in_flight = true;
            if !*receiver.borrow() {
                let _ = receiver.changed().await;
            }
            continue;
        }

        let build_started_at = Instant::now();
        let _baseline_build_guard = baseline_build_gate.lock().await;
        let reconcile_gate = state.sqlite_batch_writer.dashboard_reconcile_gate();
        let reconcile_guard = reconcile_gate.lock().await;
        let has_last_good = {
            let cache = state.dashboard_activity_snapshot_cache.lock().await;
            cache.entries.contains_key(&selection)
        };
        let mut consistency_connection = if has_last_good {
            try_begin_dashboard_activity_consistency_barrier(&state.pool).await?
        } else {
            Some(begin_dashboard_activity_consistency_barrier(&state.pool).await?)
        };
        // SQLite now serializes commits against the consistency transaction. Releasing the
        // process-local gate lets the writer retain failed work without filling its input queue.
        drop(reconcile_guard);
        let consistency_barrier_unavailable = consistency_connection.is_none();
        if consistency_barrier_unavailable {
            let mut cache = state.dashboard_activity_snapshot_cache.lock().await;
            if let Some(entry) = cache.entries.get_mut(&selection) {
                mark_dashboard_activity_reconcile_failed(entry);
                let response = entry.response.clone();
                let cache_entry_age_ms = entry.cached_at.elapsed().as_millis() as u64;
                let baseline_cursor = entry.baseline_snapshot_cursor;
                let in_flight = cache.in_flight.remove(&selection);
                if let Some(guard) = flight_guard.as_mut() {
                    guard.disarm();
                }
                let coalesced_waiter_count =
                    in_flight.as_ref().map_or(0, |flight| flight.waiter_count);
                if let Some(in_flight) = in_flight {
                    let _ = in_flight.signal.send(true);
                }
                return Ok((
                    response,
                    DashboardActivitySnapshotCacheOutcome {
                        cache_hit_or_miss: "last_good_fallback",
                        cache_bypass_reason: "consistency_barrier_unavailable",
                        coalesced_waiter_count,
                        db_build_elapsed_ms: build_started_at.elapsed().as_millis() as u64,
                        cache_ttl_ms,
                        cache_entry_age_ms,
                        cache_entry_count: cache.entries.len(),
                        in_flight_count: cache.in_flight.len(),
                        refresh_reason: "reconcile_lock_contended",
                        selection_fingerprint,
                        terminal_delta_count: cache.read_model.terminal_delta_count,
                        duplicate_delta_count: cache.read_model.duplicate_delta_count,
                        pending_delta_count: cache.read_model.pending_terminal_deltas.len(),
                        pending_delta_estimated_bytes: cache
                            .read_model
                            .pending_delta_estimated_bytes,
                        persisted_ack_pending_count: cache.read_model.persisted_ack_pending_count,
                        delta_pruned_count: cache.read_model.delta_pruned_count,
                        expiry_delta_count: 0,
                        hard_limit_reason: cache.read_model.hard_limit_reason.unwrap_or("none"),
                        baseline_cursor,
                        sequence_gap_count: cache.read_model.sequence_gap_count,
                        build_attempted: false,
                        snapshot_origin: "last_good",
                    },
                ));
            }
            let in_flight = cache.in_flight.remove(&selection);
            if let Some(guard) = flight_guard.as_mut() {
                guard.disarm();
            }
            if let Some(in_flight) = in_flight {
                let _ = in_flight.signal.send(true);
            }
            return Err(ApiError::from(anyhow!(
                "dashboard activity consistency barrier unavailable for initial snapshot"
            )));
        }
        let (snapshot_cursor_before_build, probe_error) =
            match if let Some(connection) = consistency_connection.as_mut() {
                query_live_dashboard_activity_snapshot_cursor_tx(connection.as_mut()).await
            } else {
                query_live_dashboard_activity_snapshot_cursor(&state.pool).await
            } {
                Ok(snapshot_cursor) => (snapshot_cursor, None),
                Err(err) => (0, Some(err)),
            };
        {
            let mut cache = state.dashboard_activity_snapshot_cache.lock().await;
            if let Some(flight) = cache.in_flight.get_mut(&selection) {
                flight.baseline_cursor = Some(snapshot_cursor_before_build);
            }
        }
        let mut result = if let Some(err) = probe_error {
            Err(err)
        } else {
            load_dashboard_activity_snapshot_for_range(
                state,
                range_name,
                reporting_tz,
                range,
                recent_limit,
                include_accounts,
                include_recent,
                in_progress_counts_override.clone(),
            )
            .await
        };
        let (snapshot_cursor_after_build, persisted_terminal_ids_after_build) = if result.is_ok() {
            let pending_terminal_keys_after_build = {
                let cache = state.dashboard_activity_snapshot_cache.lock().await;
                cache
                    .read_model
                    .pending_terminal_deltas
                    .iter()
                    .filter_map(|delta| {
                        let occurred_at = parse_to_utc_datetime(&delta.occurred_at)?;
                        dashboard_activity_selection_includes_compact_terminal(
                            &selection,
                            delta,
                            occurred_at,
                        )
                        .then_some(delta.key())
                    })
                    .collect::<Vec<_>>()
            };
            let probe = if let Some(connection) = consistency_connection.as_mut() {
                query_live_dashboard_activity_persisted_terminal_ids_tx(
                    connection.as_mut(),
                    dashboard_activity_selection_source_scope(&selection),
                    &pending_terminal_keys_after_build,
                )
                .await
                .map(|ids| (snapshot_cursor_before_build, ids))
            } else {
                Ok((snapshot_cursor_before_build, HashMap::new()))
            };
            match probe {
                Ok(probe) => probe,
                Err(err) => {
                    result = Err(err);
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
                &state.pool,
                &selection,
                range,
                snapshot_cursor_after_build,
            )
            .await
            {
                Ok(expiry) => expiry,
                Err(err) => {
                    result = Err(err);
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
        if let Some(connection) = consistency_connection.take()
            && let Err(err) =
                finish_dashboard_activity_consistency_barrier(connection, result.is_ok()).await
        {
            result = Err(err);
        }
        let db_build_elapsed_ms = build_started_at.elapsed().as_millis() as u64;

        let mut cache = state.dashboard_activity_snapshot_cache.lock().await;
        let refresh_reason =
            cache
                .invalidation_reasons
                .remove(&selection)
                .unwrap_or(if saw_expired_entry {
                    "ttl_expired"
                } else {
                    "initial_build"
                });
        let in_flight = cache.in_flight.remove(&selection);
        if let Some(guard) = flight_guard.as_mut() {
            guard.disarm();
        }
        let coalesced_waiter_count = in_flight.as_ref().map_or(0, |flight| flight.waiter_count);
        let routing_rules_changed = cache.routing_rules_generation != routing_rules_generation
            || in_flight
                .as_ref()
                .is_some_and(|flight| flight.routing_rules_generation != routing_rules_generation);
        if routing_rules_changed && result.is_ok() {
            result = Err(ApiError::from(anyhow!(
                "dashboard activity routing rules changed during build"
            )));
        }
        if result.is_ok() && !dashboard_activity_hard_limit_is_settled(&cache.read_model) {
            result = Err(ApiError::from(anyhow!(
                "dashboard activity hard-limit terminal writes are not persisted yet"
            )));
        }
        if let Ok(snapshot) = result.as_mut() {
            // Pending records are shared by every in-flight selection. Do not consume them from
            // the first completed build: another selection may have started before the same
            // terminal reached SQLite and still needs to replay its delta.
            for delta in &cache.read_model.pending_terminal_deltas {
                let Some(occurred_at) = parse_to_utc_datetime(&delta.occurred_at) else {
                    continue;
                };
                if dashboard_activity_selection_includes_compact_terminal(
                    &selection,
                    delta,
                    occurred_at,
                ) {
                    if dashboard_activity_baseline_includes_pending_delta(
                        delta,
                        snapshot_cursor_after_build,
                        &persisted_terminal_ids_after_build,
                    ) {
                        snapshot.terminal_sequence =
                            snapshot.terminal_sequence.max(delta.terminal_sequence);
                        continue;
                    }
                    if expiry_tracking_failure_reason.is_none()
                        && let Err(reason) = insert_dashboard_activity_expiry_delta(
                            &mut expiry_terminal_deltas,
                            &mut expiry_delta_estimated_bytes,
                            expiry_covered_until,
                            delta,
                            occurred_at,
                        )
                    {
                        expiry_tracking_failure_reason = Some(reason);
                        expiry_terminal_deltas.clear();
                        expiry_delta_estimated_bytes = 0;
                        expiry_covered_until = None;
                    }
                    apply_dashboard_activity_compact_terminal_delta(snapshot, delta);
                }
            }
        }
        if let Some(in_flight) = in_flight {
            if let Ok(snapshot) = &result
                && !routing_rules_changed
                && expiry_tracking_failure_reason.is_none()
            {
                cache.entries.insert(
                    selection.clone(),
                    DashboardActivitySnapshotCacheEntry {
                        cached_at: Instant::now(),
                        last_reconcile_attempted_at: Instant::now(),
                        last_reconcile_failed: false,
                        baseline_snapshot_cursor: snapshot_cursor_after_build,
                        expiry_covered_until,
                        expiry_terminal_deltas,
                        expiry_delta_estimated_bytes,
                        response: snapshot.clone(),
                    },
                );
                let evicted_entry_count =
                    prune_dashboard_activity_snapshot_entries(&mut cache, Some(&selection));
                if evicted_entry_count > 0 {
                    tracing::debug!(
                        selection_fingerprint,
                        evicted_entry_count,
                        cache_entry_count = cache.entries.len(),
                        "evicted dashboard activity snapshots after baseline insertion"
                    );
                }
                prune_dashboard_activity_terminal_deltas(&mut cache);
                clear_dashboard_activity_hard_limit_after_baseline(&mut cache.read_model);
            } else if expiry_tracking_failure_reason.is_some() {
                cache.entries.remove(&selection);
            }
            let _ = in_flight.signal.send(true);
        }
        if routing_rules_changed {
            continue;
        }
        let cache_entry_count = cache.entries.len();
        let in_flight_count = cache.in_flight.len();
        let terminal_delta_count = cache.read_model.terminal_delta_count;
        let duplicate_delta_count = cache.read_model.duplicate_delta_count;
        let pending_delta_count = cache.read_model.pending_terminal_deltas.len();
        let pending_delta_estimated_bytes = cache.read_model.pending_delta_estimated_bytes;
        let persisted_ack_pending_count = cache.read_model.persisted_ack_pending_count;
        let delta_pruned_count = cache.read_model.delta_pruned_count;
        let hard_limit_reason = cache.read_model.hard_limit_reason.unwrap_or("none");
        let sequence_gap_count = cache.read_model.sequence_gap_count;

        return match result {
            Ok(snapshot) => Ok((
                snapshot,
                DashboardActivitySnapshotCacheOutcome {
                    cache_hit_or_miss: if expiry_tracking_failure_reason.is_some() {
                        "uncached"
                    } else {
                        "cache_miss_build"
                    },
                    cache_bypass_reason: expiry_tracking_failure_reason.unwrap_or("none"),
                    coalesced_waiter_count,
                    db_build_elapsed_ms,
                    cache_ttl_ms,
                    cache_entry_age_ms: 0,
                    cache_entry_count,
                    in_flight_count,
                    refresh_reason,
                    selection_fingerprint,
                    terminal_delta_count,
                    duplicate_delta_count,
                    pending_delta_count,
                    pending_delta_estimated_bytes,
                    persisted_ack_pending_count,
                    delta_pruned_count,
                    expiry_delta_count: 0,
                    hard_limit_reason,
                    baseline_cursor: snapshot_cursor_after_build,
                    sequence_gap_count,
                    build_attempted: true,
                    snapshot_origin: if expiry_tracking_failure_reason.is_some() {
                        "exact_db_expiry_untracked"
                    } else {
                        "db_reconciled"
                    },
                },
            )),
            Err(err) => {
                let Some(entry) = cache.entries.get_mut(&selection) else {
                    return Err(err);
                };
                mark_dashboard_activity_reconcile_failed(entry);
                let cache_entry_age_ms = entry.cached_at.elapsed().as_millis() as u64;
                tracing::warn!(
                    selection_fingerprint,
                    refresh_reason,
                    cache_entry_age_ms,
                    db_build_elapsed_ms,
                    error = ?err,
                    "dashboard activity reconciliation failed; retained last-good snapshot"
                );
                Ok((
                    entry.response.clone(),
                    DashboardActivitySnapshotCacheOutcome {
                        cache_hit_or_miss: "last_good_fallback",
                        cache_bypass_reason: "reconcile_failed",
                        coalesced_waiter_count,
                        db_build_elapsed_ms,
                        cache_ttl_ms,
                        cache_entry_age_ms,
                        cache_entry_count,
                        in_flight_count,
                        refresh_reason: "reconcile_failed",
                        selection_fingerprint,
                        terminal_delta_count,
                        duplicate_delta_count,
                        pending_delta_count,
                        pending_delta_estimated_bytes,
                        persisted_ack_pending_count,
                        delta_pruned_count,
                        expiry_delta_count: 0,
                        hard_limit_reason,
                        baseline_cursor: entry.baseline_snapshot_cursor,
                        sequence_gap_count,
                        build_attempted: true,
                        snapshot_origin: "last_good",
                    },
                ))
            }
        };
    }
}
