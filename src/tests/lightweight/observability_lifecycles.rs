use super::*;

fn resource_memory_sample(
    status: crate::memory_diagnostics::ProcessMemorySampleStatus,
) -> crate::memory_diagnostics::RuntimeMemoryPressureSnapshot {
    use crate::memory_diagnostics::{ProcessMemorySnapshot, RuntimeMemoryPressureSnapshot};
    RuntimeMemoryPressureSnapshot {
        process: ProcessMemorySnapshot {
            rss_bytes: 4096,
            rss_anon_bytes: 2048,
            ..Default::default()
        },
        process_sample_status: status,
        managed_bytes: 1024,
        unattributed_anon_bytes: 1024,
        pressure_level: "normal".into(),
        malloc_arena_max: "unknown".into(),
    }
}

#[test]
fn resource_sampler_memory_failure_preserves_last_good_values_and_freshness() {
    use crate::memory_diagnostics::ProcessMemorySampleStatus;
    let metrics = ObservabilityRuntime::new(true);
    observability::record_memory_sample(
        &metrics,
        &resource_memory_sample(ProcessMemorySampleStatus::Available),
        Some(3),
    );
    metrics.gauge(
        "cvm_observability_sampler_last_success_timestamp_seconds",
        &[("source", "memory")],
        123.0,
    );
    observability::record_memory_sample(
        &metrics,
        &resource_memory_sample(ProcessMemorySampleStatus::Failed),
        Some(3),
    );
    assert_eq!(metrics.state(), "degraded");
    let text = metrics.render();
    assert!(text.contains("cvm_process_rss_bytes 4096"), "{text}");
    assert!(text.contains("cvm_observability_sampler_errors_total{source=\"memory\"} 1"));
    assert!(text.contains(
        "cvm_observability_sampler_last_success_timestamp_seconds{source=\"memory\"} 123"
    ));
    let initial_failure = ObservabilityRuntime::new(true);
    observability::record_memory_sample(
        &initial_failure,
        &resource_memory_sample(ProcessMemorySampleStatus::Failed),
        Some(3),
    );
    assert!(!initial_failure.render().contains("cvm_process_rss_bytes"));
    assert!(
        !initial_failure
            .render()
            .contains("sampler_last_success_timestamp")
    );
}

