fn add_timeseries_delta_projection_keys(
    grouped: &mut HashMap<TimeseriesMinuteProjectionKey, Vec<(i64, TimeseriesTerminalDelta)>>,
    row_id: i64,
    delta: TimeseriesTerminalDelta,
) {
    let Some(occurred) = parse_to_utc_datetime(&delta.occurred_at) else {
        return;
    };
    let minute_start_epoch = occurred.timestamp().div_euclid(60) * 60;
    let account_key = delta.upstream_account_id.unwrap_or(-1);
    let mut keys = vec![TimeseriesMinuteProjectionKey {
        minute_start_epoch,
        source_scope: "all",
        upstream_account_key: -1,
    }];
    if account_key != -1 {
        keys.push(TimeseriesMinuteProjectionKey {
            minute_start_epoch,
            source_scope: "all",
            upstream_account_key: account_key,
        });
    }
    if delta.source == SOURCE_PROXY {
        keys.push(TimeseriesMinuteProjectionKey {
            minute_start_epoch,
            source_scope: "proxy_only",
            upstream_account_key: -1,
        });
        if account_key != -1 {
            keys.push(TimeseriesMinuteProjectionKey {
                minute_start_epoch,
                source_scope: "proxy_only",
                upstream_account_key: account_key,
            });
        }
    }
    for key in keys {
        grouped
            .entry(key)
            .or_default()
            .push((row_id, delta.clone()));
    }
}

fn merge_timeseries_bucket_aggregate(target: &mut BucketAggregate, source: BucketAggregate) {
    target.total_count += source.total_count;
    target.success_count += source.success_count;
    target.failure_count += source.failure_count;
    target.in_flight_count += source.in_flight_count;
    target.in_flight_phase_counts.queued += source.in_flight_phase_counts.queued;
    target.in_flight_phase_counts.requesting += source.in_flight_phase_counts.requesting;
    target.in_flight_phase_counts.responding += source.in_flight_phase_counts.responding;
    target.total_tokens += source.total_tokens;
    target.input_tokens += source.input_tokens;
    target.output_tokens += source.output_tokens;
    target.cache_input_tokens += source.cache_input_tokens;
    target.reasoning_tokens += source.reasoning_tokens;
    target.token_components_observed |= source.token_components_observed;
    target.token_component_incomplete_count += source.token_component_incomplete_count;
    target.total_cost += source.total_cost;
    target.non_success_cost += source.non_success_cost;
    target.total_latency_sum_ms += source.total_latency_sum_ms;
    target.total_latency_sample_count += source.total_latency_sample_count;
    target
        .total_latency_values
        .extend(source.total_latency_values);
    target.first_byte_ttfb_sum_ms += source.first_byte_ttfb_sum_ms;
    target.first_byte_sample_count += source.first_byte_sample_count;
    target.first_response_byte_total_sum_ms += source.first_response_byte_total_sum_ms;
    target.first_response_byte_total_sample_count += source.first_response_byte_total_sample_count;
    target.first_token_sum_ms += source.first_token_sum_ms;
    target.first_token_sample_count += source.first_token_sample_count;
    target
        .first_byte_ttfb_values
        .extend(source.first_byte_ttfb_values);
    target
        .first_response_byte_total_values
        .extend(source.first_response_byte_total_values);
    target.first_token_values.extend(source.first_token_values);
    let first_byte_histogram = source.first_byte_histogram;
    if target.first_byte_histogram.is_empty() {
        target.first_byte_histogram = first_byte_histogram;
    } else {
        for (target_value, source_value) in target
            .first_byte_histogram
            .iter_mut()
            .zip(first_byte_histogram)
        {
            *target_value += source_value;
        }
    }
    let first_response_byte_total_histogram = source.first_response_byte_total_histogram;
    if target.first_response_byte_total_histogram.is_empty() {
        target.first_response_byte_total_histogram = first_response_byte_total_histogram;
    } else {
        for (target_value, source_value) in target
            .first_response_byte_total_histogram
            .iter_mut()
            .zip(first_response_byte_total_histogram)
        {
            *target_value += source_value;
        }
    }
    let first_token_histogram = source.first_token_histogram;
    if target.first_token_histogram.is_empty() {
        target.first_token_histogram = first_token_histogram;
    } else {
        for (target_value, source_value) in target
            .first_token_histogram
            .iter_mut()
            .zip(first_token_histogram)
        {
            *target_value += source_value;
        }
    }
}

