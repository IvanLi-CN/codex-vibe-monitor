use crate::*;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::{WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{
    BatchConfigBuilder, BatchSpanProcessor, SdkTracer, SdkTracerProvider, Span, SpanData,
    SpanExporter, SpanLimits, SpanProcessor,
};
use std::sync::atomic::AtomicUsize;

pub(super) const QUEUE_LIMIT: usize = 2048;
const EXPORT_BATCH_LIMIT: usize = 128;
// Keep one export batch of headroom so the SDK's bounded channel cannot reject
// a span after the local admission counter has reserved it.
pub(super) const ADMISSION_LIMIT: usize = QUEUE_LIMIT - EXPORT_BATCH_LIMIT;
pub(super) const ACTIVE_LIMIT: usize = 1024;

#[derive(Clone)]
pub(crate) struct TraceConfig {
    pub(crate) enabled: bool,
    pub(crate) valid: bool,
    endpoint: Option<Url>,
    token: Option<Arc<str>>,
    environment: String,
    instance: String,
}
impl std::fmt::Debug for TraceConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceConfig")
            .field("enabled", &self.enabled)
            .field("valid", &self.valid)
            .finish_non_exhaustive()
    }
}
impl Default for TraceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            valid: true,
            endpoint: None,
            token: None,
            environment: "unknown".into(),
            instance: "unknown".into(),
        }
    }
}
impl TraceConfig {
    pub(crate) fn from_env(master_enabled: bool) -> Self {
        let requested = parse_bool_env_var("OBSERVABILITY_TRACES_ENABLED", false);
        let enabled = trace_enabled(master_enabled, &requested);
        let mut result = Self {
            enabled,
            ..Self::default()
        };
        if !enabled {
            return result;
        }
        let loaded: Result<()> = (|| {
            requested?;
            let endpoint = Url::parse(&env::var("OBSERVABILITY_OTLP_TRACES_ENDPOINT")?)?;
            if !valid_endpoint(&endpoint) {
                bail!("invalid trace endpoint");
            }
            let path = env::var("OBSERVABILITY_OTLP_TOKEN_FILE")?;
            let metadata = std::fs::metadata(&path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if !token_file_mode_allowed(metadata.permissions().mode()) {
                    bail!(
                        "trace token file permissions must allow owner access and optional group read only"
                    );
                }
            }
            if !metadata.is_file() || metadata.len() > 4096 {
                bail!("invalid trace token file");
            }
            let bytes = std::fs::read(path)?;
            let token = std::str::from_utf8(&bytes)?.trim();
            if token.len() < 16 || !token.bytes().all(|b| b.is_ascii_graphic()) {
                bail!("invalid trace token");
            }
            result.environment = resource_label("OBSERVABILITY_ENVIRONMENT")?;
            result.instance = resource_label("OBSERVABILITY_INSTANCE")?;
            result.endpoint = Some(endpoint);
            result.token = Some(Arc::from(token));
            Ok(())
        })();
        // Do not log URLs, private paths or errors containing credential values.
        result.valid = loaded.is_ok();
        result
    }
}
fn trace_enabled(master_enabled: bool, requested: &Result<bool>) -> bool {
    // Missing configuration is Ok(false); an invalid value stays enabled so
    // startup reports tracing as degraded instead of silently disabling it.
    master_enabled
        && match requested {
            Ok(value) => *value,
            Err(_) => true,
        }
}
#[cfg(unix)]
fn token_file_mode_allowed(mode: u32) -> bool {
    // Deployment uses 0640 for non-root containers: group read is allowed, but
    // group write/execute and every other-user bit are rejected.
    mode & 0o400 != 0 && mode & 0o137 == 0
}
fn resource_label(name: &str) -> Result<String> {
    let value = env::var(name).unwrap_or_else(|_| "unknown".into());
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
    {
        bail!("invalid trace resource label");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_configuration_defaults_to_disabled() {
        assert!(!trace_enabled(true, &Ok(false)));
        assert!(trace_enabled(true, &Ok(true)));
        assert!(!trace_enabled(false, &Ok(true)));
        assert!(trace_enabled(true, &Err(anyhow!("invalid trace flag"))));
    }

    #[cfg(unix)]
    #[test]
    fn trace_token_permissions_allow_private_group_read_only() {
        assert!(token_file_mode_allowed(0o600));
        assert!(token_file_mode_allowed(0o640));
        assert!(token_file_mode_allowed(0o400));
        assert!(!token_file_mode_allowed(0o000));
        assert!(!token_file_mode_allowed(0o200));
        assert!(!token_file_mode_allowed(0o660));
        assert!(!token_file_mode_allowed(0o644));
        assert!(!token_file_mode_allowed(0o700));
    }
}
fn valid_endpoint(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/v1/traces"
        && url.query().is_none()
        && url.fragment().is_none()
}

#[derive(Debug, Default)]
pub(super) struct ExportState {
    queued: AtomicUsize,
    pub(super) active: AtomicUsize,
    stopped: AtomicBool,
    pub(super) degraded: AtomicBool,
    queue_drops: AtomicU64,
    failed_spans: AtomicU64,
    exported_spans: AtomicU64,
}
pub(crate) struct TraceRuntime {
    pub(crate) enabled: bool,
    pub(super) state: Arc<ExportState>,
    provider: Option<SdkTracerProvider>,
    pub(super) tracer: Option<SdkTracer>,
}
impl std::fmt::Debug for TraceRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TraceRuntime")
            .field("state", &self.state())
            .finish()
    }
}
impl TraceRuntime {
    pub(crate) fn disabled() -> Arc<Self> {
        Arc::new(Self {
            enabled: false,
            state: Arc::default(),
            provider: None,
            tracer: None,
        })
    }
    // Called on a blocking startup worker; reqwest's blocking runtime never runs on Tokio.
    pub(crate) fn new(config: &TraceConfig) -> Arc<Self> {
        let state = Arc::new(ExportState::default());
        let provider = if config.enabled && config.valid {
            build_provider(config, state.clone()).ok()
        } else {
            None
        };
        state
            .degraded
            .store(config.enabled && provider.is_none(), Ordering::Relaxed);
        let tracer = provider
            .as_ref()
            .map(|p| p.tracer("cvm.request_diagnostics"));
        Arc::new(Self {
            enabled: config.enabled,
            state,
            provider,
            tracer,
        })
    }
    pub(crate) fn state(&self) -> &'static str {
        if !self.enabled {
            "disabled"
        } else if self.state.degraded.load(Ordering::Relaxed) {
            "degraded"
        } else {
            "enabled"
        }
    }
    pub(crate) fn report(&self, metrics: &ObservabilityRuntime) {
        metrics.source_absolute(
            "cvm_trace_exported_spans_total",
            self.state.exported_spans.load(Ordering::Relaxed),
        );
        metrics.source_absolute(
            "cvm_trace_export_failed_spans_total",
            self.state.failed_spans.load(Ordering::Relaxed),
        );
        metrics.source_absolute(
            "cvm_trace_queue_dropped_spans_total",
            self.state.queue_drops.load(Ordering::Relaxed),
        );
        metrics.gauge(
            "cvm_trace_active_contexts",
            &[],
            self.state.active.load(Ordering::Relaxed) as f64,
        );
    }
    pub(crate) fn shutdown(&self) {
        self.state.stopped.store(true, Ordering::Release);
        if let Some(provider) = &self.provider {
            let _ = provider.shutdown_with_timeout(Duration::from_secs(2));
        }
    }
    #[cfg(test)]
    pub(super) fn for_test_batched(exporter: impl SpanExporter + 'static) -> Arc<Self> {
        Self::for_test_batched_with_config(exporter, QUEUE_LIMIT, Duration::from_secs(1))
    }
    #[cfg(test)]
    pub(super) fn for_test_batched_with_config(
        exporter: impl SpanExporter + 'static,
        max_queue_size: usize,
        scheduled_delay: Duration,
    ) -> Arc<Self> {
        let state = Arc::new(ExportState::default());
        let processor = BatchSpanProcessor::builder(CountedExporter {
            inner: exporter,
            state: state.clone(),
        })
        .with_batch_config(
            BatchConfigBuilder::default()
                .with_max_queue_size(max_queue_size)
                .with_max_export_batch_size(EXPORT_BATCH_LIMIT)
                .with_scheduled_delay(scheduled_delay)
                .build(),
        )
        .build();
        let provider = SdkTracerProvider::builder()
            .with_span_processor(CountedProcessor {
                inner: processor,
                state: state.clone(),
            })
            .with_resource(Resource::builder_empty().build())
            .build();
        let tracer = provider.tracer("cvm.request_diagnostics");
        Arc::new(Self {
            enabled: true,
            state,
            provider: Some(provider),
            tracer: Some(tracer),
        })
    }
    #[cfg(test)]
    pub(super) fn force_flush_for_test(&self) -> OTelSdkResult {
        self.provider
            .as_ref()
            .expect("test trace runtime provider")
            .force_flush()
    }
    #[cfg(test)]
    pub(super) fn for_test(exporter: impl SpanExporter + 'static) -> Arc<Self> {
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter)
            .with_resource(Resource::builder_empty().build())
            .build();
        let tracer = provider.tracer("cvm.request_diagnostics");
        Arc::new(Self {
            enabled: true,
            state: Arc::default(),
            provider: Some(provider),
            tracer: Some(tracer),
        })
    }
}
fn build_provider(config: &TraceConfig, state: Arc<ExportState>) -> Result<SdkTracerProvider> {
    let client = PrivateHttpClient {
        client: reqwest::blocking::Client::builder()
            .tls_built_in_native_certs(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()?,
        endpoint: config.endpoint.clone().context("trace endpoint missing")?,
        token: config.token.clone().context("trace credential missing")?,
    };
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
        .with_endpoint(client.endpoint.as_str())
        .with_timeout(Duration::from_secs(2))
        .with_retry_policy(opentelemetry_otlp::RetryPolicy::default().with_max_retries(0))
        .with_max_request_body_size(1024 * 1024)
        .with_http_client(client)
        .build()?;
    let exporter = CountedExporter {
        inner: exporter,
        state: state.clone(),
    };
    let batch = BatchSpanProcessor::builder(exporter)
        .with_batch_config(
            BatchConfigBuilder::default()
                .with_max_queue_size(QUEUE_LIMIT)
                .with_max_export_batch_size(EXPORT_BATCH_LIMIT)
                .with_scheduled_delay(Duration::from_secs(1))
                .build(),
        )
        .build();
    Ok(SdkTracerProvider::builder()
        .with_sampler(opentelemetry_sdk::trace::Sampler::AlwaysOn)
        .with_span_processor(CountedProcessor {
            inner: batch,
            state,
        })
        .with_span_limits(SpanLimits {
            max_attributes_per_span: 48,
            max_events_per_span: 0,
            max_links_per_span: 16,
            max_attributes_per_link: 0,
            max_attributes_per_event: 0,
        })
        .with_resource(
            Resource::builder_empty()
                .with_attributes([
                    opentelemetry::KeyValue::new("service.name", "codex-vibe-monitor"),
                    opentelemetry::KeyValue::new(
                        "deployment.environment.name",
                        config.environment.clone(),
                    ),
                    opentelemetry::KeyValue::new("service.instance.id", config.instance.clone()),
                ])
                .build(),
        )
        .build())
}

// The SDK can read OTEL_* headers/endpoints. Ignore those at the transport boundary:
// only the approved endpoint, protobuf and private bearer credential leave this process.
struct PrivateHttpClient {
    client: reqwest::blocking::Client,
    endpoint: Url,
    token: Arc<str>,
}
impl std::fmt::Debug for PrivateHttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PrivateHttpClient")
    }
}
#[async_trait::async_trait]
impl opentelemetry_http::HttpClient for PrivateHttpClient {
    async fn send_bytes(
        &self,
        request: opentelemetry_http::Request<opentelemetry_http::Bytes>,
    ) -> std::result::Result<
        opentelemetry_http::Response<opentelemetry_http::Bytes>,
        opentelemetry_http::HttpError,
    > {
        use std::io::Read;
        if request.body().len() > 1024 * 1024 || request.headers().contains_key("content-encoding")
        {
            return Err(io::Error::other("trace export body rejected").into());
        }
        let mut response = self
            .client
            .post(self.endpoint.clone())
            .bearer_auth(self.token.as_ref())
            .header("content-type", "application/x-protobuf")
            .body(request.into_body())
            .send()?;
        let status = response.status();
        let mut bytes = Vec::new();
        response
            .by_ref()
            .take(64 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 64 * 1024 {
            return Err(io::Error::other("trace export response rejected").into());
        }
        if status.is_success() {
            use prost::Message;
            let reply = opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceResponse::decode(bytes.as_slice())
                .map_err(|_| io::Error::other("invalid trace export reply"))?;
            if reply
                .partial_success
                .is_some_and(|partial| partial.rejected_spans > 0)
            {
                return Err(io::Error::other("trace export partially rejected").into());
            }
        }
        Ok(opentelemetry_http::Response::builder()
            .status(status)
            .body(bytes.into())?)
    }
}

#[derive(Debug)]
struct CountedExporter<E> {
    inner: E,
    state: Arc<ExportState>,
}
impl<E: SpanExporter> SpanExporter for CountedExporter<E> {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let count = batch.len() as u64;
        self.state
            .queued
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |queued| {
                Some(queued.saturating_sub(batch.len()))
            })
            .expect("queued counter update cannot fail");
        let result = self.inner.export(batch).await;
        if result.is_ok() {
            self.state
                .exported_spans
                .fetch_add(count, Ordering::Relaxed);
        } else {
            self.state.failed_spans.fetch_add(count, Ordering::Relaxed);
            self.state.degraded.store(true, Ordering::Relaxed);
        }
        result
    }
    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}
#[derive(Debug)]
struct CountedProcessor {
    inner: BatchSpanProcessor,
    state: Arc<ExportState>,
}
impl SpanProcessor for CountedProcessor {
    fn on_start(&self, span: &mut Span, cx: &opentelemetry::Context) {
        self.inner.on_start(span, cx);
    }
    fn on_end(&self, span: SpanData) {
        if self.state.stopped.load(Ordering::Acquire)
            || self
                .state
                .queued
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                    (n < ADMISSION_LIMIT).then_some(n + 1)
                })
                .is_err()
        {
            self.state.queue_drops.fetch_add(1, Ordering::Relaxed);
            self.state.degraded.store(true, Ordering::Relaxed);
            return;
        }
        self.inner.on_end(span);
    }
    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }
    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }
    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}