#[test]
fn resource_sampler_pending_and_unsupported_memory_remain_unknown() {
    use crate::memory_diagnostics::ProcessMemorySampleStatus;
    for status in [
        ProcessMemorySampleStatus::Pending,
        ProcessMemorySampleStatus::Unsupported,
    ] {
        let metrics = ObservabilityRuntime::new(true);
        observability::record_memory_sample(&metrics, &resource_memory_sample(status), Some(3));
        assert_eq!(metrics.state(), "enabled");
        let text = metrics.render();
        assert!(!text.contains("cvm_process_rss_bytes"));
        assert!(!text.contains("sampler_errors_total"));
        assert!(!text.contains("sampler_last_success_timestamp"));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn resource_sampler_thread_read_failure_does_not_refresh_memory_success() {
    use crate::memory_diagnostics::ProcessMemorySampleStatus;
    let metrics = ObservabilityRuntime::new(true);
    observability::record_memory_sample(
        &metrics,
        &resource_memory_sample(ProcessMemorySampleStatus::Available),
        None,
    );
    assert_eq!(metrics.state(), "degraded");
    let text = metrics.render();
    assert!(text.contains("cvm_process_rss_bytes 4096"));
    assert!(!text.contains("cvm_process_threads"));
    assert!(!text.contains("sampler_last_success_timestamp"));
    assert!(text.contains("cvm_observability_sampler_errors_total{source=\"memory\"} 1"));
}

#[test]
fn resource_sampler_file_failures_preserve_values_and_absent_wal_is_zero() {
    use std::io::ErrorKind;
    for (disk, database, wal) in [
        (Some(8192), Err(ErrorKind::PermissionDenied), Ok(2048)),
        (Some(8192), Ok(4096), Err(ErrorKind::PermissionDenied)),
        #[cfg(unix)]
        (None, Ok(4096), Ok(2048)),
    ] {
        let metrics = ObservabilityRuntime::new(true);
        observability::record_file_samples(&metrics, Some(8192), Some(Ok(4096)), Some(Ok(2048)));
        metrics.gauge(
            "cvm_observability_sampler_last_success_timestamp_seconds",
            &[("source", "files")],
            123.0,
        );
        observability::record_file_samples(&metrics, disk, Some(database), Some(wal));
        assert_eq!(metrics.state(), "degraded");
        let text = metrics.render();
        assert!(
            text.contains("cvm_storage_database_bytes{database=\"main\"} 4096"),
            "{text}"
        );
        assert!(text.contains("cvm_sqlite_wal_bytes{database=\"main\"} 2048"));
        assert!(text.contains("cvm_observability_sampler_errors_total{source=\"files\"} 1"));
        assert!(text.contains(
            "cvm_observability_sampler_last_success_timestamp_seconds{source=\"files\"} 123"
        ));
    }
    let absent_wal = ObservabilityRuntime::new(true);
    observability::record_file_samples(
        &absent_wal,
        Some(8192),
        Some(Ok(4096)),
        Some(Err(ErrorKind::NotFound)),
    );
    assert_eq!(absent_wal.state(), "enabled");
    assert!(
        absent_wal
            .render()
            .contains("cvm_sqlite_wal_bytes{database=\"main\"} 0")
    );
    let in_memory = ObservabilityRuntime::new(true);
    observability::record_file_samples(&in_memory, Some(8192), None, None);
    assert_eq!(in_memory.state(), "enabled");
    assert!(!in_memory.render().contains("cvm_storage_database_bytes"));
    assert!(!in_memory.render().contains("cvm_sqlite_wal_bytes"));
    let initial_failure = ObservabilityRuntime::new(true);
    observability::record_file_samples(
        &initial_failure,
        Some(8192),
        Some(Err(ErrorKind::NotFound)),
        Some(Err(ErrorKind::PermissionDenied)),
    );
    assert_eq!(initial_failure.state(), "degraded");
    let text = initial_failure.render();
    assert!(!text.contains("cvm_storage_database_bytes"));
    assert!(!text.contains("cvm_sqlite_wal_bytes"));
    assert!(!text.contains("sampler_last_success_timestamp"));
}

#[cfg(target_os = "linux")]
#[test]
fn cpu_sampler_failure_degrades_capabilities_without_fabricating_samples() {
    let metrics = ObservabilityRuntime::new(true);
    let independent = ObservabilityRuntime::new(true);
    assert_eq!(metrics.state(), "enabled");
    observability::record_cpu_sample(&metrics, Some((100, 20, 100.0)));
    let before = metrics.render();
    assert!(before.contains("cvm_process_cpu_seconds_total{mode=\"user\"} 1"));
    observability::record_cpu_sample(&metrics, None);
    assert_eq!(metrics.state(), "degraded");
    assert_eq!(independent.state(), "enabled");
    let failed = metrics.render();
    assert!(failed.contains("cvm_observability_sampler_errors_total{source=\"cpu\"} 1"));
    assert!(failed.contains("cvm_process_cpu_seconds_total{mode=\"user\"} 1"));
    assert!(failed.contains("cvm_process_cpu_seconds_total{mode=\"system\"} 0.2"));
    let disabled = ObservabilityRuntime::new(false);
    disabled.degraded.store(true, Ordering::Relaxed);
    assert_eq!(disabled.state(), "disabled");
}

#[tokio::test]
async fn coordinator_recorders_are_isolated_and_retired_bindings_can_be_replaced() {
    use crate::proxy_sqlite_write_coordinator::{
        ProxySqliteWriteClass, test_proxy_sqlite_write_coordinator,
    };
    let first = ObservabilityRuntime::new(true);
    let second = ObservabilityRuntime::new(true);
    let coordinator = test_proxy_sqlite_write_coordinator();
    let independent = test_proxy_sqlite_write_coordinator();
    coordinator
        .bind_observability(first.clone())
        .expect("first runtime");
    independent
        .bind_observability(second.clone())
        .expect("independent runtime");
    assert!(coordinator.bind_observability(second.clone()).is_err());
    let permit = coordinator.acquire(ProxySqliteWriteClass::P1Terminal).await;
    assert!(
        first.render().contains(
            "cvm_sqlite_coordinator_wait_duration_seconds_count{class=\"p1_terminal\"} 1"
        )
    );
    assert!(
        !second
            .render()
            .contains("cvm_sqlite_coordinator_wait_duration_seconds")
    );
    let retired = Arc::downgrade(&first);
    drop(first);
    assert!(
        coordinator.bind_observability(second.clone()).is_err(),
        "live permits retain their original recorder"
    );
    drop(permit);
    assert!(
        retired.upgrade().is_none(),
        "global arbiter must not pin an ended runtime"
    );
    coordinator
        .bind_observability(second.clone())
        .expect("replacement runtime");
    drop(coordinator.acquire(ProxySqliteWriteClass::P2Derived).await);
    assert!(
        second
            .render()
            .contains("cvm_sqlite_coordinator_hold_duration_seconds_count{class=\"p2_derived\"} 1")
    );
}

#[test]
fn websocket_terminal_dispositions_are_exported_once_and_drop_is_cancelled() {
    let metrics = ObservabilityRuntime::new(true);
    for (failure, kind, expected) in [
        (None, None, "success"),
        (Some("upstream disconnected"), None, "error"),
        (
            Some("downstream closed"),
            Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED),
            "cancelled",
        ),
    ] {
        let outcome = websocket_terminal_outcome(failure, kind);
        assert_eq!(outcome, expected);
        let mut lifetime = observability::WebsocketLifetime::new(metrics.clone(), "/v1/responses");
        lifetime.finish(outcome);
        lifetime.finish("unknown");
    }
    drop(observability::WebsocketLifetime::new(
        metrics.clone(),
        "/v1/responses",
    ));
    let text = metrics.render();
    for (outcome, count) in [("success", 1), ("error", 1), ("cancelled", 2)] {
        assert!(text.contains(&format!("cvm_proxy_stream_ends_total{{endpoint=\"responses\",transport=\"websocket\",outcome=\"{outcome}\"}} {count}")));
    }
    assert!(!text.contains("outcome=\"unknown\""));
    assert!(text.contains("cvm_http_inflight 0"));
}

#[test]
fn proxy_unknown_phase_sentinels_are_missing_but_optional_zero_ttft_is_a_sample() {
    let metrics = ObservabilityRuntime::new(true);
    let mut record = test_proxy_capture_record("zero-timing", "2026-10-03 00:00:00");
    record.timings.t_upstream_ttfb_ms = 0.0;
    record.timings.t_upstream_stream_ms = 0.0;
    record.timings.first_token_ms = Some(0.0);
    metrics.proxy_terminal(&record, Some("/v1/responses"));
    let text = metrics.render();
    assert!(!text.contains("cvm_proxy_ttfb_seconds"));
    assert!(!text.contains("cvm_proxy_stream_duration_seconds"));
    assert!(text.contains("cvm_proxy_ttft_seconds_count{endpoint=\"responses\"} 1"));
    assert!(text.contains("cvm_proxy_ttft_seconds_sum{endpoint=\"responses\"} 0"));
}