async fn load_timeseries_minute_projection_v2_key(
    pool: &Pool<Sqlite>,
    key: &TimeseriesMinuteProjectionKey,
) -> Result<Option<(BucketAggregate, i64)>, ApiError> {
    let row = sqlx::query_as::<_, TimeseriesMinuteProjectionV2Row>(
        "SELECT minute_start_epoch, aggregate_json, total_latency_samples_json, first_byte_samples_json, first_response_byte_total_samples_json, first_token_samples_json, max_row_id, coverage_state FROM timeseries_minute_projection_v2 WHERE minute_start_epoch = ?1 AND source_scope = ?2 AND upstream_account_key = ?3",
    )
    .bind(key.minute_start_epoch)
    .bind(key.source_scope)
    .bind(key.upstream_account_key)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.coverage_state != "ready" {
        return Ok(None);
    }
    let mut aggregate = serde_json::from_str::<BucketAggregate>(&row.aggregate_json)
        .map_err(|err| ApiError::from(anyhow!("invalid v2 minute aggregate: {err}")))?;
    aggregate.total_latency_values = serde_json::from_str(&row.total_latency_samples_json)
        .map_err(|err| ApiError::from(anyhow!("invalid v2 total-latency samples: {err}")))?;
    aggregate.first_byte_ttfb_values = serde_json::from_str(&row.first_byte_samples_json)
        .map_err(|err| ApiError::from(anyhow!("invalid v2 first-byte samples: {err}")))?;
    aggregate.first_response_byte_total_values =
        serde_json::from_str(&row.first_response_byte_total_samples_json)
            .map_err(|err| ApiError::from(anyhow!("invalid v2 first-response samples: {err}")))?;
    aggregate.first_token_values = serde_json::from_str(&row.first_token_samples_json)
        .map_err(|err| ApiError::from(anyhow!("invalid v2 first-token samples: {err}")))?;
    Ok(Some((aggregate, row.max_row_id)))
}

async fn rebuild_timeseries_minute_projection_v2_key(
    pool: &Pool<Sqlite>,
    key: &TimeseriesMinuteProjectionKey,
    snapshot_id: Option<i64>,
) -> Result<(BucketAggregate, i64, u64), ApiError> {
    let start = Utc
        .timestamp_opt(key.minute_start_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid minute projection start")))?;
    let end = start + ChronoDuration::minutes(1);
    let range = ExactUtcRange { start, end };
    let source_scope = if key.source_scope == "proxy_only" {
        InvocationSourceScope::ProxyOnly
    } else {
        InvocationSourceScope::All
    };
    let records = if key.upstream_account_key == -1 {
        query_invocation_aggregate_records_from_live_range(
            pool,
            range,
            source_scope,
            None,
            snapshot_id,
        )
        .await?
    } else {
        query_invocation_aggregate_records_from_live_range_for_account(
            pool,
            range,
            source_scope,
            None,
            snapshot_id,
            key.upstream_account_key,
        )
        .await?
    };
    let mut aggregate = BucketAggregate::default();
    let mut max_row_id = 0;
    let source_row_count = records.len() as u64;
    for record in records {
        if prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
            continue;
        }
        max_row_id = max_row_id.max(record.id);
        add_exact_record_to_timeseries_aggregate(&mut aggregate, &record);
    }
    Ok((aggregate, max_row_id, source_row_count))
}

fn prepare_timeseries_minute_projection_v2_write(
    key: &TimeseriesMinuteProjectionKey,
    aggregate: &BucketAggregate,
    max_row_id: i64,
) -> Result<TimeseriesMinuteProjectionPreparedWrite, ApiError> {
    Ok(TimeseriesMinuteProjectionPreparedWrite {
        key: key.clone(),
        aggregate_json: serde_json::to_string(aggregate).map_err(ApiError::from)?,
        total_latency_samples_json: serde_json::to_string(&aggregate.total_latency_values)
            .map_err(ApiError::from)?,
        first_byte_samples_json: serde_json::to_string(&aggregate.first_byte_ttfb_values)
            .map_err(ApiError::from)?,
        first_response_byte_total_samples_json: serde_json::to_string(
            &aggregate.first_response_byte_total_values,
        )
        .map_err(ApiError::from)?,
        first_token_samples_json: serde_json::to_string(&aggregate.first_token_values)
            .map_err(ApiError::from)?,
        max_row_id,
    })
}

