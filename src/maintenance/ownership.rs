use super::*;
use serde::{Deserialize, Serialize};
use std::future::Future;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MaintenanceExecutionOptions {
    pub(crate) manual: bool,
    pub(crate) admitted: bool,
    pub(crate) dry_run: bool,
}

tokio::task_local! {
    static EXECUTION_OPTIONS: MaintenanceExecutionOptions;
    static IDENTITY_CLEANUP_DEADLINE: Instant;
}

pub(crate) fn maintenance_execution_options() -> MaintenanceExecutionOptions {
    EXECUTION_OPTIONS
        .try_with(|options| *options)
        .unwrap_or(MaintenanceExecutionOptions {
            manual: false,
            admitted: false,
            dry_run: false,
        })
}

pub(crate) fn managed_raw_traversal() -> &'static Mutex<RetentionRawDirectoryTraversal> {
    static TRAVERSAL: std::sync::OnceLock<Mutex<RetentionRawDirectoryTraversal>> =
        std::sync::OnceLock::new();
    TRAVERSAL.get_or_init(|| Mutex::new(RetentionRawDirectoryTraversal::default()))
}

pub(crate) fn maintenance_execution_is_admitted() -> bool {
    EXECUTION_OPTIONS
        .try_with(|options| options.admitted)
        .unwrap_or(false)
}

pub(crate) fn maintenance_execution_is_manual() -> bool {
    EXECUTION_OPTIONS
        .try_with(|options| options.manual)
        .unwrap_or(false)
}

pub(crate) async fn with_maintenance_execution_options<F: Future>(
    options: MaintenanceExecutionOptions,
    future: F,
) -> F::Output {
    EXECUTION_OPTIONS.scope(options, future).await
}

pub(crate) async fn identity_cleanup_query<T, E, F>(future: F) -> Result<T>
where
    E: Into<anyhow::Error>,
    F: Future<Output = std::result::Result<T, E>>,
{
    let remaining = IDENTITY_CLEANUP_DEADLINE
        .try_with(|deadline| deadline.saturating_duration_since(Instant::now()))
        .ok()
        .or_else(crate::maintenance::retention_run_remaining_budget);
    if let Some(remaining) = remaining {
        tokio::time::timeout(remaining, future)
            .await
            .map_err(|_| anyhow!("invocation identity cleanup work budget expired"))?
            .map_err(Into::into)
    } else {
        future.await.map_err(Into::into)
    }
}

pub(crate) async fn with_identity_cleanup_work_budget<F: Future>(
    budget: Duration,
    future: F,
) -> F::Output {
    IDENTITY_CLEANUP_DEADLINE
        .scope(Instant::now() + budget, future)
        .await
}

pub(crate) struct OwnedMaintenanceExecution {
    pub(crate) summary: String,
    pub(crate) completion: &'static str,
    pub(crate) details: Value,
    pub(crate) next_work_secs: Option<i64>,
}

pub(crate) struct OwnedMaintenanceContext<'a> {
    pub(crate) pool: &'a Pool<Sqlite>,
    pub(crate) config: &'a AppConfig,
    pub(crate) cache: &'a Arc<Mutex<PromptCacheConversationsCacheState>>,
    pub(crate) circuit: Arc<RawCaptureCircuitBreaker>,
    pub(crate) shutdown: &'a CancellationToken,
    pub(crate) raw_traversal: &'a Mutex<RetentionRawDirectoryTraversal>,
}

