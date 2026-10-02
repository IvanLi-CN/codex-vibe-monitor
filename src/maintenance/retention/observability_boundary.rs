use super::*;

pub(crate) async fn run_data_retention_maintenance_best_effort(
    state: &Arc<AppState>,
    cancel: &CancellationToken,
    trigger: &'static str,
) -> bool {
    crate::observability::observed_future(
        state.observability.enabled,
        "retention_maintenance",
        run_data_retention_maintenance_best_effort_impl(state, cancel, trigger),
    )
    .await
}

async fn run_data_retention_maintenance_best_effort_impl(
    state: &Arc<AppState>,
    cancel: &CancellationToken,
    trigger: &'static str,
) -> bool {
    if crate::maintenance_store::legacy_worker_should_skip("retention_archive").await {
        debug!(
            trigger,
            "retention legacy worker skipped by managed task control"
        );
        return true;
    }
    let Some(_execution_lease) =
        crate::maintenance_store::try_acquire_task_execution("retention_archive")
    else {
        return true;
    };
    let observation = crate::TaskExecutionObservation::begin(
        "retention_archive",
        &crate::maintenance_store::task_title_for_observation("retention_archive"),
        trigger,
        crate::maintenance_store::task_execution_class("retention_archive"),
        "processing",
    );
    let started_at = Instant::now();
    match run_data_retention_maintenance_with_circuit_and_prompt_cache(
        &state.pool,
        &state.config,
        None,
        Some(cancel),
        state.raw_capture_circuit.clone(),
        Some(&state.prompt_cache_conversation_cache),
    )
    .await
    {
        Ok(summary) => {
            state.observability.record_duration_ms(
                "maintenance.run_duration_ms",
                "maintenance",
                started_at.elapsed().as_secs_f64() * 1000.0,
            );
            state.observability.record_counter(
                "maintenance.processed_rows",
                "maintenance",
                summary.processed_row_count(),
            );
            state.observability.record_gauge(
                "maintenance.raw_bytes_before",
                "maintenance",
                summary.raw_bytes_before as f64,
            );
            state.observability.record_gauge(
                "maintenance.raw_bytes_after",
                "maintenance",
                summary.raw_bytes_after as f64,
            );
            state.observability.record_counter(
                "maintenance.compressed_file_count",
                "maintenance",
                summary.raw_files_compressed as u64,
            );
            state.observability.record_counter(
                "maintenance.removed_file_count",
                "maintenance",
                (summary.raw_files_removed + summary.orphan_raw_files_removed) as u64,
            );
            state.observability.record_counter(
                "maintenance.archived_rows",
                "maintenance",
                (summary.invocation_rows_archived
                    + summary.forward_proxy_attempt_rows_archived
                    + summary.pool_upstream_request_attempt_rows_archived
                    + summary.quota_snapshot_rows_archived) as u64,
            );
            if summary.deferred {
                debug!(
                    trigger,
                    "retention maintenance deferred; preserving the prompt retry schedule"
                );
                invalidate_system_status_cache(state.as_ref()).await;
                return false;
            }
            // Commit the bounded inventory reset before task bookkeeping or cancellation can
            // return. Raw path mutations must never leave the monotonic inventory stale.
            let reset_pending = match crate::system_raw_payload_metrics_inventory_reset_pending(
                &state.pool,
            )
            .await
            {
                Ok(pending) => pending,
                Err(error) => {
                    warn!(
                        trigger,
                        error = %error,
                        "failed to inspect system raw metrics inventory reset state"
                    );
                    invalidate_system_status_cache(state.as_ref()).await;
                    false
                }
            };
            if summary.raw_files_compressed > 0
                || summary.raw_files_removed > 0
                || summary.orphan_raw_files_removed > 0
                || reset_pending
            {
                match reset_retention_raw_payload_metrics_inventory(state.as_ref()).await {
                    Ok(true) => {}
                    Ok(false) => {
                        debug!(
                            trigger,
                            "system raw metrics inventory reset deferred; preserving retry schedule"
                        );
                    }
                    Err(error) => {
                        warn!(
                            trigger,
                            error = %error,
                            "failed to reset system raw metrics inventory"
                        );
                    }
                }
            }
            let touched_anything = summary.touched_anything();
            if touched_anything && !summary.dry_run {
                observation.finish();
                let task_run = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return false,
                    result = begin_system_task_run_admitted(
                        state.as_ref(),
                        crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::MaintenanceRetention,
                        SystemTaskKind::RetentionArchive,
                        trigger,
                        Some("retention maintenance completed a write pass".to_string()),
                    ) => result.ok(),
                };
                if let Some(handle) = task_run.as_ref() {
                    let (brief, detail) = summarize_retention_run_for_system_task(&summary);
                    let _ = finish_system_task_run_reliably(
                        state.as_ref(),
                        Some(cancel),
                        handle,
                        SystemTaskStatus::Success,
                        Some(brief),
                        Some(detail),
                    )
                    .await;
                }
            }
            invalidate_system_status_cache(state.as_ref()).await;
            touched_anything
        }
        Err(err) => {
            state.observability.record_duration_ms(
                "maintenance.run_duration_ms",
                "maintenance",
                started_at.elapsed().as_secs_f64() * 1000.0,
            );
            let pressure_error = crate::db_pressure::global_db_pressure_gate()
                .record_error("data_retention_maintenance", &err);
            retention_record_error("data_retention_maintenance", &err);
            if !state.config.retention_dry_run {
                observation.finish();
                let task_run = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return false,
                    result = begin_system_task_run_admitted(
                        state.as_ref(),
                        crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::MaintenanceRetention,
                        SystemTaskKind::RetentionArchive,
                        trigger,
                        Some("retention maintenance failed".to_string()),
                    ) => result.ok(),
                };
                if let Some(handle) = task_run.as_ref() {
                    let _ = finish_system_task_run_reliably(
                        state.as_ref(),
                        Some(cancel),
                        handle,
                        SystemTaskStatus::Failed,
                        Some("retention maintenance failed".to_string()),
                        Some(format!(
                            "failure_fingerprint:{}",
                            retention_error_fingerprint(&err)
                        )),
                    )
                    .await;
                }
            }
            warn!(
                trigger,
                error_fingerprint = %retention_error_fingerprint(&err),
                retry_soon = pressure_error,
                "failed to run retention maintenance"
            );
            !pressure_error
        }
    }
}
