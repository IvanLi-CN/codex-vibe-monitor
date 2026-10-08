use crate::*;
use http_body::{Body as HttpBody, Frame, SizeHint};
use std::pin::Pin;
use std::task::{Context, Poll};

pub(crate) fn http_request_trace_path(request: &Request<Body>) -> &str {
    request.uri().path()
}

pub(crate) async fn retired_performance_preflight(
    request: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    let retired = request.method() == Method::OPTIONS
        && matches!(
            request.uri().path(),
            "/api/system/performance"
                | "/api/system/performance/health"
                | "/api/system/performance/browser"
        );
    let response = next.run(request).await;
    if !retired {
        return response;
    }
    // CORS answers OPTIONS without entering the route. Preserve its policy headers
    // while returning the same static tombstone as the other retired API methods.
    let mut tombstone = super::performance_retired().await;
    for (name, value) in response.headers() {
        if name.as_str().starts_with("access-control-") || name.as_str() == "vary" {
            tombstone.headers_mut().append(name.clone(), value.clone());
        }
    }
    tombstone
}

pub(super) fn route_family(path: &str) -> &'static str {
    match path {
        "/v1/responses" => "/v1/responses",
        "/v1/chat/completions" => "/v1/chat/completions",
        "/health" => "/health",
        "/api/system/observability" | "/api/system/observability/browser" => {
            "/api/system/observability"
        }
        p if p.starts_with("/api/system/observability/hotpath/") => {
            "/api/system/observability/hotpath/{report}"
        }
        p if p.starts_with("/api/system/") => "/api/system/{resource}",
        p if p.starts_with("/api/invocations") => "/api/invocations/{resource}",
        p if p.starts_with("/api/summary") => "/api/summary/{resource}",
        p if p.starts_with("/api/") => "/api/{resource}",
        _ => "<static-or-unmatched>",
    }
}
pub(super) const HTTP_ROUTE_COUNT: usize = 10;
pub(super) const HTTP_METHOD_COUNT: usize = 8;
pub(super) fn route_index(route: &'static str) -> usize {
    match route {
        "/v1/responses" => 0,
        "/v1/chat/completions" => 1,
        "/health" => 2,
        "/api/system/observability" => 3,
        "/api/system/observability/hotpath/{report}" => 4,
        "/api/system/{resource}" => 5,
        "/api/invocations/{resource}" => 6,
        "/api/summary/{resource}" => 7,
        "/api/{resource}" => 8,
        _ => 9,
    }
}
fn method_family(method: &Method) -> &'static str {
    match method.as_str() {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "PATCH" => "PATCH",
        "DELETE" => "DELETE",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        _ => "other",
    }
}
pub(super) fn method_index(method: &'static str) -> usize {
    match method {
        "GET" => 0,
        "POST" => 1,
        "PUT" => 2,
        "PATCH" => 3,
        "DELETE" => 4,
        "HEAD" => 5,
        "OPTIONS" => 6,
        _ => 7,
    }
}
pub(super) struct HttpMetricHandles {
    route: &'static str,
    method: &'static str,
    header_duration: std::sync::OnceLock<metrics::Histogram>,
    body_duration: std::sync::OnceLock<metrics::Histogram>,
    requests: [std::sync::OnceLock<metrics::Counter>; 5],
    body_ends: [std::sync::OnceLock<metrics::Counter>; 3],
}
impl HttpMetricHandles {
    pub(super) fn new(route: &'static str, method: &'static str) -> Self {
        Self {
            route,
            method,
            header_duration: std::sync::OnceLock::new(),
            body_duration: std::sync::OnceLock::new(),
            requests: std::array::from_fn(|_| std::sync::OnceLock::new()),
            body_ends: std::array::from_fn(|_| std::sync::OnceLock::new()),
        }
    }
    fn header_duration(&self, metrics: &ObservabilityRuntime) -> &metrics::Histogram {
        self.header_duration.get_or_init(|| {
            if metrics.enabled {
                metrics.register_histogram(ObservabilityRuntime::key(
                    "cvm_http_header_duration_seconds",
                    &[("route", self.route), ("method", self.method)],
                ))
            } else {
                metrics::Histogram::noop()
            }
        })
    }
    fn body_duration(&self, metrics: &ObservabilityRuntime) -> &metrics::Histogram {
        self.body_duration.get_or_init(|| {
            if metrics.enabled {
                metrics.register_histogram(ObservabilityRuntime::key(
                    "cvm_http_body_duration_seconds",
                    &[("route", self.route), ("method", self.method)],
                ))
            } else {
                metrics::Histogram::noop()
            }
        })
    }
    fn request(
        &self,
        metrics: &ObservabilityRuntime,
        status_class: &'static str,
    ) -> Option<&metrics::Counter> {
        ["1xx", "2xx", "3xx", "4xx", "5xx"]
            .iter()
            .position(|candidate| *candidate == status_class)
            .map(|index| {
                self.requests[index].get_or_init(|| {
                    if metrics.enabled {
                        metrics.register_counter(ObservabilityRuntime::key(
                            "cvm_http_requests_total",
                            &[
                                ("route", self.route),
                                ("method", self.method),
                                ("status_class", status_class),
                            ],
                        ))
                    } else {
                        metrics::Counter::noop()
                    }
                })
            })
    }
    fn body_end(
        &self,
        metrics: &ObservabilityRuntime,
        outcome: &'static str,
    ) -> Option<&metrics::Counter> {
        ["complete", "error", "cancelled"]
            .iter()
            .position(|candidate| *candidate == outcome)
            .map(|index| {
                self.body_ends[index].get_or_init(|| {
                    if metrics.enabled {
                        metrics.register_counter(ObservabilityRuntime::key(
                            "cvm_http_body_ends_total",
                            &[("route", self.route), ("outcome", outcome)],
                        ))
                    } else {
                        metrics::Counter::noop()
                    }
                })
            })
    }
}
struct RequestLifetime {
    metrics: Arc<ObservabilityRuntime>,
    http: Arc<HttpMetricHandles>,
    route: &'static str,
    method: &'static str,
    started: Instant,
    finished: bool,
    diagnostic: Option<super::diagnostics::DiagnosticContext>,
    status_class: &'static str,
}
impl RequestLifetime {
    fn finish(&mut self, outcome: &'static str) {
        if self.finished {
            return;
        }
        self.finished = true;
        if let Some(context) = &self.diagnostic {
            context.finish_response(outcome, self.status_class);
        }
        self.http
            .body_duration(&self.metrics)
            .record(self.started.elapsed().as_secs_f64());
        if let Some(counter) = self.http.body_end(&self.metrics, outcome) {
            counter.increment(1);
        } else {
            self.metrics.counter(
                "cvm_http_body_ends_total",
                &[("route", self.route), ("outcome", outcome)],
                1,
            );
        }
        self.metrics.http_inflight().decrement(1.0);
    }
}
impl Drop for RequestLifetime {
    fn drop(&mut self) {
        self.finish("cancelled");
    }
}
struct ObservedBody {
    inner: Body,
    lifetime: RequestLifetime,
}
impl HttpBody for ObservedBody {
    type Data = axum::body::Bytes;
    type Error = axum::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let result = Pin::new(&mut self.inner).poll_frame(cx);
        if let Poll::Ready(Some(Ok(frame))) = &result
            && frame.data_ref().is_some_and(|data| !data.is_empty())
            && let Some(context) = &self.lifetime.diagnostic
        {
            context.milestone("first_byte");
        }
        match &result {
            Poll::Ready(None) => self.lifetime.finish("complete"),
            Poll::Ready(Some(Err(_))) => self.lifetime.finish("error"),
            _ => {}
        }
        if matches!(&result, Poll::Ready(Some(Ok(_)))) && self.inner.is_end_stream() {
            self.lifetime.finish("complete");
        }
        result
    }
    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}
