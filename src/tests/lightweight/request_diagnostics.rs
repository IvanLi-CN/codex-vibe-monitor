use super::*;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};

#[derive(Clone, Debug, Default)]
struct MemoryExporter(Arc<std::sync::Mutex<Vec<SpanData>>>);
impl SpanExporter for MemoryExporter {
    async fn export(&self, spans: Vec<SpanData>) -> OTelSdkResult {
        self.0.lock().unwrap().extend(spans);
        Ok(())
    }
}
fn observed() -> (Arc<ObservabilityRuntime>, MemoryExporter) {
    let metrics = ObservabilityRuntime::new(true);
    let exporter = MemoryExporter::default();
    metrics
        .traces
        .set(super::super::traces::TraceRuntime::for_test(
            exporter.clone(),
        ))
        .unwrap();
    (metrics, exporter)
}
fn attribute(span: &SpanData, key: &str) -> Option<String> {
    span.attributes
        .iter()
        .find(|a| a.key.as_str() == key)
        .map(|a| a.value.to_string())
}

#[test]
fn request_diagnostics_body_end_once_and_late_commit_share_trace() {
    let (metrics, exporter) = observed();
    let context = DiagnosticContext::begin(metrics.clone(), "responses").unwrap();
    context.milestone("head");
    context.milestone("first_byte");
    let persistence = context.persistence();
    for _ in 0..2 {
        context.phase(Phase::Attempt).complete();
    }
    context.finish_response("complete", "2xx");
    context.finish_response("cancelled", "2xx");
    std::thread::sleep(Duration::from_millis(2));
    persistence.finish("committed", true);
    persistence.finish("committed", true);
    let spans = exporter.0.lock().unwrap();
    let response = spans
        .iter()
        .find(|s| s.name == "cvm.proxy.response")
        .unwrap();
    let durable = spans
        .iter()
        .find(|s| s.name == "cvm.terminal.enqueue_to_commit")
        .unwrap();
    assert_eq!(
        spans
            .iter()
            .filter(|s| s.name == "cvm.proxy.response")
            .count(),
        1
    );
    assert_eq!(
        spans
            .iter()
            .filter(|s| s.name == "cvm.terminal.enqueue_to_commit")
            .count(),
        1
    );
    assert_eq!(
        response.span_context.trace_id(),
        durable.span_context.trace_id()
    );
    assert!(durable.end_time > response.end_time);
    assert_eq!(attribute(response, "cvm.attempts").as_deref(), Some("2"));
    assert_eq!(
        attribute(response, "cvm.persistence").as_deref(),
        Some("pending")
    );
    assert_eq!(
        attribute(durable, "cvm.exclusive_cost").as_deref(),
        Some("false")
    );
    assert_eq!(
        attribute(response, "cvm.ttft_state").as_deref(),
        Some("unobserved")
    );
    assert!(metrics.render().contains(
        "cvm_request_response_ends_total{endpoint=\"responses\",outcome=\"complete\"} 1"
    ));
}

#[test]
fn request_diagnostics_nonstream_ttft_is_not_applicable() {
    let (metrics, exporter) = observed();
    for endpoint in ["responses", "chat_completions", "image_generation"] {
        let context = DiagnosticContext::begin(metrics.clone(), endpoint).unwrap();
        context.ttft_applicable(false);
        context.finish_response("complete", "2xx");
    }
    let context = DiagnosticContext::begin(metrics.clone(), "responses").unwrap();
    context.ttft_applicable(true);
    context.milestone("model_delta");
    context.finish_response("complete", "2xx");
    let spans = exporter.0.lock().unwrap();
    assert_eq!(spans.len(), 4);
    for root in &spans[..3] {
        assert_eq!(
            attribute(root, "cvm.ttft_state").as_deref(),
            Some("not_applicable")
        );
        assert!(attribute(root, "cvm.model_delta_ms").is_none());
    }
    assert_eq!(
        attribute(&spans[3], "cvm.ttft_state").as_deref(),
        Some("observed")
    );
    assert!(attribute(&spans[3], "cvm.model_delta_ms").is_some());
}

