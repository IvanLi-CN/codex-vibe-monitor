use crate::*;
use metrics::{Key, Label, Metadata, Recorder};
use metrics_exporter_prometheus::{
    Matcher, PrometheusBuilder, PrometheusHandle, PrometheusRecorder,
};

mod browser;
mod config;
pub(crate) mod contention;
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
// Preserve the short-duration buckets and resolve maintenance budgets and long tails.
const MAINTENANCE_BUCKETS: &[f64] = &[
    0.00001, 0.00005, 0.0001, 0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.2, 0.25, 0.5, 1.0,
    2.5, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0, 300.0,
];
const REQUEST_HISTOGRAMS: &[&str] = &[
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
];
const MAINTENANCE_HISTOGRAMS: &[&str] = &[
    "cvm_retention_phase_duration_seconds",
    "cvm_maintenance_store_write_duration_seconds",
    "cvm_summary_response_duration_seconds",
    "cvm_stats_query_duration_seconds",
    "cvm_invocation_range_phase_duration_seconds",
];

const PROXY_ENDPOINT_COUNT: usize = 3;
const PROXY_PHASE_COUNT: usize = 10;
const PROXY_OUTCOME_COUNT: usize = 3;
const PROXY_DIRECTION_COUNT: usize = 2;
const PROXY_PHASES: [&str; PROXY_PHASE_COUNT] = [
    "total",
    "request_read",
    "request_parse",
    "connect",
    "first_byte",
    "stream",
    "response_parse",
    "persist",
    "transport_connect",
    "transport_head",
];
const PROXY_INVOCATION_OUTCOMES: [&str; PROXY_OUTCOME_COUNT] = ["success", "error", "cancelled"];
const PROXY_ATTEMPT_OUTCOMES: [&str; PROXY_OUTCOME_COUNT] = ["success", "rejected", "error"];
const PROXY_DIRECTIONS: [&str; PROXY_DIRECTION_COUNT] = ["upload", "download"];

fn proxy_invocation_outcome_index(outcome: &'static str) -> usize {
    match outcome {
        "success" => 0,
        "error" => 1,
        "cancelled" => 2,
        _ => panic!("unknown proxy invocation outcome: {outcome}"),
    }
}

fn proxy_attempt_outcome_index(outcome: &'static str) -> usize {
    match outcome {
        "success" => 0,
        "rejected" => 1,
        "error" => 2,
        _ => panic!("unknown proxy attempt outcome: {outcome}"),
    }
}

fn proxy_direction_index(direction: &'static str) -> usize {
    match direction {
        "upload" => 0,
        "download" => 1,
        _ => panic!("unknown proxy transfer direction: {direction}"),
    }
}

fn proxy_endpoint_index(endpoint: &'static str) -> usize {
    match endpoint {
        "responses" => 0,
        "chat_completions" => 1,
        _ => 2,
    }
}

fn proxy_phase_index(phase: &'static str) -> usize {
    match phase {
        "total" => 0,
        "request_read" => 1,
        "request_parse" => 2,
        "connect" => 3,
        "first_byte" => 4,
        "stream" => 5,
        "response_parse" => 6,
        "persist" => 7,
        "transport_connect" => 8,
        "transport_head" => 9,
        _ => panic!("unknown proxy phase: {phase}"),
    }
}

#[derive(Debug)]
pub(crate) struct ProxyMetricHandles {
    invocations: [std::sync::OnceLock<metrics::Counter>; PROXY_OUTCOME_COUNT],
    phases: [std::sync::OnceLock<metrics::Histogram>; PROXY_PHASE_COUNT],
    ttfb: std::sync::OnceLock<metrics::Histogram>,
    ttft: std::sync::OnceLock<metrics::Histogram>,
    stream: std::sync::OnceLock<metrics::Histogram>,
    attempts: [std::sync::OnceLock<metrics::Counter>; PROXY_OUTCOME_COUNT],
    transfer: [std::sync::OnceLock<metrics::Counter>; PROXY_DIRECTION_COUNT],
}

