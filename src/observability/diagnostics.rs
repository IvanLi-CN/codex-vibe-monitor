//! Explicit, bounded request diagnostics. No tracing log layer or inbound propagation.
use crate::*;
use opentelemetry::trace::{Span as _, SpanContext, SpanKind, TraceContextExt, Tracer as _};
use opentelemetry::{Context as OtelContext, KeyValue};
use opentelemetry_sdk::trace::Span;
use std::future::Future;
use std::time::SystemTime;

const SPAN_LIMIT: usize = 64;
const BYTE_LIMIT: usize = 64 * 1024;
const WAIT_LIMIT: usize = 8;
const ATTEMPT_LIMIT: usize = 8;

tokio::task_local! { static CURRENT: Option<DiagnosticContext>; }
pub(crate) fn current() -> Option<DiagnosticContext> {
    CURRENT.try_with(Clone::clone).ok().flatten()
}
pub(crate) async fn scope<F: Future>(context: Option<DiagnosticContext>, future: F) -> F::Output {
    CURRENT.scope(context, future).await
}
pub(crate) fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let context = current();
    tokio::spawn(scope(context, future))
}

pub(crate) fn endpoint(path: &str) -> &'static str {
    match path {
        "/v1/responses" => "responses",
        "/v1/chat/completions" => "chat_completions",
        "/v1/responses/compact" => "compact",
        "/v1/alpha/search" => "search",
        "/v1/images/generations" => "image_generation",
        "/v1/images/edits" => "image_edits",
        _ => "other",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Resource {
    Coordinator,
    DbPool,
    AccountCapacity,
    RetryBackoff,
    DownstreamChannel,
    TerminalPriority,
    JournalLock,
}
impl Resource {
    const ALL: [Self; 7] = [
        Self::Coordinator,
        Self::DbPool,
        Self::AccountCapacity,
        Self::RetryBackoff,
        Self::DownstreamChannel,
        Self::TerminalPriority,
        Self::JournalLock,
    ];
    fn index(self) -> usize {
        self as usize
    }
    fn name(self) -> &'static str {
        match self {
            Self::Coordinator => "sqlite_coordinator",
            Self::DbPool => "db_pool",
            Self::AccountCapacity => "account_capacity",
            Self::RetryBackoff => "retry_backoff",
            Self::DownstreamChannel => "downstream_channel",
            Self::TerminalPriority => "terminal_priority",
            Self::JournalLock => "journal_lock",
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Phase {
    RequestRead,
    RequestParse,
    AuthRoute,
    Attempt,
    Connect,
    UpstreamHead,
    Forward,
    JournalAppend,
    Finalize,
}
impl Phase {
    const ALL: [Self; 9] = [
        Self::RequestRead,
        Self::RequestParse,
        Self::AuthRoute,
        Self::Attempt,
        Self::Connect,
        Self::UpstreamHead,
        Self::Forward,
        Self::JournalAppend,
        Self::Finalize,
    ];
    fn index(self) -> usize {
        self as usize
    }
    fn name(self) -> &'static str {
        match self {
            Self::RequestRead => "request_read",
            Self::RequestParse => "request_parse",
            Self::AuthRoute => "auth_route",
            Self::Attempt => "attempt",
            Self::Connect => "connect",
            Self::UpstreamHead => "upstream_head",
            Self::Forward => "forward",
            Self::JournalAppend => "journal_append",
            Self::Finalize => "finalize",
        }
    }
}
pub(super) const ENDPOINT_COUNT: usize = 7;
pub(super) fn endpoint_index(endpoint: &'static str) -> usize {
    match endpoint {
        "responses" => 0,
        "chat_completions" => 1,
        "compact" => 2,
        "search" => 3,
        "image_generation" => 4,
        "image_edits" => 5,
        _ => 6,
    }
}

pub(super) struct RequestMetricHandles {
    response_duration: metrics::Histogram,
    local_wait: metrics::Histogram,
    unattributed: metrics::Histogram,
    response_ends: [metrics::Counter; 3],
    resource_wait: [metrics::Histogram; 7],
    wait_affected: [metrics::Counter; 7],
    wait_over_100ms: [metrics::Counter; 7],
    resource_events: [metrics::Histogram; 7],
    resource_event_outcomes: [[metrics::Counter; 2]; 7],
    stages: [metrics::Histogram; 9],
    milestones: [metrics::Histogram; 3],
    persistence: [metrics::Histogram; 3],
}
impl RequestMetricHandles {
    pub(super) fn new(metrics: &ObservabilityRuntime, endpoint: &'static str) -> Self {
        let counter = |name: &'static str, labels: Vec<(&'static str, &'static str)>| {
            metrics.register_counter(ObservabilityRuntime::key(name, &labels))
        };
        let histogram = |name: &'static str, labels: Vec<(&'static str, &'static str)>| {
            metrics.register_histogram(ObservabilityRuntime::key(name, &labels))
        };
        let response_duration = histogram(
            "cvm_request_response_duration_seconds",
            vec![("endpoint", endpoint)],
        );
        let local_wait = histogram(
            "cvm_request_local_wait_seconds",
            vec![("endpoint", endpoint)],
        );
        let unattributed = histogram(
            "cvm_request_unattributed_seconds",
            vec![("endpoint", endpoint)],
        );
        let response_ends = std::array::from_fn(|index| {
            let outcome = ["complete", "error", "cancelled"][index];
            counter(
                "cvm_request_response_ends_total",
                vec![("endpoint", endpoint), ("outcome", outcome)],
            )
        });
        let resource_wait = std::array::from_fn(|index| {
            histogram(
                "cvm_request_resource_wait_seconds",
                vec![
                    ("endpoint", endpoint),
                    ("resource", Resource::ALL[index].name()),
                ],
            )
        });
        let wait_affected = std::array::from_fn(|index| {
            counter(
                "cvm_request_wait_affected_total",
                vec![
                    ("endpoint", endpoint),
                    ("resource", Resource::ALL[index].name()),
                ],
            )
        });
        let wait_over_100ms = std::array::from_fn(|index| {
            counter(
                "cvm_request_wait_over_100ms_total",
                vec![
                    ("endpoint", endpoint),
                    ("resource", Resource::ALL[index].name()),
                ],
            )
        });
        let resource_events = std::array::from_fn(|index| {
            histogram(
                "cvm_resource_wait_event_seconds",
                vec![("resource", Resource::ALL[index].name())],
            )
        });
        let resource_event_outcomes = std::array::from_fn(|index| {
            std::array::from_fn(|outcome_index| {
                let outcome = ["complete", "cancelled"][outcome_index];
                counter(
                    "cvm_resource_wait_events_total",
                    vec![
                        ("resource", Resource::ALL[index].name()),
                        ("outcome", outcome),
                    ],
                )
            })
        });
        let stages = std::array::from_fn(|index| {
            let phase = Phase::ALL[index];
            histogram(
                "cvm_request_stage_seconds",
                vec![("endpoint", endpoint), ("phase", phase.name())],
            )
        });
        let milestones = std::array::from_fn(|index| {
            let milestone = ["head", "first_byte", "model_delta"][index];
            histogram(
                "cvm_request_milestone_seconds",
                vec![("endpoint", endpoint), ("milestone", milestone)],
            )
        });
        let persistence = std::array::from_fn(|index| {
            let outcome = [
                "committed",
                "association_unavailable",
                "coalesced_unavailable",
            ][index];
            histogram(
                "cvm_request_persistence_seconds",
                vec![("endpoint", endpoint), ("outcome", outcome)],
            )
        });
        Self {
            response_duration,
            local_wait,
            unattributed,
            response_ends,
            resource_wait,
            wait_affected,
            wait_over_100ms,
            resource_events,
            resource_event_outcomes,
            stages,
            milestones,
            persistence,
        }
    }
    fn response_end(&self, outcome: &'static str) -> Option<&metrics::Counter> {
        ["complete", "error", "cancelled"]
            .iter()
            .position(|candidate| *candidate == outcome)
            .map(|index| &self.response_ends[index])
    }
    fn persistence_histogram(&self, outcome: &'static str) -> Option<&metrics::Histogram> {
        [
            "committed",
            "association_unavailable",
            "coalesced_unavailable",
        ]
        .iter()
        .position(|candidate| *candidate == outcome)
        .map(|index| &self.persistence[index])
    }
}
#[derive(Clone, Copy, Default)]
struct WaitStats {
    count: u64,
    sum: Duration,
    max: Duration,
    active: u32,
    active_starts: Duration,
}
#[derive(Clone, Copy)]
struct WaitInterval {
    resource: Resource,
    start: Duration,
    end: Duration,
    lower_bound: bool,
}
impl WaitInterval {
    fn duration(self) -> Duration {
        self.end.saturating_sub(self.start)
    }
}
#[derive(Default)]
struct IntervalUnion {
    active: u32,
    opened: Duration,
    total: Duration,
}
impl IntervalUnion {
    fn begin(&mut self, at: Duration) {
        if self.active == 0 {
            self.opened = at;
        }
        self.active += 1;
    }
    fn end(&mut self, at: Duration) {
        self.active = self.active.saturating_sub(1);
        if self.active == 0 {
            self.total += at.saturating_sub(self.opened);
        }
    }
    fn at(&self, at: Duration) -> Duration {
        self.total
            + if self.active > 0 {
                at.saturating_sub(self.opened)
            } else {
                Duration::ZERO
            }
    }
}
struct State {
    root: Option<Span>,
    parent: Option<SpanContext>,
    ended: Option<Duration>,
    waits: [WaitStats; 7],
    wait_union: IntervalUnion,
    phase_union: IntervalUnion,
    longest: Vec<WaitInterval>,
    spans: usize,
    bytes: usize,
    truncated: bool,
    attempts: usize,
    milestones: [Option<Duration>; 3],
    persistence: &'static str,
    ttft_applicable: Option<bool>,
}
struct Inner {
    metrics: Arc<ObservabilityRuntime>,
    metric_handles: Arc<RequestMetricHandles>,
    traces: Arc<super::traces::TraceRuntime>,
    started: Instant,
    utc: SystemTime,
    endpoint: &'static str,
    state: std::sync::Mutex<State>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.traces.state.active.fetch_sub(1, Ordering::AcqRel);
    }
}
#[derive(Clone)]
pub(crate) struct DiagnosticContext(Arc<Inner>);
impl std::fmt::Debug for DiagnosticContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DiagnosticContext")
    }
}
impl DiagnosticContext {
    pub(crate) fn begin(
        metrics: Arc<ObservabilityRuntime>,
        endpoint: &'static str,
    ) -> Option<Self> {
        if !metrics.enabled {
            return None;
        }
        let traces = metrics.trace_runtime();
        if traces
            .state
            .active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < super::traces::ACTIVE_LIMIT).then_some(n + 1)
            })
            .is_err()
        {
            metrics.counter(
                "cvm_diagnostic_dropped_total",
                &[("reason", "active_limit")],
                1,
            );
            return None;
        }
        let started = Instant::now();
        let utc = SystemTime::now();
        let metric_handles = metrics.request_metrics(endpoint);
        let root = traces.tracer.as_ref().map(|tracer| {
            tracer.build_with_context(
                tracer
                    .span_builder("cvm.proxy.response")
                    .with_kind(SpanKind::Server)
                    .with_start_time(utc)
                    .with_attributes([
                        KeyValue::new("cvm.endpoint", endpoint),
                        KeyValue::new("cvm.record", "response"),
                    ]),
                &OtelContext::new(),
            )
        });
        let parent = root.as_ref().map(|s| s.span_context().clone());
        Some(Self(Arc::new(Inner {
            metrics,
            metric_handles,
            traces,
            started,
            utc,
            endpoint,
            state: std::sync::Mutex::new(State {
                root,
                parent,
                ended: None,
                waits: [WaitStats::default(); 7],
                wait_union: IntervalUnion::default(),
                phase_union: IntervalUnion::default(),
                longest: Vec::with_capacity(WAIT_LIMIT),
                spans: 1,
                bytes: 4096,
                truncated: false,
                attempts: 0,
                milestones: [None; 3],
                persistence: "not_observed",
                ttft_applicable: None,
            }),
        })))
    }
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn at(&self) -> Duration {
        self.0.started.elapsed()
    }
    pub(crate) fn ttft_applicable(&self, value: bool) {
        self.state().ttft_applicable = Some(value);
    }
    pub(crate) fn milestone(&self, name: &'static str) {
        let index = match name {
            "head" => 0,
            "first_byte" => 1,
            "model_delta" => 2,
            _ => return,
        };
        let mut state = self.state();
        if state.ended.is_none() && state.milestones[index].is_none() {
            let elapsed = self.at();
            state.milestones[index] = Some(elapsed);
            self.0.metric_handles.milestones[index].record(elapsed.as_secs_f64());
        }
    }
    fn emit(&self, name: &'static str, start: Duration, end: Duration, attributes: Vec<KeyValue>) {
        self.emit_linked(name, start, end, attributes, Vec::new());
    }
    pub(crate) fn reference_batch(&self, batch: Option<SpanContext>) {
        if let Some(batch) = batch {
            let at = self.at();
            self.emit_linked(
                "cvm.terminal.batch_reference",
                at,
                at,
                vec![
                    KeyValue::new("cvm.shared_work", true),
                    KeyValue::new("cvm.exclusive_cost", false),
                ],
                vec![opentelemetry::trace::Link::with_context(batch)],
            );
        }
    }
    fn emit_linked(
        &self,
        name: &'static str,
        start: Duration,
        end: Duration,
        attributes: Vec<KeyValue>,
        links: Vec<opentelemetry::trace::Link>,
    ) {
        let Some(tracer) = self.0.traces.tracer.as_ref() else {
            return;
        };
        let parent = {
            let mut state = self.state();
            // Reserve conservative protobuf space for bounded, fixed attributes. Root has 4 KiB.
            let reserved = if name == "cvm.terminal.enqueue_to_commit" {
                0
            } else {
                1024
            };
            if state.spans >= SPAN_LIMIT - usize::from(reserved > 0)
                || state.bytes + 1024 + reserved > BYTE_LIMIT
            {
                if !state.truncated {
                    self.0.metrics.counter(
                        "cvm_diagnostic_dropped_total",
                        &[("reason", "detail_limit")],
                        1,
                    );
                }
                state.truncated = true;
                return;
            }
            state.spans += 1;
            state.bytes += 1024;
            state.parent.clone()
        };
        let Some(parent) = parent else { return };
        let mut span = tracer.build_with_context(
            tracer
                .span_builder(name)
                .with_start_time(self.0.utc + start)
                .with_attributes(attributes)
                .with_links(links),
            &OtelContext::new().with_remote_span_context(parent),
        );
        span.end_with_timestamp(self.0.utc + end);
    }
    pub(crate) fn finish_response(&self, outcome: &'static str, status_class: &'static str) {
        let at = self.at();
        let (
            mut root,
            waits,
            longest,
            wait_total,
            unattributed,
            attempts,
            truncated,
            milestones,
            persistence,
            incomplete,
            ttft_applicable,
        ) = {
            let mut state = self.state();
            if state.ended.is_some() {
                return;
            }
            state.ended = Some(at);
            let incomplete = state.wait_union.active > 0;
            let mut waits = state.waits;
            for stats in &mut waits {
                stats.sum += at
                    .saturating_mul(stats.active)
                    .saturating_sub(stats.active_starts);
            }
            (
                state.root.take(),
                waits,
                std::mem::take(&mut state.longest),
                state.wait_union.at(at),
                at.saturating_sub(state.phase_union.at(at)),
                state.attempts,
                state.truncated,
                state.milestones,
                state.persistence,
                incomplete,
                state.ttft_applicable,
            )
        };
        self.0
            .metric_handles
            .response_duration
            .record(at.as_secs_f64());
        self.0
            .metric_handles
            .local_wait
            .record(wait_total.as_secs_f64());
        self.0
            .metric_handles
            .unattributed
            .record(unattributed.as_secs_f64());
        if let Some(counter) = self.0.metric_handles.response_end(outcome) {
            counter.increment(1);
        } else {
            self.0.metrics.counter(
                "cvm_request_response_ends_total",
                &[("endpoint", self.0.endpoint), ("outcome", outcome)],
                1,
            );
        }
        for resource in Resource::ALL {
            let stats = waits[resource.index()];
            // Include zeros once per observed request, so affected/request denominators agree.
            self.0.metric_handles.resource_wait[resource.index()].record(stats.sum.as_secs_f64());
            if stats.count + u64::from(stats.active) > 0 {
                self.0.metric_handles.wait_affected[resource.index()].increment(1);
            }
            if stats.sum > Duration::from_millis(100) {
                self.0.metric_handles.wait_over_100ms[resource.index()].increment(1);
            }
        }
        for interval in longest {
            self.emit(
                "cvm.resource.wait",
                interval.start,
                interval.end,
                vec![
                    KeyValue::new("cvm.resource", interval.resource.name()),
                    KeyValue::new("cvm.lower_bound", interval.lower_bound),
                    KeyValue::new("cvm.selected_interval", true),
                ],
            );
        }
        if let Some(root) = root.as_mut() {
            let random_candidate = root.span_context().trace_id().to_bytes()[0] & 15 == 0;
            for kv in [
                KeyValue::new("cvm.outcome", outcome),
                KeyValue::new("cvm.status_class", status_class),
                KeyValue::new("cvm.response_ms", at.as_secs_f64() * 1000.0),
                KeyValue::new("cvm.local_wait_ms", wait_total.as_secs_f64() * 1000.0),
                KeyValue::new("cvm.unattributed_ms", unattributed.as_secs_f64() * 1000.0),
                KeyValue::new("cvm.attempts", attempts as i64),
                KeyValue::new("cvm.retry", attempts > 1),
                KeyValue::new("cvm.slow", at >= Duration::from_secs(30)),
                KeyValue::new("cvm.high_wait", wait_total >= Duration::from_millis(250)),
                KeyValue::new(
                    "cvm.normal_candidate",
                    random_candidate
                        && outcome == "complete"
                        && status_class == "2xx"
                        && attempts <= 1
                        && wait_total < Duration::from_millis(250)
                        && at < Duration::from_secs(30),
                ),
                KeyValue::new("cvm.persistence", persistence),
                KeyValue::new("cvm.lower_bound", incomplete),
                KeyValue::new("cvm.detail_truncated", truncated || self.state().truncated),
                KeyValue::new(
                    "cvm.ttft_state",
                    if milestones[2].is_some() {
                        "observed"
                    } else if ttft_applicable != Some(false)
                        && matches!(self.0.endpoint, "responses" | "chat_completions")
                        && status_class == "2xx"
                    {
                        "unobserved"
                    } else {
                        "not_applicable"
                    },
                ),
                KeyValue::new("cvm.export_completeness", "unknown"),
            ] {
                root.set_attribute(kv);
            }
            for (index, name) in ["cvm.head_ms", "cvm.first_byte_ms", "cvm.model_delta_ms"]
                .into_iter()
                .enumerate()
            {
                if let Some(value) = milestones[index] {
                    root.set_attribute(KeyValue::new(name, value.as_secs_f64() * 1000.0));
                }
            }
            for resource in Resource::ALL {
                let stats = waits[resource.index()];
                root.set_attribute(KeyValue::new(
                    format!("cvm.wait.{}.count", resource.name()),
                    stats.count as i64,
                ));
                root.set_attribute(KeyValue::new(
                    format!("cvm.wait.{}.sum_ms", resource.name()),
                    stats.sum.as_secs_f64() * 1000.0,
                ));
                root.set_attribute(KeyValue::new(
                    format!("cvm.wait.{}.max_ms", resource.name()),
                    stats.max.as_secs_f64() * 1000.0,
                ));
            }
            root.end_with_timestamp(self.0.utc + at);
        }
    }
    pub(crate) fn phase(&self, phase: Phase) -> Guard {
        Guard::new(self.clone(), Kind::Phase(phase))
    }
    pub(crate) fn waiting(&self, resource: Resource) -> Guard {
        Guard::new(self.clone(), Kind::Wait(resource))
    }
    pub(crate) fn persistence(&self) -> Arc<PersistenceTicket> {
        self.state().persistence = "pending";
        Arc::new(PersistenceTicket {
            context: self.clone(),
            start: self.at(),
            finished: AtomicBool::new(false),
        })
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Phase(Phase),
    Wait(Resource),
}
pub(crate) struct Guard {
    context: DiagnosticContext,
    kind: Kind,
    start: Duration,
    complete: bool,
    detail: bool,
}
impl Guard {
    fn new(context: DiagnosticContext, kind: Kind) -> Self {
        let start = context.at();
        let mut state = context.state();
        let mut detail = true;
        if matches!(kind, Kind::Phase(Phase::Attempt)) {
            state.attempts += 1;
            detail = state.attempts <= ATTEMPT_LIMIT;
            if state.attempts == ATTEMPT_LIMIT + 1 {
                context.0.metrics.counter(
                    "cvm_diagnostic_dropped_total",
                    &[("reason", "attempt_limit")],
                    1,
                );
            }
            if !detail {
                state.truncated = true;
            }
        }
        if state.ended.is_none() {
            state.phase_union.begin(start);
            if let Kind::Wait(resource) = kind {
                state.wait_union.begin(start);
                let stats = &mut state.waits[resource.index()];
                stats.active += 1;
                stats.active_starts += start;
            }
        }
        drop(state);
        Self {
            context,
            kind,
            start,
            complete: false,
            detail,
        }
    }
    pub(crate) fn complete(mut self) {
        self.complete = true;
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        let end = self.context.at();
        let duration = end.saturating_sub(self.start);
        let mut state = self.context.state();
        if state.ended.is_none() {
            state.phase_union.end(end);
            if let Kind::Wait(resource) = self.kind {
                state.wait_union.end(end);
                let stats = &mut state.waits[resource.index()];
                stats.active = stats.active.saturating_sub(1);
                stats.active_starts = stats.active_starts.saturating_sub(self.start);
                stats.count += 1;
                stats.sum += duration;
                stats.max = stats.max.max(duration);
                let interval = WaitInterval {
                    resource,
                    start: self.start,
                    end,
                    lower_bound: !self.complete,
                };
                if state.longest.len() < WAIT_LIMIT {
                    state.longest.push(interval);
                } else if let Some((index, shortest)) = state
                    .longest
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, i)| i.duration())
                    && duration > shortest.duration()
                {
                    state.longest[index] = interval;
                }
            }
        }
        drop(state);
        match self.kind {
            Kind::Wait(resource) => {
                self.context.0.metric_handles.resource_events[resource.index()]
                    .record(duration.as_secs_f64());
                self.context.0.metric_handles.resource_event_outcomes[resource.index()]
                    [usize::from(!self.complete)]
                .increment(1);
            }
            Kind::Phase(phase) => {
                self.context.0.metric_handles.stages[phase.index()].record(duration.as_secs_f64());
                if self.detail {
                    self.context.emit(
                        phase.name(),
                        self.start,
                        end,
                        vec![
                            KeyValue::new("cvm.phase", phase.name()),
                            KeyValue::new("cvm.lower_bound", !self.complete),
                        ],
                    );
                }
            }
        }
    }
}
pub(crate) fn phase(phase: Phase) -> Option<Guard> {
    current().map(|c| c.phase(phase))
}
pub(crate) fn waiting(resource: Resource) -> Option<Guard> {
    current().map(|c| c.waiting(resource))
}
pub(crate) async fn wait<F: Future>(resource: Resource, future: F) -> F::Output {
    let guard = waiting(resource);
    let output = future.await;
    if let Some(guard) = guard {
        guard.complete();
    }
    output
}
pub(crate) async fn measure_phase<F: Future>(name: Phase, future: F) -> F::Output {
    let guard = phase(name);
    let output = future.await;
    if let Some(guard) = guard {
        guard.complete();
    }
    output
}
pub(crate) fn milestone(name: &'static str) {
    if let Some(context) = current() {
        context.milestone(name);
    }
}
pub(crate) async fn send<T>(
    sender: &mpsc::Sender<T>,
    value: T,
) -> std::result::Result<(), mpsc::error::SendError<T>> {
    match sender.try_send(value) {
        Ok(()) => Ok(()),
        Err(mpsc::error::TrySendError::Closed(value)) => Err(mpsc::error::SendError(value)),
        Err(mpsc::error::TrySendError::Full(value)) => {
            wait(Resource::DownstreamChannel, sender.send(value)).await
        }
    }
}

