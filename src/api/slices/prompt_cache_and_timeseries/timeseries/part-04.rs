pub(crate) async fn prepare_timeseries_minute_projection_after_restart(
    state: &AppState,
    cancellation: &CancellationToken,
) -> Result<TimeseriesMinuteProjectionFlushOutcome, ApiError> {
    if cancellation.is_cancelled() {
        return Ok(TimeseriesMinuteProjectionFlushOutcome::Cancelled);
    }
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    loop {
        let Some(recovery_generation) =
            timeseries_minute_projection_recovery_generation(&state.pool).await?
        else {
            return Ok(TimeseriesMinuteProjectionFlushOutcome::Flushed);
        };
        if let Some(outcome) = prepare_timeseries_restart_generation(
            state,
            cancellation,
            &coordinator,
            recovery_generation,
        )
        .await?
        {
            return Ok(outcome);
        }
        tokio::task::yield_now().await;
    }
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

async fn prepare_timeseries_restart_generation(
    state: &AppState,
    cancellation: &CancellationToken,
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
    recovery_generation: i64,
) -> Result<Option<TimeseriesMinuteProjectionFlushOutcome>, ApiError> {
    let admission = match try_acquire_timeseries_minute_projection_write(
        coordinator,
        crate::db_pressure::global_db_pressure_gate(),
        "startup_recovery",
        "startup_state_warming",
        0,
    )
    .await
    {
        TimeseriesMinuteProjectionWriteAdmissionOutcome::Acquired(admission) => admission,
        TimeseriesMinuteProjectionWriteAdmissionOutcome::Deferred(deferred) => {
            return Ok(Some(TimeseriesMinuteProjectionFlushOutcome::Deferred(
                deferred,
            )));
        }
    };
    if cancellation.is_cancelled() {
        drop(admission);
        return Ok(Some(TimeseriesMinuteProjectionFlushOutcome::Cancelled));
    }
    mark_timeseries_restart_warming(state).await?;
    drop(admission);

    let coverage = match invalidate_timeseries_minute_projection_coverage(
        state,
        coordinator,
        "startup_recovery",
        0,
        Some(cancellation),
    )
    .await?
    {
        TimeseriesMinuteProjectionCoverageInvalidationOutcome::Invalidated(stats) => stats,
        TimeseriesMinuteProjectionCoverageInvalidationOutcome::Deferred(deferred) => {
            return Ok(Some(TimeseriesMinuteProjectionFlushOutcome::Deferred(
                deferred,
            )));
        }
        TimeseriesMinuteProjectionCoverageInvalidationOutcome::Cancelled => {
            return Ok(Some(TimeseriesMinuteProjectionFlushOutcome::Cancelled));
        }
    };
    let admission = match try_acquire_timeseries_minute_projection_write(
        coordinator,
        crate::db_pressure::global_db_pressure_gate(),
        "startup_recovery",
        "startup_state_ready",
        0,
    )
    .await
    {
        TimeseriesMinuteProjectionWriteAdmissionOutcome::Acquired(admission) => admission,
        TimeseriesMinuteProjectionWriteAdmissionOutcome::Deferred(deferred) => {
            return Ok(Some(TimeseriesMinuteProjectionFlushOutcome::Deferred(
                deferred,
            )));
        }
    };
    if cancellation.is_cancelled() {
        drop(admission);
        return Ok(Some(TimeseriesMinuteProjectionFlushOutcome::Cancelled));
    }
    let recovery_cleared = clear_timeseries_restart_generation(state, recovery_generation).await?;
    drop(admission);
    if recovery_cleared {
        debug!(
            route = "timeseries_projection",
            builder = "minute_projection_v2",
            response_source = "startup_invalidation",
            recovery_generation,
            coverage_invalidation_row_count = coverage.row_count,
            coverage_invalidation_transaction_count = coverage.transaction_count,
            "invalidated stale minute coverage before accepting projection reads"
        );
        return Ok(Some(TimeseriesMinuteProjectionFlushOutcome::Flushed));
    }
    Ok(None)
}

async fn mark_timeseries_restart_warming(state: &AppState) -> Result<(), ApiError> {
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO timeseries_minute_projection_v2_state (consumer, cursor_row_id, last_flush_at, last_error, updated_at) VALUES (?1, 0, NULL, 'warming', datetime('now')) ON CONFLICT(consumer) DO UPDATE SET last_error = 'warming', updated_at = excluded.updated_at",
    )
    .bind(TIMESERIES_MINUTE_PROJECTION_RECOVERY_CONSUMER)
    .execute(tx.as_mut())
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn clear_timeseries_restart_generation(
    state: &AppState,
    recovery_generation: i64,
) -> Result<bool, ApiError> {
    let mut tx = state.pool.begin().await?;
    let recovery_cleared = sqlx::query(
        "UPDATE timeseries_minute_projection_v2_recovery SET invalidation_pending = 0, updated_at = datetime('now') WHERE consumer = ?1 AND generation = ?2 AND invalidation_pending = 1",
    )
    .bind(TIMESERIES_MINUTE_PROJECTION_RECOVERY_CONSUMER)
    .bind(recovery_generation)
    .execute(tx.as_mut())
    .await?
    .rows_affected()
        != 0;
    if recovery_cleared {
        sqlx::query(
            "UPDATE timeseries_minute_projection_v2_state SET last_error = 'ready', updated_at = datetime('now') WHERE consumer = ?1",
        )
        .bind(TIMESERIES_MINUTE_PROJECTION_RECOVERY_CONSUMER)
        .execute(tx.as_mut())
        .await?;
    }
    tx.commit().await?;
    Ok(recovery_cleared)
}