impl Default for ProxyMetricHandles {
    fn default() -> Self {
        Self {
            invocations: std::array::from_fn(|_| std::sync::OnceLock::new()),
            phases: std::array::from_fn(|_| std::sync::OnceLock::new()),
            ttfb: std::sync::OnceLock::new(),
            ttft: std::sync::OnceLock::new(),
            stream: std::sync::OnceLock::new(),
            attempts: std::array::from_fn(|_| std::sync::OnceLock::new()),
            transfer: std::array::from_fn(|_| std::sync::OnceLock::new()),
        }
    }
}

impl ProxyMetricHandles {
    fn invocation(
        &self,
        metrics: &ObservabilityRuntime,
        endpoint: &'static str,
        outcome: &'static str,
    ) -> &metrics::Counter {
        let index = proxy_invocation_outcome_index(outcome);
        self.invocations[index].get_or_init(|| {
            metrics.register_counter(ObservabilityRuntime::key(
                "cvm_proxy_invocations_total",
                &[("endpoint", endpoint), ("outcome", outcome)],
            ))
        })
    }

    fn phase(
        &self,
        metrics: &ObservabilityRuntime,
        endpoint: &'static str,
        phase: &'static str,
    ) -> &metrics::Histogram {
        self.phase_at(metrics, endpoint, proxy_phase_index(phase))
    }

    fn phase_at(
        &self,
        metrics: &ObservabilityRuntime,
        endpoint: &'static str,
        index: usize,
    ) -> &metrics::Histogram {
        let phase = PROXY_PHASES[index];
        self.phases[index].get_or_init(|| {
            metrics.register_histogram(ObservabilityRuntime::key(
                "cvm_proxy_phase_duration_seconds",
                &[("endpoint", endpoint), ("phase", phase)],
            ))
        })
    }

    fn ttfb(&self, metrics: &ObservabilityRuntime, endpoint: &'static str) -> &metrics::Histogram {
        self.ttfb.get_or_init(|| {
            metrics.register_histogram(ObservabilityRuntime::key(
                "cvm_proxy_ttfb_seconds",
                &[("endpoint", endpoint)],
            ))
        })
    }

    fn ttft(&self, metrics: &ObservabilityRuntime, endpoint: &'static str) -> &metrics::Histogram {
        self.ttft.get_or_init(|| {
            metrics.register_histogram(ObservabilityRuntime::key(
                "cvm_proxy_ttft_seconds",
                &[("endpoint", endpoint)],
            ))
        })
    }

    fn stream(
        &self,
        metrics: &ObservabilityRuntime,
        endpoint: &'static str,
    ) -> &metrics::Histogram {
        self.stream.get_or_init(|| {
            metrics.register_histogram(ObservabilityRuntime::key(
                "cvm_proxy_stream_duration_seconds",
                &[("endpoint", endpoint)],
            ))
        })
    }

    fn attempt(
        &self,
        metrics: &ObservabilityRuntime,
        endpoint: &'static str,
        outcome: &'static str,
    ) -> &metrics::Counter {
        let index = proxy_attempt_outcome_index(outcome);
        self.attempts[index].get_or_init(|| {
            metrics.register_counter(ObservabilityRuntime::key(
                "cvm_proxy_upstream_attempts_total",
                &[("endpoint", endpoint), ("outcome", outcome)],
            ))
        })
    }

    pub(crate) fn transfer(
        &self,
        metrics: &ObservabilityRuntime,
        endpoint: &'static str,
        direction: &'static str,
    ) -> &metrics::Counter {
        let index = proxy_direction_index(direction);
        self.transfer[index].get_or_init(|| {
            metrics.register_counter(ObservabilityRuntime::key(
                "cvm_proxy_transfer_bytes_total",
                &[("endpoint", endpoint), ("direction", direction)],
            ))
        })
    }
}

// Fixed production dimensions bypass the per-sample registry lookup and series admission path.
const CACHED_COUNTER_COUNT: usize = 19;
const CACHED_HISTOGRAM_COUNT: usize = 5;
const CACHED_GAUGE_COUNT: usize = 3;