pub(crate) struct PersistenceTicket {
    pub(crate) context: DiagnosticContext,
    start: Duration,
    finished: AtomicBool,
}
impl std::fmt::Debug for PersistenceTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PersistenceTicket")
    }
}
impl PersistenceTicket {
    pub(crate) fn finish(&self, outcome: &'static str, shared_batch: bool) {
        self.finish_with_batch(outcome, shared_batch, None);
    }
    pub(crate) fn finish_with_batch(
        &self,
        outcome: &'static str,
        shared_batch: bool,
        batch: Option<SpanContext>,
    ) {
        if self.finished.swap(true, Ordering::AcqRel) {
            return;
        }
        let end = self.context.at();
        self.context.state().persistence = outcome;
        let duration = end.saturating_sub(self.start);
        if let Some(histogram) = self.context.0.metric_handles.persistence_histogram(outcome) {
            histogram.record(duration.as_secs_f64());
        } else {
            self.context.0.metrics.duration(
                "cvm_request_persistence_seconds",
                &[("endpoint", self.context.0.endpoint), ("outcome", outcome)],
                duration,
            );
        }
        self.context.emit_linked(
            "cvm.terminal.enqueue_to_commit",
            self.start,
            end,
            vec![
                KeyValue::new("cvm.record", "persistence"),
                KeyValue::new("cvm.outcome", outcome),
                KeyValue::new("cvm.shared_batch", shared_batch),
                KeyValue::new("cvm.exclusive_cost", false),
            ],
            batch
                .into_iter()
                .map(opentelemetry::trace::Link::with_context)
                .collect(),
        );
    }
}
impl Drop for PersistenceTicket {
    fn drop(&mut self) {
        self.finish("association_unavailable", false);
    }
}