pub(crate) async fn observability_http_middleware(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    let metrics = state.observability.clone();
    if !metrics.enabled || request.uri().path() == "/api/system/observability/browser" {
        return next.run(request).await;
    }
    let route = route_family(request.uri().path());
    let method = method_family(request.method());
    let http = metrics.http_metrics(route, method);
    let started = Instant::now();
    metrics.http_inflight().increment(1.0);
    let mut lifetime = RequestLifetime {
        metrics: metrics.clone(),
        http: http.clone(),
        route,
        method,
        started,
        finished: false,
        diagnostic: request
            .uri()
            .path()
            .starts_with("/v1/")
            .then(|| {
                super::diagnostics::DiagnosticContext::begin(
                    metrics.clone(),
                    super::diagnostics::endpoint(request.uri().path()),
                )
            })
            .flatten(),
        status_class: "unknown",
    };
    if request.method() != Method::POST
        && let Some(context) = &lifetime.diagnostic
    {
        context.ttft_applicable(false);
    }
    let response = super::diagnostics::scope(lifetime.diagnostic.clone(), next.run(request)).await;
    let class = match response.status().as_u16() / 100 {
        1 => "1xx",
        2 => "2xx",
        3 => "3xx",
        4 => "4xx",
        _ => "5xx",
    };
    lifetime.status_class = class;
    if let Some(context) = &lifetime.diagnostic {
        context.milestone("head");
    }
    if let Some(counter) = http.request(&metrics, class) {
        counter.increment(1);
    } else {
        metrics.counter(
            "cvm_http_requests_total",
            &[
                ("route", route),
                ("method", method),
                ("status_class", class),
            ],
            1,
        );
    }
    http.header_duration(&metrics)
        .record(started.elapsed().as_secs_f64());
    let (parts, body) = response.into_parts();
    if body.is_end_stream() {
        lifetime.finish("complete");
    }
    Response::from_parts(
        parts,
        Body::new(ObservedBody {
            inner: body,
            lifetime,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_request_trace_path_excludes_query_parameters() {
        let request = Request::builder()
            .uri("/v1/responses?api_key=secret-value")
            .body(Body::empty())
            .expect("build request with sensitive query");

        assert_eq!(http_request_trace_path(&request), "/v1/responses");
    }

    fn assert_sample(rendered: &str, name: &str, labels: &[&str], value: &str) {
        let matched = rendered
            .lines()
            .filter(|line| {
                line.starts_with(&format!("{name}{{"))
                    && line.ends_with(&format!("}} {value}"))
                    && labels.iter().all(|label| line.contains(label))
            })
            .count();
        assert_eq!(matched, 1, "expected one {name} sample in {rendered}");
    }

    fn body(metrics: Arc<ObservabilityRuntime>, body: Body) -> ObservedBody {
        metrics.http_inflight().increment(1.0);
        let http = metrics.http_metrics("/v1/responses", "POST");
        ObservedBody {
            inner: body,
            lifetime: RequestLifetime {
                diagnostic: super::super::diagnostics::DiagnosticContext::begin(
                    metrics.clone(),
                    "responses",
                ),
                http,
                metrics,
                route: "/v1/responses",
                method: "POST",
                started: Instant::now(),
                finished: false,
                status_class: "2xx",
            },
        }
    }

    #[tokio::test]
    async fn streaming_inflight_survives_headers_and_finishes_once() {
        let metrics = ObservabilityRuntime::new(true);
        let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Bytes, io::Error>>(1);
        let stream = futures_util::stream::unfold(receiver, |mut receiver| async {
            receiver.recv().await.map(|value| (value, receiver))
        });
        let mut observed = body(metrics.clone(), Body::from_stream(stream));
        assert!(metrics.render().contains("cvm_http_inflight 1"));
        sender
            .send(Ok(Bytes::from_static(b"data: delta\n\n")))
            .await
            .unwrap();
        assert!(
            futures_util::future::poll_fn(|cx| Pin::new(&mut observed).poll_frame(cx))
                .await
                .unwrap()
                .is_ok()
        );
        assert!(metrics.render().contains("cvm_http_inflight 1"));
        drop(sender);
        assert!(
            futures_util::future::poll_fn(|cx| Pin::new(&mut observed).poll_frame(cx))
                .await
                .is_none()
        );
        drop(observed);
        let rendered = metrics.render();
        assert!(rendered.contains("cvm_http_inflight 0"));
        assert!(rendered.contains("outcome=\"complete\""));
        assert!(!rendered.contains("outcome=\"cancelled\""));
        assert_sample(
            &rendered,
            "cvm_http_body_duration_seconds_count",
            &["method=\"POST\"", "route=\"/v1/responses\""],
            "1",
        );
        assert_sample(
            &rendered,
            "cvm_request_response_ends_total",
            &["endpoint=\"responses\"", "outcome=\"complete\""],
            "1",
        );
    }

    #[tokio::test]
    async fn stream_error_and_cancel_are_distinct_terminal_outcomes() {
        let metrics = ObservabilityRuntime::new(true);
        let mut failed = body(
            metrics.clone(),
            Body::from_stream(futures_util::stream::once(async {
                Err::<Bytes, _>(io::Error::other("stream failed"))
            })),
        );
        assert!(
            futures_util::future::poll_fn(|cx| Pin::new(&mut failed).poll_frame(cx))
                .await
                .unwrap()
                .is_err()
        );
        drop(failed);
        drop(body(
            metrics.clone(),
            Body::from_stream(futures_util::stream::pending::<Result<Bytes, io::Error>>()),
        ));
        let rendered = metrics.render();
        for outcome in ["error", "cancelled"] {
            assert_sample(
                &rendered,
                "cvm_http_body_ends_total",
                &[&format!("outcome=\"{outcome}\""), "route=\"/v1/responses\""],
                "1",
            );
            assert_sample(
                &rendered,
                "cvm_request_response_ends_total",
                &[&format!("outcome=\"{outcome}\""), "endpoint=\"responses\""],
                "1",
            );
        }
        assert!(rendered.contains("cvm_http_inflight 0"));
    }
}
