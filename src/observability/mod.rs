use crate::*;
use metrics::{Key, Label, Metadata, Recorder};
use metrics_exporter_prometheus::{
    Matcher, PrometheusBuilder, PrometheusHandle, PrometheusRecorder,
};

mod browser;
mod config;
pub(crate) mod diagnostics;
mod http;
mod limits;
mod locks;
pub(crate) mod traces;
pub(crate) use locks::DiagnosticMutex;
pub(super) static PROFILER_ENABLED: AtomicBool = AtomicBool::new(false);
mod registry;
mod reports;
mod sampler;
// Exercise the exact vendored normalization/cache source with this project's runner.
#[cfg(test)]
#[path = "../../vendor/hotpath/src/lib_on/sql/normalize.rs"]
mod hotpath_sql_normalization;
pub(crate) use browser::{BROWSER_MAX_BYTES, browser_ingest_router};
pub(crate) use config::{ObservabilityConfig, prepare_hotpath};
pub(crate) use http::{
    http_request_trace_path, observability_http_middleware, retired_performance_preflight,
};
pub(crate) use reports::{hotpath_report, observability_capabilities};
#[cfg(all(test, target_os = "linux"))]
pub(crate) use sampler::record_cpu_sample;
pub(crate) use sampler::spawn_observability_sampler;
#[cfg(test)]
pub(crate) use sampler::{record_file_samples, record_memory_sample};

const METADATA: Metadata<'static> = Metadata::new("cvm", metrics::Level::INFO, None);
const SHORT_BUCKETS: &[f64] = &[
    0.00001, 0.00005, 0.0001, 0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5,
    5.0, 10.0,
];
const REQUEST_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0,
];