#[test]
fn request_diagnostics_nested_wait_union_and_affected_denominator() {
    let mut union = IntervalUnion::default();
    union.begin(Duration::from_millis(10));
    union.begin(Duration::from_millis(20));
    union.end(Duration::from_millis(40));
    assert_eq!(
        union.at(Duration::from_millis(50)),
        Duration::from_millis(40)
    );
    union.end(Duration::from_millis(60));
    union.begin(Duration::from_millis(90));
    union.end(Duration::from_millis(100));
    assert_eq!(
        union.at(Duration::from_millis(110)),
        Duration::from_millis(60)
    );
    let (metrics, _) = observed();
    let context = DiagnosticContext::begin(metrics.clone(), "other").unwrap();
    context.waiting(Resource::DbPool).complete();
    context.waiting(Resource::DbPool).complete();
    context.finish_response("error", "5xx");
    let rendered = metrics.render();
    assert!(
        rendered
            .contains("cvm_request_wait_affected_total{endpoint=\"other\",resource=\"db_pool\"} 1")
    );
    assert!(
        rendered.contains(
            "cvm_resource_wait_events_total{resource=\"db_pool\",outcome=\"complete\"} 2"
        )
    );
    assert!(rendered.contains(
        "cvm_request_resource_wait_seconds_count{endpoint=\"other\",resource=\"db_pool\"} 1"
    ));
}

#[test]
fn request_diagnostics_detail_limits_preserve_summary_and_private_attributes() {
    let (_, exporter) = observed();
    let metrics = ObservabilityRuntime::new(true);
    metrics
        .traces
        .set(super::super::traces::TraceRuntime::for_test(
            exporter.clone(),
        ))
        .unwrap();
    let context = DiagnosticContext::begin(metrics.clone(), "responses").unwrap();
    for _ in 0..16 {
        context.phase(Phase::Attempt).complete();
    }
    for _ in 0..100 {
        context.phase(Phase::Connect).complete();
        context.waiting(Resource::DbPool).complete();
    }
    assert!(context.state().longest.len() <= WAIT_LIMIT);
    assert!(context.state().bytes <= BYTE_LIMIT);
    let persistence = context.persistence();
    context.finish_response("cancelled", "2xx");
    persistence.finish("committed", true);
    let spans = exporter.0.lock().unwrap();
    assert!(spans.len() <= SPAN_LIMIT);
    use prost::Message;
    let resource =
        opentelemetry_proto::transform::common::tonic::ResourceAttributesWithSchema::from(
            &opentelemetry_sdk::Resource::builder_empty().build(),
        );
    let wire = opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest {
        resource_spans:
            opentelemetry_proto::transform::trace::tonic::group_spans_by_resource_and_scope(
                spans.clone(),
                &resource,
            ),
    };
    assert!(wire.encoded_len() < BYTE_LIMIT);
    assert!(spans.iter().filter(|s| s.name == "attempt").count() <= ATTEMPT_LIMIT);
    let root = spans
        .iter()
        .find(|s| s.name == "cvm.proxy.response")
        .unwrap();
    assert_eq!(attribute(root, "cvm.attempts").as_deref(), Some("16"));
    assert_eq!(
        attribute(root, "cvm.detail_truncated").as_deref(),
        Some("true")
    );
    assert!(
        spans
            .iter()
            .any(|s| s.name == "cvm.terminal.enqueue_to_commit")
    );
    for span in spans.iter() {
        assert!(span.events.is_empty());
        assert!(
            span.attributes
                .iter()
                .all(|a| a.key.as_str().starts_with("cvm."))
        );
        assert!(!format!("{span:?}").contains("invoke_id"));
    }
}

#[test]
fn request_diagnostics_active_limit_and_failed_initialization_do_not_block() {
    let metrics = ObservabilityRuntime::new(true);
    let contexts = (0..super::super::traces::ACTIVE_LIMIT)
        .map(|_| DiagnosticContext::begin(metrics.clone(), "other").unwrap())
        .collect::<Vec<_>>();
    assert!(DiagnosticContext::begin(metrics.clone(), "other").is_none());
    drop(contexts);
    assert!(DiagnosticContext::begin(metrics.clone(), "other").is_some());
    assert!(DiagnosticContext::begin(ObservabilityRuntime::new(false), "other").is_none());
    let mut invalid = super::super::traces::TraceConfig::default();
    invalid.enabled = true;
    invalid.valid = false;
    let failed = super::super::traces::TraceRuntime::new(&invalid);
    assert_eq!(failed.state(), "degraded");
    assert!(failed.tracer.is_none());
}

