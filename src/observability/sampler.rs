use crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator;
use crate::*;

pub(crate) fn spawn_observability_sampler(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(async move {
        if !state.observability.enabled {
            return;
        }
        let mut ticker = tokio::time::interval(Duration::from_secs(5));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut resource_tick = 0u8;
        loop {
            tokio::select! {
                _=state.shutdown.cancelled()=>return,
                _=ticker.tick()=>{
                    state.observability.handle.run_upkeep();
                    sample_cpu(&state.observability);
                    sample_runtime(&state).await;
                    if resource_tick.is_multiple_of(6) {sample_process_health(&state).await;}
                    resource_tick=(resource_tick+1)%6;
                }
            }
        }
    })
}
#[cfg(target_os = "linux")]
fn sample_cpu(metrics: &ObservabilityRuntime) {
    record_cpu_sample(metrics, cpu_ticks());
}
#[cfg(target_os = "linux")]
pub(crate) fn record_cpu_sample(metrics: &ObservabilityRuntime, ticks: Option<(u64, u64, f64)>) {
    let Some((user, system, hz)) = ticks else {
        metrics.degraded.store(true, Ordering::Relaxed);
        metrics.counter(
            "cvm_observability_sampler_errors_total",
            &[("source", "cpu")],
            1,
        );
        return;
    };
    metrics.gauge(
        "cvm_observability_sampler_last_success_timestamp_seconds",
        &[("source", "cpu")],
        Utc::now().timestamp() as f64,
    );
    metrics
        .cpu_user_nanos
        .store((user as f64 / hz * 1e9) as u64, Ordering::Relaxed);
    metrics
        .cpu_system_nanos
        .store((system as f64 / hz * 1e9) as u64, Ordering::Relaxed);
    metrics.cpu_valid.store(true, Ordering::Release);
}
#[cfg(target_os = "linux")]
fn cpu_ticks() -> Option<(u64, u64, f64)> {
    let process_stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let mut fields = process_stat
        .get(process_stat.rfind(')')? + 2..)?
        .split_whitespace();
    let user = fields.nth(11)?.parse().ok()?;
    let system = fields.next()?.parse().ok()?;
    // sysconf reads a process constant and has no pointer or mutation precondition.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    (hz > 0).then_some((user, system, hz as f64))
}
#[cfg(not(target_os = "linux"))]
fn sample_cpu(_: &ObservabilityRuntime) {}
async fn sample_runtime(state: &AppState) {
    let metrics = &state.observability;
    let accounting = state.sqlite_batch_writer.accounting_snapshot();
    metrics.record_gauge("p1.queue_depth", "p1", accounting.pending_depth as f64);
    metrics.record_gauge("p1.queue_bytes", "p1", accounting.pending_bytes as f64);
    let (p1, p2) = state.sqlite_batch_writer.observed_queue_depths();
    metrics.gauge(
        "cvm_sqlite_pending_items",
        &[("queue", "p1_terminal")],
        p1 as f64,
    );
    metrics.record_gauge("p2.queue_depth", "p2", p2 as f64);
    metrics.record_gauge(
        "p2.next_attempt_delay_ms",
        "p2",
        accounting.p2_next_attempt_in_ms as f64,
    );
    metrics.record_gauge(
        "p2.deferred_age_ms",
        "p2",
        accounting.p2_deferred_age_ms as f64,
    );
    let coordinator = proxy_sqlite_write_coordinator().snapshot().await;
    for (class, count) in [
        ("p1_terminal", coordinator.p1_waiter_count),
        ("interactive_proxy", coordinator.interactive_waiter_count),
        ("p2_derived", coordinator.p2_waiter_count),
        (
            "maintenance_retention",
            coordinator.maintenance_waiter_count,
        ),
    ] {
        metrics.record_gauge("sqlite.coordinator_waiters", class, count as f64);
    }
    let pressure = crate::db_pressure::global_db_pressure_gate().snapshot();
    for (id, value) in [
        ("sqlite.busy_count", pressure.sqlite_busy_events),
        ("sqlite.locked_count", pressure.sqlite_locked_events),
        (
            "sqlite.pool_timeout_count",
            pressure.pool_acquire_timeout_events,
        ),
        ("sqlite.background_skip_count", pressure.background_skips),
    ] {
        metrics.source_counter(id, "main", value);
    }
    let subscribers = state
        .subscription_hub
        .dashboard_activity_live_subscriber_count()
        .await;
    let projection = state.proxy_runtime_invocations.health_snapshot(subscribers);
    metrics.source_counter(
        "projection.build_count",
        "dashboard",
        projection.build_count,
    );
    metrics.source_counter(
        "projection.live_db_read_count",
        "dashboard",
        projection.live_path_db_read_count,
    );
    for (slice, value) in [
        ("current", &projection.slice_counters.current),
        ("network", &projection.slice_counters.network),
        ("terminal", &projection.slice_counters.terminal),
    ] {
        metrics.source_counter(
            "projection.cadence_miss_count",
            slice,
            value.cadence_miss_count,
        );
        metrics.source_counter("projection.revision_count", slice, value.revision_count);
    }
    if let Some(age) = projection.last_good_age_ms {
        metrics.record_gauge("projection.last_good_age_ms", "dashboard", age as f64);
    }
    metrics.record_gauge("sse.active_subscribers", "dashboard", subscribers as f64);
    let retention = retention_recovery_health_snapshot();
    if let Some(age) = retention
        .oldest_backlog_age_secs
        .or_else(|| (retention.expired_backlog_count == Some(0)).then_some(0))
    {
        metrics.record_gauge(
            "maintenance.backlog_age_ms",
            "maintenance",
            age as f64 * 1000.0,
        );
    }
}