pub(crate) struct ObservabilityRuntime {
    pub(crate) enabled: bool,
    recorder: PrometheusRecorder,
    pub(crate) handle: PrometheusHandle,
    pub(crate) started_at: DateTime<Utc>,
    pub(crate) degraded: AtomicBool,
    cpu_user_nanos: AtomicU64,
    cpu_system_nanos: AtomicU64,
    cpu_valid: AtomicBool,
    browser_limiter: std::sync::Mutex<browser::BrowserLimiter>,
    report_limiter: std::sync::Mutex<reports::ReportLimiter>,
    traces: std::sync::OnceLock<Arc<traces::TraceRuntime>>,
    http_inflight: std::sync::OnceLock<metrics::Gauge>,
    http_metrics: [std::sync::OnceLock<Arc<http::HttpMetricHandles>>;
        http::HTTP_ROUTE_COUNT * http::HTTP_METHOD_COUNT],
    request_metrics:
        [std::sync::OnceLock<Arc<diagnostics::RequestMetricHandles>>; diagnostics::ENDPOINT_COUNT],
    series: limits::SeriesBudget,
}
impl std::fmt::Debug for ObservabilityRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObservabilityRuntime")
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}
impl ObservabilityRuntime {
    pub(crate) fn state(&self) -> &'static str {
        if !self.enabled {
            "disabled"
        } else if self.degraded.load(Ordering::Relaxed) {
            "degraded"
        } else {
            "enabled"
        }
    }

    pub(crate) fn new(enabled: bool) -> Arc<Self> {
        let mut builder = PrometheusBuilder::new()
            .set_buckets(SHORT_BUCKETS)
            .expect("static buckets");
        for name in [
            "cvm_http_header_duration_seconds",
            "cvm_http_body_duration_seconds",
            "cvm_proxy_stream_duration_seconds",
            "cvm_task_run_duration_seconds",
            "cvm_proxy_phase_duration_seconds",
            "cvm_proxy_ttfb_seconds",
            "cvm_proxy_ttft_seconds",
            "cvm_request_local_wait_seconds",
            "cvm_request_resource_wait_seconds",
            "cvm_request_unattributed_seconds",
            "cvm_request_response_duration_seconds",
            "cvm_request_stage_seconds",
            "cvm_request_milestone_seconds",
            "cvm_request_persistence_seconds",
        ] {
            builder = builder
                .set_buckets_for_metric(Matcher::Full(name.into()), REQUEST_BUCKETS)
                .expect("static request buckets");
        }
        let recorder = builder.build_recorder();
        let handle = recorder.handle();
        Arc::new(Self {
            enabled,
            recorder,
            handle,
            started_at: Utc::now(),
            degraded: AtomicBool::new(false),
            cpu_user_nanos: AtomicU64::new(0),
            cpu_system_nanos: AtomicU64::new(0),
            cpu_valid: AtomicBool::new(false),
            browser_limiter: std::sync::Mutex::new(browser::BrowserLimiter::default()),
            report_limiter: std::sync::Mutex::new(reports::ReportLimiter::default()),
            traces: std::sync::OnceLock::new(),
            http_inflight: std::sync::OnceLock::new(),
            http_metrics: std::array::from_fn(|_| std::sync::OnceLock::new()),
            request_metrics: std::array::from_fn(|_| std::sync::OnceLock::new()),
            series: limits::SeriesBudget::default(),
        })
    }
    #[cfg(test)]
    pub(crate) fn disabled_for_tests() -> Arc<Self> {
        Self::new(false)
    }
    fn key(name: &'static str, labels: &[(&'static str, &'static str)]) -> Key {
        Key::from_parts(
            name,
            labels
                .iter()
                .map(|(key, value)| Label::new(*key, *value))
                .collect::<Vec<_>>(),
        )
    }
    fn register_counter(&self, key: Key) -> metrics::Counter {
        if self.series.admit(&key, 0) {
            self.recorder.register_counter(&key, &METADATA)
        } else {
            self.degraded.store(true, Ordering::Relaxed);
            metrics::Counter::noop()
        }
    }
    fn register_gauge(&self, key: Key) -> metrics::Gauge {
        if self.series.admit(&key, 1) {
            self.recorder.register_gauge(&key, &METADATA)
        } else {
            self.degraded.store(true, Ordering::Relaxed);
            metrics::Gauge::noop()
        }
    }
    fn register_histogram(&self, key: Key) -> metrics::Histogram {
        if self.series.admit(&key, 2) {
            self.recorder.register_histogram(&key, &METADATA)
        } else {
            self.degraded.store(true, Ordering::Relaxed);
            metrics::Histogram::noop()
        }
    }
    pub(crate) fn counter(
        &self,
        name: &'static str,
        labels: &[(&'static str, &'static str)],
        value: u64,
    ) {
        if self.enabled {
            self.register_counter(Self::key(name, labels))
                .increment(value);
        }
    }
    pub(crate) fn render(&self) -> String {
        if let Some(traces) = self.traces.get() {
            traces.report(self);
        }
        let mut output = self.handle.render();
        let dropped = self.series.dropped();
        if dropped > 0 {
            output.push_str(&format!(
                "# TYPE cvm_metric_series_dropped_total counter\ncvm_metric_series_dropped_total {dropped}\n"
            ));
        }
        if self.cpu_valid.load(Ordering::Acquire) {
            output.push_str("# TYPE cvm_process_cpu_seconds_total counter\n");
            for (mode, value) in [
                ("user", self.cpu_user_nanos.load(Ordering::Relaxed)),
                ("system", self.cpu_system_nanos.load(Ordering::Relaxed)),
            ] {
                output.push_str(&format!(
                    "cvm_process_cpu_seconds_total{{mode=\"{mode}\"}} {}\n",
                    value as f64 / 1e9
                ));
            }
        }
        output
    }
    pub(crate) fn trace_runtime(&self) -> Arc<traces::TraceRuntime> {
        self.traces
            .get_or_init(traces::TraceRuntime::disabled)
            .clone()
    }
    fn http_inflight(&self) -> &metrics::Gauge {
        self.http_inflight.get_or_init(|| {
            if self.enabled {
                self.register_gauge(Key::from_name("cvm_http_inflight"))
            } else {
                metrics::Gauge::noop()
            }
        })
    }
    fn http_metrics(
        &self,
        route: &'static str,
        method: &'static str,
    ) -> Arc<http::HttpMetricHandles> {
        let index = http::route_index(route) * http::HTTP_METHOD_COUNT + http::method_index(method);
        self.http_metrics[index]
            .get_or_init(|| Arc::new(http::HttpMetricHandles::new(route, method)))
            .clone()
    }
    fn request_metrics(&self, endpoint: &'static str) -> Arc<diagnostics::RequestMetricHandles> {
        let index = diagnostics::endpoint_index(endpoint);
        self.request_metrics[index]
            .get_or_init(|| Arc::new(diagnostics::RequestMetricHandles::new(self, endpoint)))
            .clone()
    }
    pub(crate) async fn initialize_traces(&self, config: traces::TraceConfig) {
        let requested = config.enabled;
        let runtime = tokio::task::spawn_blocking(move || traces::TraceRuntime::new(&config))
            .await
            .unwrap_or_else(|_| {
                let mut failed = traces::TraceConfig::default();
                failed.enabled = requested;
                failed.valid = false;
                traces::TraceRuntime::new(&failed)
            });
        let _ = self.traces.set(runtime);
    }
    pub(crate) fn source_absolute(&self, name: &'static str, value: u64) {
        if self.enabled {
            self.register_counter(Key::from_name(name)).absolute(value);
        }
    }
    pub(crate) fn source_counter(&self, id: &'static str, dimension: &'static str, value: u64) {
        if self.enabled
            && let Some((name, labels, _)) = registry::mapped_metric(id, dimension)
        {
            self.register_counter(Key::from_parts(name, labels))
                .absolute(value);
        }
    }
    pub(crate) fn gauge(
        &self,
        name: &'static str,
        labels: &[(&'static str, &'static str)],
        value: f64,
    ) {
        if self.enabled && value.is_finite() {
            self.register_gauge(Self::key(name, labels)).set(value);
        }
    }
    pub(crate) fn duration(
        &self,
        name: &'static str,
        labels: &[(&'static str, &'static str)],
        value: Duration,
    ) {
        if self.enabled {
            self.register_histogram(Self::key(name, labels))
                .record(value.as_secs_f64());
        }
    }
    pub(crate) fn record_counter(&self, id: &'static str, dimension: &'static str, value: u64) {
        if !self.enabled {
            return;
        }
        if let Some((name, labels, _)) = registry::mapped_metric(id, dimension) {
            self.register_counter(Key::from_parts(name, labels))
                .increment(value);
        }
    }
    pub(crate) fn record_duration_ms(&self, id: &'static str, dimension: &'static str, value: f64) {
        if !self.enabled || !value.is_finite() || value < 0.0 {
            return;
        }
        if let Some((name, labels, scale)) = registry::mapped_metric(id, dimension) {
            self.register_histogram(Key::from_parts(name, labels))
                .record(value * scale);
        }
    }
    pub(crate) fn record_gauge(&self, id: &'static str, dimension: &'static str, value: f64) {
        if !self.enabled || !value.is_finite() || value < 0.0 {
            return;
        }
        if let Some((name, labels, scale)) = registry::mapped_metric(id, dimension) {
            self.register_gauge(Key::from_parts(name, labels))
                .set(value * scale);
        }
    }
    pub(crate) fn task_completed(
        &self,
        key: &'static str,
        outcome: &'static str,
        duration: Duration,
    ) {
        self.counter(
            "cvm_task_runs_total",
            &[("task_key", key), ("outcome", outcome)],
            1,
        );
        self.duration(
            "cvm_task_run_duration_seconds",
            &[("task_key", key)],
            duration,
        );
    }
    pub(crate) fn start_exporter(
        self: &Arc<Self>,
        config: &ObservabilityConfig,
        shutdown: CancellationToken,
    ) {
        if !self.enabled {
            return;
        }
        let runtime = self.clone();
        let config = config.clone();
        tokio::spawn(async move {
            let listener = match TcpListener::bind(config.metrics_bind).await {
                Ok(v) => v,
                Err(_) => {
                    runtime.degraded.store(true, Ordering::Relaxed);
                    warn!("metrics listener unavailable");
                    return;
                }
            };
            let app = Router::new().route(
                "/metrics",
                get({
                    let runtime = runtime.clone();
                    move |headers: HeaderMap| {
                        let runtime = runtime.clone();
                        let token = config.scrape_token.clone();
                        async move {
                            if token.is_some() && !config::authorized(&headers, token.as_deref()) {
                                return StatusCode::UNAUTHORIZED.into_response();
                            }
                            (
                                [(
                                    header::CONTENT_TYPE,
                                    "text/plain; version=0.0.4; charset=utf-8",
                                )],
                                runtime.render(),
                            )
                                .into_response()
                        }
                    }
                }),
            );
            if axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    shutdown.cancelled().await;
                })
                .await
                .is_err()
            {
                runtime.degraded.store(true, Ordering::Relaxed);
            }
        });
    }
}