async fn upsert_timeseries_minute_projection_v2_prepared_write_tx(
    tx: &mut SqliteConnection,
    write: &TimeseriesMinuteProjectionPreparedWrite,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO timeseries_minute_projection_v2 (minute_start_epoch, source_scope, upstream_account_key, aggregate_json, total_latency_samples_json, first_byte_samples_json, first_response_byte_total_samples_json, first_token_samples_json, max_row_id, coverage_state, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'ready', datetime('now')) ON CONFLICT(minute_start_epoch, source_scope, upstream_account_key) DO UPDATE SET aggregate_json = excluded.aggregate_json, total_latency_samples_json = excluded.total_latency_samples_json, first_byte_samples_json = excluded.first_byte_samples_json, first_response_byte_total_samples_json = excluded.first_response_byte_total_samples_json, first_token_samples_json = excluded.first_token_samples_json, max_row_id = MAX(timeseries_minute_projection_v2.max_row_id, excluded.max_row_id), coverage_state = 'ready', updated_at = excluded.updated_at",
    )
    .bind(write.key.minute_start_epoch)
    .bind(write.key.source_scope)
    .bind(write.key.upstream_account_key)
    .bind(&write.aggregate_json)
    .bind(&write.total_latency_samples_json)
    .bind(&write.first_byte_samples_json)
    .bind(&write.first_response_byte_total_samples_json)
    .bind(&write.first_token_samples_json)
    .bind(write.max_row_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn try_acquire_timeseries_minute_projection_write(
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
    pressure_gate: &crate::db_pressure::DbPressureGate,
    trigger: &'static str,
    transaction_phase: &'static str,
    pending_event_count: usize,
) -> TimeseriesMinuteProjectionWriteAdmissionOutcome {
    let observed_eligibility_generation = pressure_gate.eligibility_generation();
    let Some(mut write_permit) = coordinator
        .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
    else {
        let snapshot = coordinator.snapshot().await;
        debug!(
            route = "timeseries_projection",
            builder = "minute_projection_v2",
            trigger,
            transaction_phase,
            admission_outcome = "deferred",
            defer_reason = "coordinator_priority",
            active_write_class = snapshot.active_write_class.as_deref().unwrap_or("none"),
            p1_waiter_count = snapshot.p1_waiter_count,
            interactive_waiter_count = snapshot.interactive_waiter_count,
            p2_waiter_count = snapshot.p2_waiter_count,
            pending_event_count,
            "minute projection deferred before opening a P2 write transaction"
        );
        return TimeseriesMinuteProjectionWriteAdmissionOutcome::Deferred(
            TimeseriesMinuteProjectionDeferred { retry_after: None },
        );
    };

    match pressure_gate.try_begin_background("timeseries_minute_projection_flush") {
        Ok(pressure_permit) => TimeseriesMinuteProjectionWriteAdmissionOutcome::Acquired(
            TimeseriesMinuteProjectionWriteAdmission {
                _write_permit: write_permit,
                _pressure_permit: pressure_permit,
            },
        ),
        Err(reason) => {
            let retry_after = match reason {
                crate::db_pressure::DbPressureDenyReason::PressureCooldown { remaining_ms } => {
                    Some(Duration::from_millis(remaining_ms.max(1)))
                }
                crate::db_pressure::DbPressureDenyReason::BackgroundBusy => None,
            };
            // This acquisition never opened a background transaction. Its P2 permit must not
            // satisfy the retry wait that was registered before this admission attempt.
            write_permit.suppress_background_eligibility_wakeup();
            drop(write_permit);
            debug!(
                route = "timeseries_projection",
                builder = "minute_projection_v2",
                trigger,
                transaction_phase,
                admission_outcome = "deferred",
                defer_reason = "writer_pressure",
                observed_eligibility_generation,
                pending_event_count,
                %reason,
                "minute projection deferred before opening a P2 write transaction"
            );
            TimeseriesMinuteProjectionWriteAdmissionOutcome::Deferred(
                TimeseriesMinuteProjectionDeferred { retry_after },
            )
        }
    }
}

async fn invalidate_timeseries_minute_projection_coverage(
    state: &AppState,
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
    trigger: &'static str,
    pending_event_count: usize,
    cancellation: Option<&tokio_util::sync::CancellationToken>,
) -> Result<TimeseriesMinuteProjectionCoverageInvalidationOutcome, ApiError> {
    let mut stats = TimeseriesMinuteProjectionCoverageInvalidationStats::default();
    loop {
        if timeseries_minute_projection_is_cancelled(cancellation) {
            return Ok(TimeseriesMinuteProjectionCoverageInvalidationOutcome::Cancelled);
        }
        let row_ids = sqlx::query_scalar::<_, i64>(
            "SELECT rowid FROM timeseries_minute_projection_v2 WHERE coverage_state <> 'warming' ORDER BY rowid LIMIT ?1",
        )
        .bind(TIMESERIES_MINUTE_PROJECTION_INVALIDATION_ROW_BATCH_LIMIT)
        .fetch_all(&state.pool)
        .await?;
        if row_ids.is_empty() {
            return Ok(TimeseriesMinuteProjectionCoverageInvalidationOutcome::Invalidated(stats));
        }

        let admission = match try_acquire_timeseries_minute_projection_write(
            coordinator,
            crate::db_pressure::global_db_pressure_gate(),
            trigger,
            "coverage_invalidation",
            pending_event_count,
        )
        .await
        {
            TimeseriesMinuteProjectionWriteAdmissionOutcome::Acquired(admission) => admission,
            TimeseriesMinuteProjectionWriteAdmissionOutcome::Deferred(deferred) => {
                return Ok(
                    TimeseriesMinuteProjectionCoverageInvalidationOutcome::Deferred(deferred),
                );
            }
        };

        let started = Instant::now();
        let mut tx = state.pool.begin().await?;
        let mut query = QueryBuilder::<Sqlite>::new(
            "UPDATE timeseries_minute_projection_v2 SET coverage_state = 'warming' WHERE rowid IN (",
        );
        {
            let mut separated = query.separated(", ");
            for row_id in &row_ids {
                separated.push_bind(row_id);
            }
        }
        query.push(")");
        let updated_rows = query.build().execute(tx.as_mut()).await?.rows_affected();
        tx.commit().await?;
        stats.row_count = stats.row_count.saturating_add(updated_rows);
        stats.transaction_count = stats.transaction_count.saturating_add(1);
        debug!(
            route = "timeseries_projection",
            builder = "minute_projection_v2",
            trigger,
            transaction_phase = "coverage_invalidation",
            transaction_row_limit = TIMESERIES_MINUTE_PROJECTION_INVALIDATION_ROW_BATCH_LIMIT,
            transaction_row_count = updated_rows,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "invalidated a bounded minute projection coverage slice"
        );
        drop(admission);
        tokio::task::yield_now().await;
    }
}

pub(crate) async fn flush_timeseries_minute_projection(
    state: &AppState,
    trigger: &'static str,
) -> Result<TimeseriesMinuteProjectionFlushOutcome, ApiError> {
    flush_timeseries_minute_projection_with_coordinator(
        state,
        trigger,
        &crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator(),
    )
    .await
}

pub(crate) async fn flush_timeseries_minute_projection_with_coordinator(
    state: &AppState,
    trigger: &'static str,
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
) -> Result<TimeseriesMinuteProjectionFlushOutcome, ApiError> {
    flush_timeseries_minute_projection_with_coordinator_and_cancellation(
        state,
        trigger,
        coordinator,
        None,
    )
    .await
}

async fn flush_timeseries_minute_projection_with_coordinator_and_cancellation(
    state: &AppState,
    trigger: &'static str,
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
    cancellation: Option<&tokio_util::sync::CancellationToken>,
) -> Result<TimeseriesMinuteProjectionFlushOutcome, ApiError> {
    if timeseries_minute_projection_is_cancelled(cancellation) {
        return Ok(TimeseriesMinuteProjectionFlushOutcome::Cancelled);
    }
    if timeseries_minute_projection_recovery_pending(&state.pool).await? {
        let startup_cancellation = cancellation
            .cloned()
            .unwrap_or_else(tokio_util::sync::CancellationToken::new);
        let recovery =
            prepare_timeseries_minute_projection_after_restart(state, &startup_cancellation)
                .await?;
        if recovery != TimeseriesMinuteProjectionFlushOutcome::Flushed {
            return Ok(recovery);
        }
    }
    let pending = state
        .terminal_projection_hub
        .pending_timeseries_deltas(10_000);
    let coverage_generation = state
        .terminal_projection_hub
        .timeseries_coverage_invalidation_pending();
    if pending.is_empty() && coverage_generation.is_none() {
        return Ok(TimeseriesMinuteProjectionFlushOutcome::Flushed);
    }
    let memory_baseline = state.memory_diagnostics.begin_operation(state).await;
    let result = flush_timeseries_pending_deltas(TimeseriesPendingFlushRequest {
        state,
        trigger,
        coordinator,
        cancellation,
        pending,
        coverage_generation,
    })
    .await;
    let loaded_row_count = result
        .as_ref()
        .map(|stats| stats.loaded_row_count)
        .unwrap_or_default();
    state
        .memory_diagnostics
        .observe_operation(
            state,
            "timeseries_minute_projection_flush",
            memory_baseline,
            loaded_row_count,
            true,
        )
        .await;
    match result {
        Ok(outcome) => Ok(outcome.outcome),
        Err(error) => {
            let gate = crate::db_pressure::global_db_pressure_gate();
            if let Some(deferred) = timeseries_minute_projection_pressure_deferred(
                gate,
                "timeseries_minute_projection_flush",
                &error,
            ) {
                debug!(
                    route = "timeseries_projection",
                    builder = "minute_projection_v2",
                    trigger,
                    defer_reason = "sqlite_pressure",
                    retry_after_ms = deferred.retry_after.map(|value| value.as_millis() as u64),
                    "minute projection flush deferred after a database pressure error"
                );
                Ok(TimeseriesMinuteProjectionFlushOutcome::Deferred(deferred))
            } else {
                Err(error)
            }
        }
    }
}

struct TimeseriesPendingFlushRequest<'a> {
    state: &'a AppState,
    trigger: &'static str,
    coordinator: &'a Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
    cancellation: Option<&'a tokio_util::sync::CancellationToken>,
    pending: Vec<(u64, i64, TimeseriesTerminalDelta)>,
    coverage_generation: Option<u64>,
}