#[tokio::test]
async fn request_diagnostics_spawn_propagates_and_cancelled_wait_has_lower_bound() {
    let (metrics, exporter) = observed();
    let context = DiagnosticContext::begin(metrics.clone(), "other").unwrap();
    scope(Some(context.clone()), async {
        let task = spawn(async {
            assert!(current().is_some());
            milestone("head");
        });
        task.await.unwrap();
        let guard = waiting(Resource::AccountCapacity).unwrap();
        context.finish_response("cancelled", "2xx");
        drop(guard);
    })
    .await;
    assert!(current().is_none());
    let spans = exporter.0.lock().unwrap();
    let root = spans
        .iter()
        .find(|s| s.name == "cvm.proxy.response")
        .unwrap();
    assert_eq!(attribute(root, "cvm.lower_bound").as_deref(), Some("true"));
    assert_eq!(
        attribute(root, "cvm.ttft_state").as_deref(),
        Some("not_applicable")
    );
}

#[test]
fn request_diagnostics_shared_batch_cost_has_links_and_replay_has_no_association() {
    let (metrics, exporter) = observed();
    let first = DiagnosticContext::begin(metrics.clone(), "responses").unwrap();
    let second = DiagnosticContext::begin(metrics, "chat_completions").unwrap();
    let tickets = [first.persistence(), second.persistence()];
    first.finish_response("complete", "2xx");
    second.finish_response("complete", "2xx");
    let mut batch = SharedBatch::begin(tickets.iter(), 2);
    batch.pool_acquired();
    let batch_id = batch.committed().unwrap();
    for ticket in &tickets {
        ticket.finish_with_batch("committed", true, Some(batch_id.clone()));
    }
    assert!(
        SharedBatch::begin(std::iter::empty(), 1)
            .committed()
            .is_none()
    );
    let spans = exporter.0.lock().unwrap();
    let shared = spans
        .iter()
        .find(|s| s.name == "cvm.terminal.shared_batch")
        .unwrap();
    assert_eq!(shared.links.len(), 2);
    assert_eq!(
        spans
            .iter()
            .filter(|s| s.name == "cvm.terminal.batch.execute")
            .count(),
        1
    );
    for span in spans
        .iter()
        .filter(|s| s.name == "cvm.terminal.enqueue_to_commit")
    {
        assert_eq!(span.links.len(), 1);
        assert_eq!(span.links[0].span_context, batch_id);
        assert_ne!(span.span_context.trace_id(), batch_id.trace_id());
    }
}

#[derive(Debug)]
struct BlockedFailureExporter(Arc<AtomicBool>);
impl SpanExporter for BlockedFailureExporter {
    async fn export(&self, _batch: Vec<SpanData>) -> OTelSdkResult {
        while self.0.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(1));
        }
        Err(opentelemetry_sdk::error::OTelSdkError::InternalFailure(
            "synthetic failure".into(),
        ))
    }
}
#[test]
fn request_diagnostics_sdk_queue_drops_without_waiting_for_failed_exporter() {
    let blocked = Arc::new(AtomicBool::new(true));
    let traces = super::super::traces::TraceRuntime::for_test_batched(BlockedFailureExporter(
        blocked.clone(),
    ));
    let tracer = traces.tracer.as_ref().unwrap();
    // Holding the exporter makes SDK backpressure deterministic, without a network.
    for _ in 0..(super::super::traces::QUEUE_LIMIT * 3) {
        tracer.start("cvm.synthetic.queue").end();
    }
    blocked.store(false, Ordering::Release);
    traces.shutdown();
    let metrics = ObservabilityRuntime::new(true);
    traces.report(&metrics);
    let rendered = metrics.render();
    assert!(!rendered.contains("cvm_trace_queue_dropped_spans_total 0\n"));
    assert!(!rendered.contains("cvm_trace_export_failed_spans_total 0\n"));
    assert_eq!(traces.state(), "degraded");
}

#[test]
fn request_diagnostics_prometheus_series_budget_preserves_existing_series() {
    let metrics = ObservabilityRuntime::new(true);
    metrics.counter("cvm.synthetic.existing", &[], 1);
    for value in 0..300 {
        let key = metrics::Key::from_parts(
            "cvm.synthetic.histogram",
            vec![metrics::Label::new("enum", value.to_string())],
        );
        metrics.register_histogram(key).record(0.1);
    }
    metrics.counter("cvm.synthetic.existing", &[], 1);
    let rendered = metrics.render();
    assert!(rendered.contains("cvm_synthetic_existing 2\n"));
    assert!(metrics.series.dropped() > 0);
    assert_eq!(metrics.state(), "degraded");
    assert!(
        rendered
            .lines()
            .filter(|s| !s.starts_with('#') && !s.is_empty())
            .count()
            <= 4000
    );
}