#[derive(Debug, Default)]
pub(crate) struct MetricsSink(std::sync::OnceLock<Arc<ObservabilityRuntime>>);
impl MetricsSink {
    pub(crate) fn bind(&self, metrics: Arc<ObservabilityRuntime>) {
        let _ = self.0.set(metrics);
    }
    pub(crate) fn get(&self) -> Option<&ObservabilityRuntime> {
        self.0.get().map(Arc::as_ref)
    }
}

pub(crate) async fn performance_retired() -> Response {
    (
        StatusCode::GONE,
        Json(json!({"code":"performance_retired","replacement":"/api/system/observability"})),
    )
        .into_response()
}

/// Conditional scope keeps the same binary usable for the enabled/disabled experiment.
pub(crate) async fn observed_future<F: Future>(
    enabled: bool,
    name: &'static str,
    future: F,
) -> F::Output {
    if enabled {
        hotpath::measure_block!(name, future.await)
    } else {
        future.await
    }
}

impl ObservabilityRuntime {
    pub(crate) fn proxy_terminal(&self, record: &ProxyCaptureRecord, endpoint: Option<&str>) {
        let endpoint = endpoint_from_path(endpoint.unwrap_or(""));
        let outcome = match record.status.as_str() {
            "success" => "success",
            "cancelled" | "canceled" => "cancelled",
            _ => "error",
        };
        self.counter(
            "cvm_proxy_invocations_total",
            &[("endpoint", endpoint), ("outcome", outcome)],
            1,
        );
        let t = &record.timings;
        for (phase, value) in [
            ("total", t.t_total_ms),
            ("request_read", t.t_req_read_ms),
            ("request_parse", t.t_req_parse_ms),
            ("connect", t.t_upstream_connect_ms),
            ("first_byte", t.t_upstream_ttfb_ms),
            ("stream", t.t_upstream_stream_ms),
            ("response_parse", t.t_resp_parse_ms),
            ("persist", t.t_persist_ms),
        ] {
            if value.is_finite() && value > 0.0 {
                self.duration(
                    "cvm_proxy_phase_duration_seconds",
                    &[("endpoint", endpoint), ("phase", phase)],
                    Duration::from_secs_f64(value / 1000.0),
                );
            }
        }
        if t.t_upstream_ttfb_ms.is_finite() && t.t_upstream_ttfb_ms > 0.0 {
            self.duration(
                "cvm_proxy_ttfb_seconds",
                &[("endpoint", endpoint)],
                Duration::from_secs_f64(t.t_upstream_ttfb_ms / 1000.0),
            );
        }
        if let Some(ttft) = t
            .first_token_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.duration(
                "cvm_proxy_ttft_seconds",
                &[("endpoint", endpoint)],
                Duration::from_secs_f64(ttft / 1000.0),
            );
        }
        if t.t_upstream_stream_ms.is_finite() && t.t_upstream_stream_ms > 0.0 {
            self.duration(
                "cvm_proxy_stream_duration_seconds",
                &[("endpoint", endpoint)],
                Duration::from_secs_f64(t.t_upstream_stream_ms / 1000.0),
            );
        }
    }
}
pub(crate) struct ObservationSpan {
    metrics: Arc<ObservabilityRuntime>,
    name: &'static str,
    labels: Vec<(&'static str, &'static str)>,
    started: Instant,
}
impl ObservationSpan {
    pub(crate) fn new(
        metrics: Arc<ObservabilityRuntime>,
        name: &'static str,
        labels: Vec<(&'static str, &'static str)>,
    ) -> Self {
        Self {
            metrics,
            name,
            labels,
            started: Instant::now(),
        }
    }
}
impl Drop for ObservationSpan {
    fn drop(&mut self) {
        self.metrics
            .duration(self.name, &self.labels, self.started.elapsed());
    }
}