struct TimeseriesPendingFlushStats {
    outcome: TimeseriesMinuteProjectionFlushOutcome,
    loaded_row_count: u64,
    written_key_count: usize,
    exact_fallback_minute_count: usize,
    transaction_count: usize,
    coverage: TimeseriesMinuteProjectionCoverageInvalidationStats,
    event_count: usize,
}

async fn flush_timeseries_pending_deltas(
    request: TimeseriesPendingFlushRequest<'_>,
) -> Result<TimeseriesPendingFlushStats, ApiError> {
    let TimeseriesPendingFlushRequest {
        state,
        trigger,
        coordinator,
        cancellation,
        pending,
        coverage_generation,
    } = request;
    let started = Instant::now();
    let mut grouped = HashMap::new();
    let mut event_ids = Vec::with_capacity(pending.len());
    for (event_id, row_id, delta) in pending {
        event_ids.push(event_id);
        add_timeseries_delta_projection_keys(&mut grouped, row_id, delta);
    }
    let coverage = match prepare_timeseries_pending_coverage(
        state,
        coordinator,
        trigger,
        cancellation,
        coverage_generation,
        event_ids.len(),
    )
    .await?
    {
        TimeseriesPendingCoverageDecision::Continue(stats) => stats,
        TimeseriesPendingCoverageDecision::Complete(stats) => return Ok(stats),
    };
    let mut batches = grouped.into_iter().collect::<Vec<_>>();
    batches.sort_by_key(|(key, _)| {
        (
            key.minute_start_epoch,
            key.source_scope,
            key.upstream_account_key,
        )
    });
    let batches = build_timeseries_projection_work_batches(batches);
    let mut stats = TimeseriesPendingFlushStats {
        outcome: TimeseriesMinuteProjectionFlushOutcome::Flushed,
        loaded_row_count: 0,
        written_key_count: 0,
        exact_fallback_minute_count: 0,
        transaction_count: 0,
        coverage,
        event_count: event_ids.len(),
    };
    for key_batch in batches {
        if timeseries_minute_projection_is_cancelled(cancellation) {
            stats.outcome = TimeseriesMinuteProjectionFlushOutcome::Cancelled;
            return Ok(stats);
        }
        let batch = flush_timeseries_projection_batch(
            state,
            coordinator,
            trigger,
            cancellation,
            coverage_generation,
            key_batch,
        )
        .await?;
        stats.loaded_row_count = stats
            .loaded_row_count
            .saturating_add(batch.loaded_row_count);
        stats.written_key_count += batch.written_key_count;
        stats.exact_fallback_minute_count += batch.exact_fallback_minute_count;
        stats.transaction_count += batch.transaction_count;
        if let Some(outcome) = batch.outcome {
            stats.outcome = outcome;
            return Ok(stats);
        }
    }
    if let Some(generation) = coverage_generation {
        state
            .terminal_projection_hub
            .complete_timeseries_coverage_invalidation(generation);
    }
    state
        .terminal_projection_hub
        .mark_timeseries_deltas_flushed(&event_ids);
    debug!(
        route = "timeseries_projection",
        builder = "minute_projection_v2",
        trigger,
        response_source = "memory_overlay_flush",
        event_count = stats.event_count,
        minute_rollup_count = stats.written_key_count,
        exact_fallback_minute_count = stats.exact_fallback_minute_count,
        raw_row_count = stats.loaded_row_count,
        coverage_invalidation_row_count = stats.coverage.row_count,
        coverage_invalidation_transaction_count = stats.coverage.transaction_count,
        write_transaction_count = stats.transaction_count,
        coverage_invalidation_pending = coverage_generation.is_some(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "flushed terminal deltas into minute projection"
    );
    Ok(stats)
}

enum TimeseriesPendingCoverageDecision {
    Continue(TimeseriesMinuteProjectionCoverageInvalidationStats),
    Complete(TimeseriesPendingFlushStats),
}

async fn prepare_timeseries_pending_coverage(
    state: &AppState,
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
    trigger: &'static str,
    cancellation: Option<&tokio_util::sync::CancellationToken>,
    coverage_generation: Option<u64>,
    event_count: usize,
) -> Result<TimeseriesPendingCoverageDecision, ApiError> {
    let Some(_) = coverage_generation else {
        return Ok(TimeseriesPendingCoverageDecision::Continue(
            TimeseriesMinuteProjectionCoverageInvalidationStats::default(),
        ));
    };
    let outcome = invalidate_timeseries_minute_projection_coverage(
        state,
        coordinator,
        trigger,
        event_count,
        cancellation,
    )
    .await?;
    let decision = match outcome {
        TimeseriesMinuteProjectionCoverageInvalidationOutcome::Invalidated(stats) => {
            TimeseriesPendingCoverageDecision::Continue(stats)
        }
        TimeseriesMinuteProjectionCoverageInvalidationOutcome::Deferred(deferred) => {
            TimeseriesPendingCoverageDecision::Complete(TimeseriesPendingFlushStats {
                outcome: TimeseriesMinuteProjectionFlushOutcome::Deferred(deferred),
                loaded_row_count: 0,
                written_key_count: 0,
                exact_fallback_minute_count: 0,
                transaction_count: 0,
                coverage: TimeseriesMinuteProjectionCoverageInvalidationStats::default(),
                event_count,
            })
        }
        TimeseriesMinuteProjectionCoverageInvalidationOutcome::Cancelled => {
            TimeseriesPendingCoverageDecision::Complete(TimeseriesPendingFlushStats {
                outcome: TimeseriesMinuteProjectionFlushOutcome::Cancelled,
                loaded_row_count: 0,
                written_key_count: 0,
                exact_fallback_minute_count: 0,
                transaction_count: 0,
                coverage: TimeseriesMinuteProjectionCoverageInvalidationStats::default(),
                event_count,
            })
        }
    };
    Ok(decision)
}

type TimeseriesProjectionWorkBatch = (
    TimeseriesMinuteProjectionKey,
    Vec<(i64, TimeseriesTerminalDelta)>,
);
type TimeseriesProjectionWorkBatches = Vec<Vec<TimeseriesProjectionWorkBatch>>;

fn build_timeseries_projection_work_batches(
    mut grouped: Vec<TimeseriesProjectionWorkBatch>,
) -> TimeseriesProjectionWorkBatches {
    let mut batches = Vec::new();
    let mut batch = Vec::new();
    let mut delta_count = 0;
    for (key, mut deltas) in grouped.drain(..) {
        deltas.sort_by_key(|(row_id, _)| *row_id);
        for delta_batch in deltas.chunks(TIMESERIES_MINUTE_PROJECTION_WRITE_DELTA_BATCH_LIMIT) {
            if !batch.is_empty()
                && (batch.len() >= TIMESERIES_MINUTE_PROJECTION_WRITE_KEY_BATCH_LIMIT
                    || delta_count + delta_batch.len()
                        > TIMESERIES_MINUTE_PROJECTION_WRITE_DELTA_BATCH_LIMIT)
            {
                batches.push(std::mem::take(&mut batch));
                delta_count = 0;
            }
            batch.push((key.clone(), delta_batch.to_vec()));
            delta_count += delta_batch.len();
        }
    }
    if !batch.is_empty() {
        batches.push(batch);
    }
    batches
}

struct TimeseriesProjectionBatchStats {
    outcome: Option<TimeseriesMinuteProjectionFlushOutcome>,
    loaded_row_count: u64,
    written_key_count: usize,
    exact_fallback_minute_count: usize,
    transaction_count: usize,
}

async fn flush_timeseries_projection_batch(
    state: &AppState,
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
    trigger: &'static str,
    cancellation: Option<&tokio_util::sync::CancellationToken>,
    coverage_generation: Option<u64>,
    key_batch: Vec<(
        TimeseriesMinuteProjectionKey,
        Vec<(i64, TimeseriesTerminalDelta)>,
    )>,
) -> Result<TimeseriesProjectionBatchStats, ApiError> {
    let source_snapshot_id =
        resolve_invocation_snapshot_id(&state.pool, InvocationSourceScope::All).await?;
    let mut prepared_writes = Vec::with_capacity(key_batch.len());
    let mut loaded_row_count = 0;
    let mut exact_fallback_minute_count = 0;
    for (key, deltas) in &key_batch {
        let prepared =
            prepare_timeseries_projection_key(&state.pool, key, deltas, source_snapshot_id).await?;
        loaded_row_count += prepared.loaded_row_count;
        exact_fallback_minute_count += prepared.exact_fallback as usize;
        prepared_writes.push(prepared.write);
    }
    let admission = match try_acquire_timeseries_minute_projection_write(
        coordinator,
        crate::db_pressure::global_db_pressure_gate(),
        trigger,
        "minute_projection_keys",
        key_batch.len(),
    )
    .await
    {
        TimeseriesMinuteProjectionWriteAdmissionOutcome::Acquired(admission) => admission,
        TimeseriesMinuteProjectionWriteAdmissionOutcome::Deferred(deferred) => {
            return Ok(TimeseriesProjectionBatchStats {
                outcome: Some(TimeseriesMinuteProjectionFlushOutcome::Deferred(deferred)),
                loaded_row_count,
                written_key_count: 0,
                exact_fallback_minute_count,
                transaction_count: 0,
            });
        }
    };
    if timeseries_minute_projection_is_cancelled(cancellation) {
        drop(admission);
        return Ok(TimeseriesProjectionBatchStats {
            outcome: Some(TimeseriesMinuteProjectionFlushOutcome::Cancelled),
            loaded_row_count,
            written_key_count: 0,
            exact_fallback_minute_count,
            transaction_count: 0,
        });
    }
    let transaction_started = Instant::now();
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    if timeseries_projection_batch_needs_defer(
        state,
        tx.as_mut(),
        source_snapshot_id,
        coverage_generation,
    )
    .await?
    {
        return Ok(TimeseriesProjectionBatchStats {
            outcome: Some(TimeseriesMinuteProjectionFlushOutcome::Deferred(
                TimeseriesMinuteProjectionDeferred { retry_after: None },
            )),
            loaded_row_count,
            written_key_count: 0,
            exact_fallback_minute_count,
            transaction_count: 0,
        });
    }
    for write in &prepared_writes {
        upsert_timeseries_minute_projection_v2_prepared_write_tx(tx.as_mut(), write).await?;
    }
    tx.commit().await?;
    debug!(
        route = "timeseries_projection",
        builder = "minute_projection_v2",
        trigger,
        transaction_phase = "minute_projection_keys",
        transaction_key_count = key_batch.len(),
        transaction_delta_count = key_batch
            .iter()
            .map(|(_, deltas)| deltas.len())
            .sum::<usize>(),
        elapsed_ms = transaction_started.elapsed().as_millis() as u64,
        "flushed a bounded minute projection key slice"
    );
    drop(admission);
    Ok(TimeseriesProjectionBatchStats {
        outcome: None,
        loaded_row_count,
        written_key_count: prepared_writes.len(),
        exact_fallback_minute_count,
        transaction_count: 1,
    })
}

async fn timeseries_projection_batch_needs_defer(
    state: &AppState,
    tx: &mut SqliteConnection,
    source_snapshot_id: i64,
    coverage_generation: Option<u64>,
) -> Result<bool, ApiError> {
    let durable_recovery_pending = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM timeseries_minute_projection_v2_recovery WHERE consumer = ?1 AND invalidation_pending = 1)",
    )
    .bind(TIMESERIES_MINUTE_PROJECTION_RECOVERY_CONSUMER)
    .fetch_one(&mut *tx)
    .await?
        != 0;
    let new_coverage = state
        .terminal_projection_hub
        .timeseries_coverage_invalidation_pending();
    Ok(durable_recovery_pending
        || resolve_invocation_snapshot_id_tx(&mut *tx, InvocationSourceScope::All).await?
            != source_snapshot_id
        || new_coverage != coverage_generation)
}

