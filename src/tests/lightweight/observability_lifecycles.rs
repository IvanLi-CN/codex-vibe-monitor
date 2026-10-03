use super::*;

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