async fn sample_process_health(state: &AppState) {
    let telemetry = &state.observability;
    let snapshot = state.memory_diagnostics.runtime_pressure_snapshot();
    if snapshot.process.rss_bytes > 0 {
        telemetry.gauge(
            "cvm_observability_sampler_last_success_timestamp_seconds",
            &[("source", "memory")],
            Utc::now().timestamp() as f64,
        );
        telemetry.record_gauge(
            "process.rss_bytes",
            "process",
            snapshot.process.rss_bytes as f64,
        );
        telemetry.record_gauge(
            "process.rss_anon_bytes",
            "process",
            snapshot.process.rss_anon_bytes as f64,
        );
        telemetry.record_gauge(
            "process.swap_bytes",
            "process",
            snapshot.process.swap_bytes as f64,
        );
    }
    telemetry.record_gauge(
        "process.managed_bytes",
        "process",
        snapshot.managed_bytes as f64,
    );
    if snapshot.process.rss_bytes > 0 {
        telemetry.record_gauge(
            "process.unattributed_anon_bytes",
            "process",
            snapshot.unattributed_anon_bytes as f64,
        );
    }
    if let Some(thread_count) = read_process_thread_count() {
        telemetry.record_gauge("process.thread_count", "process", thread_count as f64);
    }
    if let Some(available_bytes) = crate::proxy::filesystem_available_bytes(
        state
            .config
            .database_path
            .parent()
            .unwrap_or_else(|| Path::new(".")),
    ) {
        telemetry.record_gauge("process.disk_free_bytes", "process", available_bytes as f64);
    }
    if let Ok(metadata) = std::fs::metadata(&state.config.database_path) {
        telemetry.gauge(
            "cvm_observability_sampler_last_success_timestamp_seconds",
            &[("source", "files")],
            Utc::now().timestamp() as f64,
        );
        telemetry.record_gauge("storage.main_db_bytes", "main_db", metadata.len() as f64);
    }
    let main_wal_path = PathBuf::from(format!("{}-wal", state.config.database_path.display()));
    match std::fs::metadata(main_wal_path) {
        Ok(metadata) => telemetry.record_gauge("sqlite.wal_bytes", "main", metadata.len() as f64),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            telemetry.record_gauge("sqlite.wal_bytes", "main", 0.0)
        }
        Err(_) => {}
    }
}

#[cfg(target_os = "linux")]
fn read_process_thread_count() -> Option<u64> {
    fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("Threads:")?.trim().parse().ok())
}

#[cfg(not(target_os = "linux"))]
fn read_process_thread_count() -> Option<u64> {
    None
}