struct PreparedTimeseriesProjectionKey {
    write: TimeseriesMinuteProjectionPreparedWrite,
    loaded_row_count: u64,
    exact_fallback: bool,
}

async fn prepare_timeseries_projection_key(
    pool: &Pool<Sqlite>,
    key: &TimeseriesMinuteProjectionKey,
    deltas: &[(i64, TimeseriesTerminalDelta)],
    source_snapshot_id: i64,
) -> Result<PreparedTimeseriesProjectionKey, ApiError> {
    let Some((aggregate, existing_max_row_id)) =
        load_timeseries_minute_projection_v2_key(pool, key).await?
    else {
        let (aggregate, max_row_id, source_row_count) =
            rebuild_timeseries_minute_projection_v2_key(pool, key, Some(source_snapshot_id))
                .await?;
        return Ok(PreparedTimeseriesProjectionKey {
            write: prepare_timeseries_minute_projection_v2_write(key, &aggregate, max_row_id)?,
            loaded_row_count: source_row_count,
            exact_fallback: true,
        });
    };
    if timeseries_projection_requires_exact_rebuild(deltas, existing_max_row_id) {
        let (aggregate, max_row_id, source_row_count) =
            rebuild_timeseries_minute_projection_v2_key(pool, key, Some(source_snapshot_id))
                .await?;
        return Ok(PreparedTimeseriesProjectionKey {
            write: prepare_timeseries_minute_projection_v2_write(key, &aggregate, max_row_id)?,
            loaded_row_count: source_row_count,
            exact_fallback: true,
        });
    }
    let mut aggregate = aggregate;
    let mut max_row_id = existing_max_row_id;
    for (row_id, delta) in deltas {
        add_timeseries_terminal_delta_to_aggregate(&mut aggregate, delta);
        max_row_id = max_row_id.max(*row_id);
    }
    Ok(PreparedTimeseriesProjectionKey {
        write: prepare_timeseries_minute_projection_v2_write(key, &aggregate, max_row_id)?,
        loaded_row_count: 0,
        exact_fallback: false,
    })
}
pub(crate) fn spawn_timeseries_minute_projection_supervisor(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if !recover_timeseries_minute_projection_at_startup(state.as_ref(), &cancel).await {
            return;
        }
        state
            .terminal_projection_hub
            .activate_timeseries_consumer(0);
        let pressure_gate = crate::db_pressure::global_db_pressure_gate();
        let mut ticker = interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        ticker.tick().await;
        let mut deferred_eligibility_generation = None;
        let mut deferred_retry_after = None;
        loop {
            if let Some(observed_eligibility_generation) = deferred_eligibility_generation.take() {
                if let Some(retry_after) = deferred_retry_after.take() {
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        _ = ticker.tick() => {}
                        _ = pressure_gate.wait_for_eligibility_change(observed_eligibility_generation) => {}
                        _ = tokio::time::sleep(retry_after) => {}
                    }
                } else {
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        _ = ticker.tick() => {}
                        _ = pressure_gate.wait_for_eligibility_change(observed_eligibility_generation) => {}
                    }
                }
            } else {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = ticker.tick() => {}
                }
            }

            match flush_timeseries_minute_projection_with_coordinator_and_cancellation(
                state.as_ref(),
                "terminal_deadline",
                &crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator(),
                Some(&cancel),
            )
            .await
            {
                Ok(TimeseriesMinuteProjectionFlushOutcome::Flushed) => {}
                Ok(TimeseriesMinuteProjectionFlushOutcome::Deferred(deferred)) => {
                    let observed_eligibility_generation = pressure_gate.eligibility_generation();
                    deferred_eligibility_generation = Some(observed_eligibility_generation);
                    deferred_retry_after = deferred.retry_after;
                    debug!(
                        route = "timeseries_projection",
                        builder = "minute_projection_v2",
                        trigger = "terminal_deadline",
                        flush_outcome = "deferred",
                        observed_eligibility_generation,
                        retry_after_ms = deferred.retry_after.map(|value| value.as_millis() as u64),
                        "minute projection flush yielded to higher-priority database work"
                    );
                }
                Ok(TimeseriesMinuteProjectionFlushOutcome::Cancelled) => return,
                Err(error) => {
                    warn!(
                        route = "timeseries_projection",
                        builder = "minute_projection_v2",
                        trigger = "terminal_deadline",
                        ?error,
                        "minute projection flush failed"
                    );
                }
            }
        }
    })
}

