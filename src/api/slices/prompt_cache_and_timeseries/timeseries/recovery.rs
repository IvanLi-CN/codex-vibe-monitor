use super::*;

pub(crate) async fn prepare_timeseries_minute_projection_after_restart(
    state: &AppState,
    cancellation: &CancellationToken,
) -> Result<TimeseriesMinuteProjectionFlushOutcome, ApiError> {
    prepare_timeseries_minute_projection_after_restart_for_managed_run(state, cancellation, None)
        .await
}

pub(crate) async fn prepare_timeseries_minute_projection_after_restart_for_managed_run(
    state: &AppState,
    cancellation: &CancellationToken,
    managed_run_id: Option<i64>,
) -> Result<TimeseriesMinuteProjectionFlushOutcome, ApiError> {
    if cancellation.is_cancelled() {
        return Ok(TimeseriesMinuteProjectionFlushOutcome::Cancelled);
    }
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let mut observation = None;
    let result = async {
        loop {
        let Some(recovery_generation) =
            timeseries_minute_projection_recovery_generation(&state.pool).await?
        else {
            return Ok(TimeseriesMinuteProjectionFlushOutcome::Flushed);
        };
        let admission = match try_acquire_timeseries_minute_projection_write(
            &coordinator,
            crate::db_pressure::global_db_pressure_gate(),
            "startup_recovery",
            "startup_state_warming",
            0,
        )
        .await
        {
            TimeseriesMinuteProjectionWriteAdmissionOutcome::Acquired(admission) => admission,
            TimeseriesMinuteProjectionWriteAdmissionOutcome::Deferred(deferred) => {
                return Ok(TimeseriesMinuteProjectionFlushOutcome::Deferred(deferred));
            }
        };
        if cancellation.is_cancelled() {
            drop(admission);
            return Ok(TimeseriesMinuteProjectionFlushOutcome::Cancelled);
        }
        observation.get_or_insert_with(|| {
            begin_timeseries_minute_projection_observation("startup_recovery", managed_run_id)
        });
        let mut tx = state.pool.begin().await?;
        sqlx::query(
            "INSERT INTO timeseries_minute_projection_v2_state (consumer, cursor_row_id, last_flush_at, last_error, updated_at) VALUES (?1, 0, NULL, 'warming', datetime('now')) ON CONFLICT(consumer) DO UPDATE SET last_error = 'warming', updated_at = excluded.updated_at",
        )
        .bind(TIMESERIES_MINUTE_PROJECTION_RECOVERY_CONSUMER)
        .execute(tx.as_mut())
        .await?;
        tx.commit().await?;
        drop(admission);

        let coverage = match invalidate_timeseries_minute_projection_coverage(
            state,
            &coordinator,
            "startup_recovery",
            0,
            Some(cancellation),
            managed_run_id,
            &mut observation,
        )
        .await?
        {
            TimeseriesMinuteProjectionCoverageInvalidationOutcome::Invalidated(stats) => stats,
            TimeseriesMinuteProjectionCoverageInvalidationOutcome::Deferred(deferred) => {
                return Ok(TimeseriesMinuteProjectionFlushOutcome::Deferred(deferred));
            }
            TimeseriesMinuteProjectionCoverageInvalidationOutcome::Cancelled => {
                return Ok(TimeseriesMinuteProjectionFlushOutcome::Cancelled);
            }
        };

        let admission = match try_acquire_timeseries_minute_projection_write(
            &coordinator,
            crate::db_pressure::global_db_pressure_gate(),
            "startup_recovery",
            "startup_state_ready",
            0,
        )
        .await
        {
            TimeseriesMinuteProjectionWriteAdmissionOutcome::Acquired(admission) => admission,
            TimeseriesMinuteProjectionWriteAdmissionOutcome::Deferred(deferred) => {
                return Ok(TimeseriesMinuteProjectionFlushOutcome::Deferred(deferred));
            }
        };
        if cancellation.is_cancelled() {
            drop(admission);
            return Ok(TimeseriesMinuteProjectionFlushOutcome::Cancelled);
        }
        observation.get_or_insert_with(|| {
            begin_timeseries_minute_projection_observation("startup_recovery", managed_run_id)
        });
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
            return Ok(TimeseriesMinuteProjectionFlushOutcome::Flushed);
        }
        tokio::task::yield_now().await;
        }
    }
    .await;
    if managed_run_id.is_none()
        && let Some(observation) = observation
    {
        let status = match &result {
            Ok(TimeseriesMinuteProjectionFlushOutcome::Flushed) => "success",
            Ok(TimeseriesMinuteProjectionFlushOutcome::Deferred(_)) => "skipped",
            Ok(TimeseriesMinuteProjectionFlushOutcome::Cancelled) => "interrupted",
            Err(_) => "failed",
        };
        observation.finish_with_status(status);
    }
    result
}