fn cached_counter_index(id: &'static str, dimension: &'static str) -> Option<usize> {
    match (id, dimension) {
        ("p1.transfer_bytes", "p1") => Some(0),
        ("p1.retry_count", "p1") => Some(1),
        ("p2.flush_attempt_count", "p2") => Some(2),
        ("p2.pressure_defer_count", "p2") => Some(3),
        ("p2.retry_count", "p2") => Some(4),
        ("p2.lock_retry_count", "p2") => Some(5),
        ("projection.reconcile_count", "dashboard") => Some(6),
        ("projection.reconcile_defer_count", "writer_pressure") => Some(7),
        ("projection.reconcile_defer_count", "background_busy") => Some(8),
        ("projection.reconcile_failure_count", "dashboard") => Some(9),
        ("sse.frame_bytes", "current") => Some(10),
        ("sse.frame_bytes", "network") => Some(11),
        ("sse.frame_bytes", "terminal") => Some(12),
        ("sse.publish_error_count", "current") => Some(13),
        ("sse.publish_error_count", "network") => Some(14),
        ("sse.publish_error_count", "terminal") => Some(15),
        ("projection.publish_count", "current") => Some(16),
        ("projection.publish_count", "network") => Some(17),
        ("projection.publish_count", "terminal") => Some(18),
        _ => None,
    }
}

fn cached_histogram_index(id: &'static str, dimension: &'static str) -> Option<usize> {
    match (id, dimension) {
        ("p1.ack_duration_ms", "p1") => Some(0),
        ("projection.reconcile_duration_ms", "dashboard") => Some(1),
        ("projection.publish_duration_ms", "current") => Some(2),
        ("projection.publish_duration_ms", "network") => Some(3),
        ("projection.publish_duration_ms", "terminal") => Some(4),
        _ => None,
    }
}

fn cached_gauge_index(id: &'static str, dimension: &'static str) -> Option<usize> {
    match (id, dimension) {
        ("projection.snapshot_bytes", "current") => Some(0),
        ("projection.snapshot_bytes", "network") => Some(1),
        ("projection.snapshot_bytes", "terminal") => Some(2),
        _ => None,
    }
}

struct MappedHistogramHandle {
    histogram: metrics::Histogram,
    scale: f64,
}