pub(crate) fn endpoint_from_path(path: &str) -> &'static str {
    match path {
        "/v1/responses" => "responses",
        "/v1/chat/completions" => "chat_completions",
        _ => "other",
    }
}

pub(crate) struct UpstreamAttempt {
    metrics: Arc<ObservabilityRuntime>,
    endpoint: &'static str,
    pub(crate) outcome: &'static str,
}
impl UpstreamAttempt {
    pub(crate) fn new(metrics: Arc<ObservabilityRuntime>, path: &str) -> Self {
        Self {
            metrics,
            endpoint: endpoint_from_path(path),
            outcome: "error",
        }
    }
}
impl Drop for UpstreamAttempt {
    fn drop(&mut self) {
        self.metrics.counter(
            "cvm_proxy_upstream_attempts_total",
            &[("endpoint", self.endpoint), ("outcome", self.outcome)],
            1,
        );
    }
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    #[test]
    fn all_77_source_signals_have_their_locked_retirement_or_metric_action() {
        let contract = include_str!("../../docs/design/performance-observability-metrics.md");
        let mut sources = std::collections::HashSet::new();
        let mut retired = 0;
        let mut merged = 0;
        for line in contract.lines() {
            let columns: Vec<_> = line.split('|').map(str::trim).collect();
            let Some(source) = columns.get(1).and_then(|value| value.strip_prefix('`')) else {
                continue;
            };
            let Some(id) = source.strip_suffix('`').filter(|value| value.contains('.')) else {
                continue;
            };
            assert!(sources.insert(id), "duplicate source action: {id}");
            let action = columns[2];
            let dimension = registry::metric_dimensions(id)
                .first()
                .copied()
                .unwrap_or("");
            let mapping = registry::mapped_metric(id, dimension);
            if action == "退役" {
                retired += 1;
                assert!(mapping.is_none(), "retired source still exports: {id}");
            } else if action.starts_with("合并到") {
                merged += 1;
                assert_eq!(id, "sse.publish_duration_ms");
                assert!(mapping.is_none(), "duplicate publish window still exports");
            } else {
                let expected = action
                    .split('`')
                    .nth(1)
                    .expect("metric action")
                    .split('{')
                    .next()
                    .expect("metric name");
                let (name, _, scale) = mapping.unwrap_or_else(|| panic!("missing mapping: {id}"));
                assert_eq!(name, expected, "metric contract differs: {id}");
                assert_eq!(scale, if id.ends_with("_ms") { 0.001 } else { 1.0 });
                assert!(registry::mapped_metric(id, "sensitive-dynamic-value").is_none());
            }
        }
        assert_eq!(sources.len(), 77);
        assert_eq!(retired, 9);
        assert_eq!(merged, 1);
    }