async fn recover_timeseries_minute_projection_at_startup(
    state: &AppState,
    cancel: &CancellationToken,
) -> bool {
    let pressure_gate = crate::db_pressure::global_db_pressure_gate();
    loop {
        match prepare_timeseries_minute_projection_after_restart(state, cancel).await {
            Ok(TimeseriesMinuteProjectionFlushOutcome::Flushed) => return true,
            Ok(TimeseriesMinuteProjectionFlushOutcome::Deferred(deferred)) => {
                let observed_eligibility_generation = pressure_gate.eligibility_generation();
                debug!(
                    route = "timeseries_projection",
                    builder = "minute_projection_v2",
                    trigger = "startup_recovery",
                    observed_eligibility_generation,
                    "startup minute projection recovery yielded to higher-priority database work"
                );
                if let Some(retry_after) = deferred.retry_after {
                    tokio::select! {
                        _ = cancel.cancelled() => return false,
                        _ = pressure_gate.wait_for_eligibility_change(observed_eligibility_generation) => {}
                        _ = tokio::time::sleep(retry_after) => {}
                    }
                } else {
                    tokio::select! {
                        _ = cancel.cancelled() => return false,
                        _ = pressure_gate.wait_for_eligibility_change(observed_eligibility_generation) => {}
                        _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                    }
                }
            }
            Ok(TimeseriesMinuteProjectionFlushOutcome::Cancelled) => return false,
            Err(error) => {
                let pressure_deferred = timeseries_minute_projection_pressure_deferred(
                    pressure_gate,
                    "timeseries_minute_projection_startup",
                    &error,
                );
                let observed_eligibility_generation = pressure_gate.eligibility_generation();
                warn!(
                    route = "timeseries_projection",
                    builder = "minute_projection_v2",
                    pressure_deferred = pressure_deferred.is_some(),
                    ?error,
                    "failed to invalidate minute projection during startup recovery"
                );
                if let Some(deferred) = pressure_deferred {
                    tokio::select! {
                        _ = cancel.cancelled() => return false,
                        _ = pressure_gate.wait_for_eligibility_change(observed_eligibility_generation) => {}
                        _ = tokio::time::sleep(deferred.retry_after.expect("pressure deferral has a retry deadline")) => {}
                    }
                } else {
                    tokio::select! {
                        _ = cancel.cancelled() => return false,
                        _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                    }
                }
            }
        }
    }
}
