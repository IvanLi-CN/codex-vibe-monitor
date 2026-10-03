use crate::*;
use http_body::{Body as HttpBody, Frame, SizeHint};
use metrics::Recorder;
use std::pin::Pin;
use std::task::{Context, Poll};

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
struct RequestLifetime {
    metrics: Arc<ObservabilityRuntime>,
    route: &'static str,
    method: &'static str,
    started: Instant,
    finished: bool,
}
impl RequestLifetime {
    fn finish(&mut self, outcome: &'static str) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.metrics.duration(
            "cvm_http_body_duration_seconds",
            &[("route", self.route), ("method", self.method)],
            self.started.elapsed(),
        );
        self.metrics.counter(
            "cvm_http_body_ends_total",
            &[("route", self.route), ("outcome", outcome)],
            1,
        );
        if self.metrics.enabled {
            let key = metrics::Key::from_name("cvm_http_inflight");
            self.metrics
                .recorder
                .register_gauge(&key, &super::METADATA)
                .decrement(1.0);
        }
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
    let started = Instant::now();
    metrics
        .recorder
        .register_gauge(
            &metrics::Key::from_name("cvm_http_inflight"),
            &super::METADATA,
        )
        .increment(1.0);
    let mut lifetime = RequestLifetime {
        metrics: metrics.clone(),
        route,
        method,
        started,
        finished: false,
    };
    let response = next.run(request).await;
    let class = match response.status().as_u16() / 100 {
        1 => "1xx",
        2 => "2xx",
        3 => "3xx",
        4 => "4xx",
        _ => "5xx",
    };
    metrics.counter(
        "cvm_http_requests_total",
        &[
            ("route", route),
            ("method", method),
            ("status_class", class),
        ],
        1,
    );
    metrics.duration(
        "cvm_http_header_duration_seconds",
        &[("route", route), ("method", method)],
        started.elapsed(),
    );
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
        metrics
            .recorder
            .register_gauge(
                &metrics::Key::from_name("cvm_http_inflight"),
                &super::super::METADATA,
            )
            .increment(1.0);
        ObservedBody {
            inner: body,
            lifetime: RequestLifetime {
                metrics,
                route: "/v1/responses",
                method: "POST",
                started: Instant::now(),
                finished: false,
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
        }
        assert!(rendered.contains("cvm_http_inflight 0"));
    }
}
