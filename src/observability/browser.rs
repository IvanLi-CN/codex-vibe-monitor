use crate::*;

pub(crate) const BROWSER_MAX_BYTES: usize = 2048;

pub(crate) fn browser_ingest_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/system/observability/browser",
            post(ingest_browser_observations)
                .layer(DefaultBodyLimit::max(BROWSER_MAX_BYTES))
                .layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    browser_ingest_rate_limit,
                )),
        )
        .with_state(state)
}

#[derive(Debug)]
pub(super) struct BrowserLimiter {
    started: Instant,
    total: u32,
    clients: HashMap<String, u32>,
}

impl Default for BrowserLimiter {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            total: 0,
            clients: HashMap::new(),
        }
    }
}
impl BrowserLimiter {
    fn allow(&mut self, client: &str) -> bool {
        if self.started.elapsed() >= Duration::from_secs(60) {
            self.started = Instant::now();
            self.total = 0;
            self.clients.clear();
        }
        let client: String = client.chars().take(128).collect();
        if self.total >= 120 || self.clients.get(&client).copied().unwrap_or_default() >= 30 {
            return false;
        }
        self.total += 1;
        *self.clients.entry(client).or_default() += 1;
        true
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Page {
    Dashboard,
    Records,
    System,
}
impl Page {
    fn label(&self) -> &'static str {
        match self {
            Self::Dashboard => "dashboard",
            Self::Records => "records",
            Self::System => "system",
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Device {
    Mobile,
    Desktop,
}
impl Device {
    fn label(&self) -> &'static str {
        match self {
            Self::Mobile => "mobile",
            Self::Desktop => "desktop",
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    DataReady,
    UpdateToPaint,
    LongTask,
    ApiRequest,
    Sse,
    Unsupported,
    Hidden,
    Dropped,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Normal,
    Error,
    Unknown,
}
impl Outcome {
    fn label(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Error => "error",
            Self::Unknown => "unknown",
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Event {
    page: Page,
    device: Device,
    kind: Kind,
    value_seconds: f64,
    outcome: Option<Outcome>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BrowserRequest {
    events: Vec<Event>,
}
async fn browser_ingest_rate_limit(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    let metrics = &state.observability;
    // The reverse proxy owns client identity. It is used only in this bounded limiter, never as a label.
    let client = request
        .headers()
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");
    if !metrics
        .browser_limiter
        .lock()
        .map(|mut limiter| limiter.allow(client))
        .unwrap_or(false)
    {
        metrics.counter(
            "cvm_browser_ingest_total",
            &[("outcome", "rejected"), ("reason", "rate")],
            1,
        );
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    next.run(request).await
}

async fn ingest_browser_observations(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<BrowserRequest>,
) -> Response {
    let metrics = &state.observability;
    if !is_same_origin_settings_write(&headers) {
        metrics.counter(
            "cvm_browser_ingest_total",
            &[("outcome", "rejected"), ("reason", "origin")],
            1,
        );
        return StatusCode::FORBIDDEN.into_response();
    }
    if payload.events.len() > 8
        || payload.events.iter().any(|e| {
            !e.value_seconds.is_finite()
                || !(0.0..=3600.0).contains(&e.value_seconds)
                || (matches!(e.kind, Kind::Sse) != e.outcome.is_some())
        })
    {
        metrics.counter(
            "cvm_browser_ingest_total",
            &[("outcome", "rejected"), ("reason", "validation")],
            1,
        );
        return StatusCode::BAD_REQUEST.into_response();
    }
    for event in payload.events {
        let page = event.page.label();
        let labels = &[("page", page)];
        let duration = Duration::from_secs_f64(event.value_seconds);
        match event.kind {
            Kind::DataReady => metrics.duration(
                "cvm_browser_data_ready_seconds",
                &[("page", page), ("device", event.device.label())],
                duration,
            ),
            Kind::UpdateToPaint => metrics.duration(
                "cvm_browser_update_to_paint_seconds",
                &[("page", page), ("device", event.device.label())],
                duration,
            ),
            Kind::LongTask => {
                metrics.duration("cvm_browser_long_task_duration_seconds", labels, duration);
                metrics.counter(
                    "cvm_browser_events_total",
                    &[("page", page), ("event", "long_task")],
                    1,
                );
            }
            Kind::ApiRequest => {
                metrics.duration("cvm_browser_api_duration_seconds", labels, duration);
                metrics.counter(
                    "cvm_browser_events_total",
                    &[("page", page), ("event", "api_request")],
                    1,
                );
            }
            Kind::Sse => {
                metrics.duration("cvm_browser_sse_duration_seconds", labels, duration);
                metrics.counter(
                    "cvm_browser_sse_ends_total",
                    &[
                        ("page", page),
                        (
                            "outcome",
                            event
                                .outcome
                                .as_ref()
                                .map(Outcome::label)
                                .unwrap_or("unknown"),
                        ),
                    ],
                    1,
                );
            }
            Kind::Unsupported => metrics.counter("cvm_browser_unsupported_total", labels, 1),
            Kind::Hidden => metrics.counter("cvm_browser_visibility_hidden_total", labels, 1),
            Kind::Dropped => metrics.counter("cvm_browser_dropped_total", labels, 1),
        }
    }
    metrics.counter(
        "cvm_browser_ingest_total",
        &[("outcome", "accepted"), ("reason", "none")],
        1,
    );
    StatusCode::NO_CONTENT.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limiter_enforces_client_and_global_budgets_with_bounded_state() {
        let mut limiter = BrowserLimiter::default();
        for _ in 0..30 {
            assert!(limiter.allow("client-a"));
        }
        assert!(!limiter.allow("client-a"));
        for index in 0..90 {
            assert!(limiter.allow(&format!("client-{index}")));
        }
        for index in 0..1000 {
            assert!(!limiter.allow(&format!("rejected-{index}")));
        }
        assert_eq!(limiter.clients.len(), 91);
        limiter.started = Instant::now() - Duration::from_secs(61);
        assert!(limiter.allow("fresh"));
        assert_eq!(limiter.clients.len(), 1);
    }

    #[test]
    fn browser_schema_rejects_arbitrary_labels_and_unknown_fields() {
        for event in [
            json!({"page":"customer-123","device":"desktop","kind":"api_request","valueSeconds":0.1}),
            json!({"page":"dashboard","device":"desktop","kind":"api_request","valueSeconds":0.1,"account":"secret"}),
        ] {
            assert!(serde_json::from_value::<BrowserRequest>(json!({"events":[event]})).is_err());
        }
    }
}