fn histogram_buckets(name: &str) -> &'static [f64] {
    if MAINTENANCE_HISTOGRAMS.contains(&name) {
        MAINTENANCE_BUCKETS
    } else if REQUEST_HISTOGRAMS.contains(&name) {
        REQUEST_BUCKETS
    } else {
        SHORT_BUCKETS
    }
}

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
    proxy_metrics: [std::sync::OnceLock<Arc<ProxyMetricHandles>>; PROXY_ENDPOINT_COUNT],
    cached_counters: [std::sync::OnceLock<metrics::Counter>; CACHED_COUNTER_COUNT],
    cached_histograms: [std::sync::OnceLock<MappedHistogramHandle>; CACHED_HISTOGRAM_COUNT],
    cached_gauges: [std::sync::OnceLock<metrics::Gauge>; CACHED_GAUGE_COUNT],
    series: limits::SeriesBudget,
    contention: contention::ContentionMetrics,
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
        for name in REQUEST_HISTOGRAMS.iter().chain(MAINTENANCE_HISTOGRAMS) {
            builder = builder
                .set_buckets_for_metric(Matcher::Full((*name).into()), histogram_buckets(name))
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
            proxy_metrics: std::array::from_fn(|_| std::sync::OnceLock::new()),
            cached_counters: std::array::from_fn(|_| std::sync::OnceLock::new()),
            cached_histograms: std::array::from_fn(|_| std::sync::OnceLock::new()),
            cached_gauges: std::array::from_fn(|_| std::sync::OnceLock::new()),
            series: limits::SeriesBudget::default(),
            contention: contention::ContentionMetrics::default(),
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
    fn cached_counter(
        &self,
        index: usize,
        id: &'static str,
        dimension: &'static str,
    ) -> &metrics::Counter {
        self.cached_counters[index].get_or_init(|| {
            let Some((name, labels, _)) = registry::mapped_metric(id, dimension) else {
                return metrics::Counter::noop();
            };
            self.register_counter(Key::from_parts(name, labels))
        })
    }
    fn cached_histogram(
        &self,
        index: usize,
        id: &'static str,
        dimension: &'static str,
    ) -> &MappedHistogramHandle {
        self.cached_histograms[index].get_or_init(|| {
            let Some((name, labels, scale)) = registry::mapped_metric(id, dimension) else {
                return MappedHistogramHandle {
                    histogram: metrics::Histogram::noop(),
                    scale: 1.0,
                };
            };
            MappedHistogramHandle {
                histogram: self.register_histogram(Key::from_parts(name, labels)),
                scale,
            }
        })
    }
    fn cached_gauge(
        &self,
        index: usize,
        id: &'static str,
        dimension: &'static str,
    ) -> &metrics::Gauge {
        self.cached_gauges[index].get_or_init(|| {
            let Some((name, labels, _)) = registry::mapped_metric(id, dimension) else {
                return metrics::Gauge::noop();
            };
            self.register_gauge(Key::from_parts(name, labels))
        })
    }
    pub(crate) fn proxy_metrics(&self, endpoint: &'static str) -> Arc<ProxyMetricHandles> {
        let index = proxy_endpoint_index(endpoint);
        self.proxy_metrics[index]
            .get_or_init(|| Arc::new(ProxyMetricHandles::default()))
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
        if let Some(index) = cached_counter_index(id, dimension) {
            self.cached_counter(index, id, dimension).increment(value);
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
        if let Some(index) = cached_histogram_index(id, dimension) {
            let handle = self.cached_histogram(index, id, dimension);
            handle.histogram.record(value * handle.scale);
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
        if let Some(index) = cached_gauge_index(id, dimension) {
            self.cached_gauge(index, id, dimension).set(value);
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
    pub(crate) fn runtime(&self) -> Option<Arc<ObservabilityRuntime>> {
        self.0.get().cloned()
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
        let handles = self.proxy_metrics(endpoint);
        let outcome = match record.status.as_str() {
            "success" => "success",
            "cancelled" | "canceled" => "cancelled",
            _ => "error",
        };
        handles.invocation(self, endpoint, outcome).increment(1);
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
                handles.phase(self, endpoint, phase).record(value / 1000.0);
            }
        }
        if t.t_upstream_ttfb_ms.is_finite() && t.t_upstream_ttfb_ms > 0.0 {
            handles
                .ttfb(self, endpoint)
                .record(t.t_upstream_ttfb_ms / 1000.0);
        }
        if let Some(ttft) = t
            .first_token_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            handles.ttft(self, endpoint).record(ttft / 1000.0);
        }
        if t.t_upstream_stream_ms.is_finite() && t.t_upstream_stream_ms > 0.0 {
            handles
                .stream(self, endpoint)
                .record(t.t_upstream_stream_ms / 1000.0);
        }
    }
}
pub(crate) struct ObservationSpan {
    histogram: metrics::Histogram,
    started: Instant,
}
impl ObservationSpan {
    pub(crate) fn new(
        metrics: Arc<ObservabilityRuntime>,
        endpoint: &'static str,
        phase: &'static str,
    ) -> Self {
        Self {
            histogram: metrics
                .proxy_metrics(endpoint)
                .phase(&metrics, endpoint, phase)
                .clone(),
            started: Instant::now(),
        }
    }
}
impl Drop for ObservationSpan {
    fn drop(&mut self) {
        self.histogram.record(self.started.elapsed().as_secs_f64());
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
    handles: Arc<ProxyMetricHandles>,
    endpoint: &'static str,
    pub(crate) outcome: &'static str,
}
impl UpstreamAttempt {
    pub(crate) fn new(metrics: Arc<ObservabilityRuntime>, endpoint: &'static str) -> Self {
        let handles = metrics.proxy_metrics(endpoint);
        Self {
            metrics,
            handles,
            endpoint,
            outcome: "error",
        }
    }
}
impl Drop for UpstreamAttempt {
    fn drop(&mut self) {
        self.handles
            .attempt(&self.metrics, self.endpoint, self.outcome)
            .increment(1);
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
    fn cached_fixed_dimensions_preserve_metric_mapping() {
        let metrics = ObservabilityRuntime::new(true);
        metrics.record_counter("p1.retry_count", "p1", 2);
        metrics.record_gauge("projection.snapshot_bytes", "network", 42.0);
        metrics.record_duration_ms("projection.publish_duration_ms", "terminal", 25.0);

        let text = metrics.render();
        assert!(
            text.contains("cvm_sqlite_retries_total{class=\"p1_terminal\",reason=\"other\"} 2")
        );
        assert!(text.contains("cvm_projection_snapshot_bytes{slice=\"network\"} 42"));
        assert!(
            text.contains("cvm_projection_publish_duration_seconds_sum{slice=\"terminal\"} 0.025")
        );
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