pub(crate) struct SharedBatch {
    span: Option<Span>,
    tracer: Option<opentelemetry_sdk::trace::SdkTracer>,
    utc: SystemTime,
    started: Instant,
    pool_end: Duration,
}
impl SharedBatch {
    pub(crate) fn begin<'a>(
        tickets: impl Iterator<Item = &'a Arc<PersistenceTicket>>,
        size: usize,
    ) -> Self {
        let mut tracer = None;
        let mut links = Vec::with_capacity(16);
        for ticket in tickets.take(16) {
            tracer = ticket.context.0.traces.tracer.clone();
            if let Some(parent) = ticket.context.state().parent.clone() {
                links.push(opentelemetry::trace::Link::with_context(parent));
            }
        }
        let utc = SystemTime::now();
        let started = Instant::now();
        let span = tracer.as_ref().map(|tracer| {
            tracer.build_with_context(
                tracer
                    .span_builder("cvm.terminal.shared_batch")
                    .with_start_time(utc)
                    .with_links(links)
                    .with_attributes([
                        KeyValue::new("cvm.record", "shared_batch"),
                        KeyValue::new("cvm.batch_size", size as i64),
                        KeyValue::new("cvm.links_truncated", size > 16),
                        KeyValue::new("cvm.lower_bound", true),
                    ]),
                &OtelContext::new(),
            )
        });
        Self {
            span,
            tracer,
            utc,
            started,
            pool_end: Duration::ZERO,
        }
    }
    fn interval(&self, name: &'static str, start: Duration, end: Duration) {
        if let (Some(span), Some(tracer)) = (&self.span, &self.tracer) {
            let mut child = tracer.build_with_context(
                tracer
                    .span_builder(name)
                    .with_start_time(self.utc + start)
                    .with_attributes([
                        KeyValue::new("cvm.shared_work", true),
                        KeyValue::new("cvm.exclusive_cost", false),
                    ]),
                &OtelContext::new().with_remote_span_context(span.span_context().clone()),
            );
            child.end_with_timestamp(self.utc + end);
        }
    }
    pub(crate) fn pool_acquired(&mut self) {
        self.pool_end = self.started.elapsed();
        self.interval("cvm.terminal.batch.pool", Duration::ZERO, self.pool_end);
    }
    pub(crate) fn coordinator_admitted(mut self) -> Option<SpanContext> {
        let end = self.started.elapsed();
        self.interval("cvm.terminal.batch.coordinator", Duration::ZERO, end);
        if let Some(mut span) = self.span.take() {
            span.set_attribute(KeyValue::new("cvm.lower_bound", false));
            span.set_attribute(KeyValue::new("cvm.outcome", "admitted"));
            let context = span.span_context().clone();
            span.end_with_timestamp(self.utc + end);
            Some(context)
        } else {
            None
        }
    }
    pub(crate) fn committed(mut self) -> Option<SpanContext> {
        let end = self.started.elapsed();
        self.interval("cvm.terminal.batch.execute", self.pool_end, end);
        if let Some(mut span) = self.span.take() {
            span.set_attribute(KeyValue::new("cvm.lower_bound", false));
            span.set_attribute(KeyValue::new("cvm.outcome", "committed"));
            let context = span.span_context().clone();
            span.end_with_timestamp(self.utc + end);
            Some(context)
        } else {
            None
        }
    }
}

#[cfg(test)]
#[path = "../tests/lightweight/request_diagnostics.rs"]
mod request_diagnostics;
