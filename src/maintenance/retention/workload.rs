use super::*;

tokio::task_local! {
    static RETENTION_WORKLOAD_OBSERVATION: RefCell<Option<crate::TaskExecutionObservation>>;
}

pub(crate) async fn run_data_retention_maintenance_with_circuit_and_prompt_cache(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    dry_run: Option<bool>,
    shutdown: Option<&CancellationToken>,
    circuit: Arc<RawCaptureCircuitBreaker>,
    prompt_cache_conversation_cache: Option<&Arc<Mutex<PromptCacheConversationsCacheState>>>,
    workload_observation: Option<crate::TaskExecutionObservation>,
) -> Result<RetentionRunSummary> {
    RETENTION_RAW_CAPTURE_CIRCUIT
        .scope(
            RefCell::new(Some(circuit)),
            RETENTION_WORKLOAD_OBSERVATION.scope(
                RefCell::new(workload_observation),
                run_data_retention_maintenance_with_prompt_cache(
                    pool,
                    config,
                    dry_run,
                    shutdown,
                    prompt_cache_conversation_cache,
                ),
            ),
        )
        .await
}

pub(super) fn record_pending_population(
    config: &AppConfig,
    summary: &RetentionRunSummary,
    source_snapshot_available: bool,
) {
    let pending_observed_at = format_utc_iso_millis(Utc::now());
    let pending_range = "complete eligible invocation snapshot at run start".to_string();
    let pending_coverage = if source_snapshot_available && summary.backlog_total.is_some() {
        "exact"
    } else {
        "unknown"
    };
    let _ = RETENTION_WORKLOAD_OBSERVATION.try_with(|observation| {
        if let Some(observation) = observation.borrow().as_ref() {
            observation.set_pending_population(
                summary.backlog_total,
                pending_observed_at,
                format!(
                    "expired_invocations:retention_policy:{}days",
                    config.invocation_max_days
                ),
                pending_range,
                pending_coverage,
            );
        }
    });
}

pub(super) fn record_discovered_count(count: usize) {
    let discovered_count = count.min(i64::MAX as usize) as i64;
    let discovered_observed_at = format_utc_iso_millis(Utc::now());
    let discovered_range = "complete eligible invocation snapshot at run start".to_string();
    let _ = RETENTION_WORKLOAD_OBSERVATION.try_with(|observation| {
        if let Some(observation) = observation.borrow().as_ref() {
            observation.set_discovered_work(
                discovered_count,
                discovered_observed_at,
                discovered_range,
            );
        }
    });
}

pub(super) fn record_processed_count(count: usize) {
    let _ = RETENTION_WORKLOAD_OBSERVATION.try_with(|observation| {
        if let Some(observation) = observation.borrow().as_ref() {
            observation.add_processed_work(count as i64);
        }
    });
}