pub(crate) async fn execute_owned_maintenance(
    context: OwnedMaintenanceContext<'_>,
    task_key: &str,
    options: MaintenanceExecutionOptions,
    observation: Option<crate::TaskExecutionObservation>,
) -> Result<OwnedMaintenanceExecution> {
    let started = Instant::now();
    let run_observation = observation.clone();
    let mut result = match task_key {
        "retention_archive" => {
            if options.dry_run
                && let Some(observation) = &observation
            {
                observation.mark_work_started();
            }
            // Keep the archive state machine off the cleanup/dispatcher futures' inline frames.
            let summary = Box::pin(
                run_data_retention_maintenance_with_circuit_and_prompt_cache(
                    context.pool,
                    context.config,
                    Some(options.dry_run),
                    Some(context.shutdown),
                    context.circuit,
                    None,
                    observation,
                ),
            )
            .await?;
            let (brief, detail) = crate::api::summarize_retention_run_for_system_task(&summary);
            OwnedMaintenanceExecution {
                summary: brief,
                completion: summary.completion(),
                next_work_secs: None,
                details: json!({"completion": summary.completion(), "coreCompletion": summary.core_completion(),
                    "budgetMs": summary.work_budget_ms, "elapsedMs": summary.elapsed_ms, "settlementMs": summary.settlement_ms,
                    "budgetExhausted": summary.budget_exhausted, "recoverableFailure": summary.recoverable_failure,
                    "waitReason": summary.wait_reason, "processedCount": summary.processed_row_count(),
                    "total": summary.backlog_total, "completed": summary.invocation_rows_archived,
                    "backlogRemaining": summary.backlog_total.map(|total| total.saturating_sub(summary.invocation_rows_archived as i64)),
                    "observedAt": summary.backlog_observed_at, "sourceMaxInvocationId": summary.source_max_invocation_id,
                    "invocationRowsArchived": summary.invocation_rows_archived, "invocationDetailsPruned": summary.invocation_details_pruned,
                    "archiveBatchesTouched": summary.archive_batches_touched, "archiveBatches": summary.batches,
                    "timeoutCount": summary.timeout_count, "rawFilesRemoved": summary.raw_files_removed, "summary": detail,
                    "fatalError": summary.fatal_error}),
            }
        }
        "invocation_identity_cleanup" => {
            let admission = tokio::select! {
                biased;
                _ = context.shutdown.cancelled() => None,
                admission = tokio::time::timeout(Duration::from_secs(15), super::retention::acquire_retention_write_admission("invocation_identity_cleanup")) => admission.ok().flatten(),
            };
            let Some(_admission) = admission else {
                return Ok(OwnedMaintenanceExecution {
                    summary: "调用身份清理等待资源准入".to_string(),
                    completion: "deferred",
                    next_work_secs: Some(300),
                    details: json!({"ownershipVersion": 1,"ownerScope": task_key,"dryRun": options.dry_run,"manual": options.manual,
                        "waitReason": "sqlite_pressure", "coverage": "unknown", "overallRemaining": null,
                        "resourceWaitMs": run_observation.as_ref().map(|value| value.resource_wait_ms()), "actualStartedAt": null}),
                });
            };
            let cleanup = with_identity_cleanup_work_budget(
                Duration::from_secs(2),
                cleanup_invocation_identities(context.pool, options.dry_run, context.cache),
            )
            .await;
            let cleanup = match cleanup {
                Ok(cleanup) => cleanup,
                Err(error) if error.to_string().contains("work budget") => {
                    return Ok(OwnedMaintenanceExecution {
                        summary: "调用身份清理达到本轮工作预算，等待下一轮继续".to_string(),
                        completion: "partial",
                        next_work_secs: Some(1),
                        details: json!({"ownershipVersion": 1, "ownerScope": task_key, "dryRun": options.dry_run,
                            "manual": options.manual, "budgetMs": 2000, "budgetExhausted": true,
                            "conversationIdentitiesChecked": null, "conversationIdentitiesReleased": null,
                            "hourPrefixesChecked": null, "hourPrefixesReleased": null, "coverage": "unknown",
                            "overallRemaining": null, "hasMore": true, "waitReason": "identity_work_budget",
                            "resourceWaitMs": run_observation.as_ref().map(|value| value.resource_wait_ms()),
                            "actualStartedAt": crate::task_runtime_observation::workload_sample(task_key).and_then(|sample| sample.actual_started_at)}),
                    });
                }
                Err(error) => return Err(error),
            };
            if let Some(observation) = observation {
                observation.set_discovered_work(
                    cleanup.conversations_checked as i64,
                    format_utc_iso_millis(Utc::now()),
                    "run-window".to_string(),
                );
                observation.set_processed_work(cleanup.conversations_released as i64);
            }
            OwnedMaintenanceExecution {
                summary: format!(
                    "检查对话身份 {} 个、小时前缀 {} 个；{} {} / {} 个",
                    cleanup.conversations_checked,
                    cleanup
                        .hours_checked
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "未知".to_string()),
                    if options.dry_run {
                        "可释放"
                    } else {
                        "已释放"
                    },
                    cleanup.conversations_released,
                    cleanup
                        .hours_released
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "未知".to_string())
                ),
                completion: if cleanup.has_more {
                    "partial"
                } else {
                    "completed"
                },
                next_work_secs: cleanup.has_more.then_some(1),
                details: json!({"conversationIdentitiesChecked": cleanup.conversations_checked, "conversationIdentitiesReleased": cleanup.conversations_released,
                    "hourPrefixesChecked": cleanup.hours_checked, "hourPrefixesReleased": cleanup.hours_released,
                    "coverage": "bounded_scan", "overallRemaining": null, "hasMore": cleanup.has_more, "budgetMs": 2000, "budgetExhausted": cleanup.budget_exhausted,
                    "waitReason": cleanup.budget_exhausted.then_some("identity_work_budget")}),
            }
        }
        "raw_orphan_sweep" => {
            let mut local_traversal = RetentionRawDirectoryTraversal::default();
            let mut traversal = context.raw_traversal.lock().await;
            // Previews must not consume the executor's formal traversal position.
            let selected = if options.dry_run {
                &mut local_traversal
            } else {
                &mut *traversal
            };
            let (pass, retry) = run_managed_raw_orphan_sweep(
                context.pool,
                context.config,
                context.shutdown,
                context.circuit,
                options,
                selected,
            )
            .await?;
            let measured = !pass.deferred
                || pass.file_candidates_checked > 0
                || pass.inspected_entries > 0
                || pass.reconciliation_rows_checked > 0;
            if measured && let Some(observation) = observation {
                observation.set_discovered_work(
                    pass.file_candidates_checked as i64,
                    format_utc_iso_millis(Utc::now()),
                    "run-window".to_string(),
                );
                observation.set_processed_work(pass.removed as i64);
            }
            let more = !pass.complete || !pass.reached_end || pass.reconciliation_has_more;
            OwnedMaintenanceExecution {
                summary: if measured {
                    format!(
                        "检查文件 {} 个，{} {} 个 / {} 字节",
                        pass.file_candidates_checked,
                        if options.dry_run {
                            "可释放"
                        } else {
                            "已释放"
                        },
                        pass.removed,
                        pass.removed_bytes
                    )
                } else {
                    "Raw 孤儿文件清理等待资源准入，检查范围未知".to_string()
                },
                completion: if pass.deferred {
                    "deferred"
                } else if more || pass.failures > 0 {
                    "partial"
                } else {
                    "completed"
                },
                next_work_secs: (more || pass.deferred || pass.failures > 0).then_some(retry),
                details: json!({"filesChecked": measured.then_some(pass.file_candidates_checked), "directoryEntriesChecked": measured.then_some(pass.inspected_entries), "reconciliationRowsChecked": measured.then_some(pass.reconciliation_rows_checked),
                    "filesReleased": measured.then_some(pass.removed), "bytesReleased": measured.then_some(pass.removed_bytes), "referencedSkipped": measured.then_some(pass.referenced_skipped),
                    "quarantined": measured.then_some(pass.quarantined), "failures": measured.then_some(pass.failures), "hasMore": more, "coverage": if measured { "bounded_scan" } else { "unknown" }, "overallRemaining": null,
                    "waitReason": pass.admission_cause, "admissionStage": pass.admission_stage, "retryAfterSecs": retry, "budgetMs": 2000}),
            }
        }
        _ => bail!("unsupported owned maintenance task: {task_key}"),
    };
    result.details["ownershipVersion"] = json!(1);
    result.details["ownerScope"] = json!(task_key);
    result.details["dryRun"] = json!(options.dry_run);
    result.details["manual"] = json!(options.manual);
    result.details["runElapsedMs"] = json!(started.elapsed().as_millis() as u64);
    result.details["resourceWaitMs"] = json!(
        run_observation
            .as_ref()
            .map(|value| value.resource_wait_ms())
    );
    result.details["actualStartedAt"] = json!(
        run_observation
            .as_ref()
            .and_then(|_| crate::task_runtime_observation::workload_sample(task_key))
            .and_then(|sample| sample.actual_started_at)
    );
    Ok(result)
}

pub(crate) async fn initialize_invocation_identity_cleanup_state(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO prompt_cache_conversation_orphan_cleanup_state(scope) VALUES('invocation_hour_prefixes')")
        .execute(pool).await?;
    Ok(())
}