    #[test]
    fn classic_buckets_units_and_recorders_are_independent() {
        let first = ObservabilityRuntime::new(true);
        let second = ObservabilityRuntime::new(true);
        first.record_duration_ms("p1.ack_duration_ms", "p1", 25.0);
        first.duration(
            "cvm_http_body_duration_seconds",
            &[("route", "/v1/responses"), ("method", "POST")],
            Duration::from_millis(30),
        );
        let text = first.render();
        assert!(text.contains("# TYPE cvm_sqlite_batch_ack_duration_seconds histogram"));
        assert!(
            text.contains("cvm_sqlite_batch_ack_duration_seconds_sum{class=\"p1_terminal\"} 0.025")
        );
        assert!(text.contains("le=\"0.00001\""));
        assert!(text.contains("le=\"600\""));
        assert!(text.contains("le=\"+Inf\""));
        assert!(!text.contains("summary"));
        assert!(!second.render().contains("cvm_http_body"));
        let disabled = ObservabilityRuntime::new(false);
        disabled.record_duration_ms("p1.ack_duration_ms", "p1", 25.0);
        assert!(disabled.render().is_empty());
    }

    #[test]
    fn unknown_labels_and_retired_storage_metrics_are_dropped() {
        let metrics = ObservabilityRuntime::new(true);
        metrics.record_counter("p1.retry_count", "account-123", 1);
        metrics.record_gauge("storage.telemetry_db_bytes", "telemetry_db", 123.0);
        metrics.record_duration_ms("sse.publish_duration_ms", "current", 1.0);
        assert!(metrics.render().is_empty());
    }
}
