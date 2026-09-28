use super::*;
use crate::db_pressure::global_db_pressure_gate;
use crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator;

// The telemetry file is disposable state. Create the final schema once and replace the file
// when the shape changes instead of running release-to-release data migrations.
const TELEMETRY_SCHEMA_VERSION: i64 = 1;
const TELEMETRY_QUEUE_CAPACITY: usize = 2_048;
const TELEMETRY_FLUSH_INTERVAL: Duration = Duration::from_secs(60);
const TELEMETRY_SAMPLE_INTERVAL: Duration = Duration::from_secs(10);
const TELEMETRY_RETENTION_HOURS: i64 = 24 * 30 * 13;
const TELEMETRY_MAX_QUERY_POINTS: usize = 720;
const TELEMETRY_MAX_PENDING_MINUTE_BUCKETS: usize = 2;
const TELEMETRY_BROWSER_MAX_EVENTS: usize = 8;
pub(crate) const TELEMETRY_BROWSER_MAX_BYTES: usize = 2 * 1024;
const TELEMETRY_BROWSER_GLOBAL_RATE_LIMIT: u32 = 120;
const TELEMETRY_BROWSER_CLIENT_RATE_LIMIT: u32 = 30;
const TELEMETRY_BROWSER_RATE_WINDOW: Duration = Duration::from_secs(60);

type TelemetryColumnContract = (&'static str, &'static str, i64, i64);
type TelemetryTableContract = (&'static str, &'static [TelemetryColumnContract]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetricKind {
    Counter,
    Duration,
    Gauge,
}

#[derive(Debug, Clone, Copy)]
struct MetricSpec {
    id: &'static str,
    section: &'static str,
    kind: MetricKind,
    long_term: bool,
}

// This is deliberately static. Request-level proxy facts live in the main database; adding a
// system metric or dimension requires a code review and a capacity check.
static METRIC_SPECS: &[MetricSpec] = &[
    MetricSpec {
        id: "http.in_flight",
        section: "overview",
        kind: MetricKind::Gauge,
        long_term: false,
    },
    MetricSpec {
        id: "p1.ack_duration_ms",
        section: "storage",
        kind: MetricKind::Duration,
        long_term: true,
    },
    MetricSpec {
        id: "p1.queue_depth",
        section: "storage",
        kind: MetricKind::Gauge,
        long_term: true,
    },
    MetricSpec {
        id: "p1.queue_bytes",
        section: "storage",
        kind: MetricKind::Gauge,
        long_term: true,
    },
    MetricSpec {
        id: "p2.queue_depth",
        section: "storage",
        kind: MetricKind::Gauge,
        long_term: false,
    },
    MetricSpec {
        id: "p1.retry_count",
        section: "storage",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "p2.retry_count",
        section: "storage",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "p1.transfer_bytes",
        section: "storage",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "p2.next_attempt_delay_ms",
        section: "storage",
        kind: MetricKind::Gauge,
        long_term: false,
    },
    MetricSpec {
        id: "p2.deferred_age_ms",
        section: "storage",
        kind: MetricKind::Gauge,
        long_term: false,
    },
    MetricSpec {
        id: "p2.flush_attempt_count",
        section: "storage",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "p2.pressure_defer_count",
        section: "storage",
        kind: MetricKind::Counter,
        long_term: true,
    },
    MetricSpec {
        id: "p2.lock_retry_count",
        section: "storage",
        kind: MetricKind::Counter,
        long_term: true,
    },
    MetricSpec {
        id: "sqlite.busy_count",
        section: "storage",
        kind: MetricKind::Counter,
        long_term: true,
    },
    MetricSpec {
        id: "sqlite.locked_count",
        section: "storage",
        kind: MetricKind::Counter,
        long_term: true,
    },
    MetricSpec {
        id: "sqlite.pool_timeout_count",
        section: "storage",
        kind: MetricKind::Counter,
        long_term: true,
    },
    MetricSpec {
        id: "sqlite.background_skip_count",
        section: "storage",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "sqlite.write_duration_ms",
        section: "storage",
        kind: MetricKind::Duration,
        long_term: false,
    },
    MetricSpec {
        id: "sqlite.write_rows",
        section: "storage",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "sqlite.write_bytes",
        section: "storage",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "sqlite.wal_bytes",
        section: "storage",
        kind: MetricKind::Gauge,
        long_term: false,
    },
    MetricSpec {
        id: "sqlite.coordinator_waiters",
        section: "storage",
        kind: MetricKind::Gauge,
        long_term: false,
    },
    MetricSpec {
        id: "sqlite.coordinator_wait_duration_ms",
        section: "storage",
        kind: MetricKind::Duration,
        long_term: false,
    },
    MetricSpec {
        id: "sqlite.coordinator_bypass_count",
        section: "storage",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "sqlite.maintenance_fairness_count",
        section: "storage",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "projection.last_good_age_ms",
        section: "projection",
        kind: MetricKind::Gauge,
        long_term: true,
    },
    MetricSpec {
        id: "projection.build_count",
        section: "projection",
        kind: MetricKind::Counter,
        long_term: true,
    },
    MetricSpec {
        id: "projection.live_db_read_count",
        section: "projection",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "sse.active_subscribers",
        section: "projection",
        kind: MetricKind::Gauge,
        long_term: true,
    },
    MetricSpec {
        id: "projection.cadence_miss_count",
        section: "projection",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "projection.revision_count",
        section: "projection",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "projection.publish_duration_ms",
        section: "projection",
        kind: MetricKind::Duration,
        long_term: false,
    },
    MetricSpec {
        id: "projection.publish_count",
        section: "projection",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "projection.snapshot_bytes",
        section: "projection",
        kind: MetricKind::Gauge,
        long_term: false,
    },
    MetricSpec {
        id: "projection.reconcile_duration_ms",
        section: "projection",
        kind: MetricKind::Duration,
        long_term: false,
    },
    MetricSpec {
        id: "projection.reconcile_count",
        section: "projection",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "projection.reconcile_defer_count",
        section: "projection",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "projection.reconcile_failure_count",
        section: "projection",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "sse.publish_error_count",
        section: "projection",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "sse.publish_duration_ms",
        section: "projection",
        kind: MetricKind::Duration,
        long_term: false,
    },
    MetricSpec {
        id: "sse.frame_bytes",
        section: "projection",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "maintenance.backlog_age_ms",
        section: "maintenance",
        kind: MetricKind::Gauge,
        long_term: true,
    },
    MetricSpec {
        id: "maintenance.run_duration_ms",
        section: "maintenance",
        kind: MetricKind::Duration,
        long_term: true,
    },
    MetricSpec {
        id: "maintenance.processed_rows",
        section: "maintenance",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "maintenance.raw_bytes_before",
        section: "maintenance",
        kind: MetricKind::Gauge,
        long_term: false,
    },
    MetricSpec {
        id: "maintenance.raw_bytes_after",
        section: "maintenance",
        kind: MetricKind::Gauge,
        long_term: false,
    },
    MetricSpec {
        id: "maintenance.compressed_file_count",
        section: "maintenance",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "maintenance.removed_file_count",
        section: "maintenance",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "maintenance.archived_rows",
        section: "maintenance",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "process.rss_bytes",
        section: "process",
        kind: MetricKind::Gauge,
        long_term: true,
    },
    MetricSpec {
        id: "process.cpu_percent",
        section: "process",
        kind: MetricKind::Gauge,
        long_term: true,
    },
    MetricSpec {
        id: "process.rss_anon_bytes",
        section: "process",
        kind: MetricKind::Gauge,
        long_term: true,
    },
    MetricSpec {
        id: "process.swap_bytes",
        section: "process",
        kind: MetricKind::Gauge,
        long_term: true,
    },
    MetricSpec {
        id: "process.managed_bytes",
        section: "process",
        kind: MetricKind::Gauge,
        long_term: true,
    },
    MetricSpec {
        id: "process.unattributed_anon_bytes",
        section: "process",
        kind: MetricKind::Gauge,
        long_term: false,
    },
    MetricSpec {
        id: "storage.main_db_bytes",
        section: "process",
        kind: MetricKind::Gauge,
        long_term: true,
    },
    MetricSpec {
        id: "storage.telemetry_db_bytes",
        section: "process",
        kind: MetricKind::Gauge,
        long_term: true,
    },
    MetricSpec {
        id: "process.thread_count",
        section: "process",
        kind: MetricKind::Gauge,
        long_term: false,
    },
    MetricSpec {
        id: "process.disk_free_bytes",
        section: "process",
        kind: MetricKind::Gauge,
        long_term: false,
    },
    MetricSpec {
        id: "storage.telemetry_wal_bytes",
        section: "process",
        kind: MetricKind::Gauge,
        long_term: false,
    },
    MetricSpec {
        id: "telemetry.queue_depth",
        section: "process",
        kind: MetricKind::Gauge,
        long_term: false,
    },
    MetricSpec {
        id: "telemetry.dropped_samples",
        section: "process",
        kind: MetricKind::Counter,
        long_term: true,
    },
    MetricSpec {
        id: "telemetry.flush_duration_ms",
        section: "process",
        kind: MetricKind::Duration,
        long_term: false,
    },
    MetricSpec {
        id: "telemetry.flush_failure_count",
        section: "process",
        kind: MetricKind::Counter,
        long_term: true,
    },
    MetricSpec {
        id: "telemetry.flush_batch_events",
        section: "process",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "telemetry.flush_batch_buckets",
        section: "process",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "telemetry.backoff_count",
        section: "process",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "browser.data_ready_ms",
        section: "browser",
        kind: MetricKind::Duration,
        long_term: false,
    },
    MetricSpec {
        id: "browser.update_to_paint_ms",
        section: "browser",
        kind: MetricKind::Duration,
        long_term: false,
    },
    MetricSpec {
        id: "browser.long_task_ms",
        section: "browser",
        kind: MetricKind::Duration,
        long_term: false,
    },
    MetricSpec {
        id: "browser.long_task_count",
        section: "browser",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "browser.api_request_duration_ms",
        section: "browser",
        kind: MetricKind::Duration,
        long_term: false,
    },
    MetricSpec {
        id: "browser.api_request_count",
        section: "browser",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "browser.sse_duration_ms",
        section: "browser",
        kind: MetricKind::Duration,
        long_term: false,
    },
    MetricSpec {
        id: "browser.sse_disconnect_count",
        section: "browser",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "browser.unsupported_count",
        section: "browser",
        kind: MetricKind::Counter,
        long_term: false,
    },
    MetricSpec {
        id: "browser.visibility_hidden_count",
        section: "browser",
        kind: MetricKind::Counter,
        long_term: false,
    },
];

#[derive(Debug, Clone, Copy)]
enum EventKind {
    Counter,
    Duration,
    Gauge,
}

#[derive(Debug)]
struct TelemetryEvent {
    metric_id: &'static str,
    dimension: &'static str,
    kind: EventKind,
    value: f64,
    weight_seconds: f64,
    at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct BucketAccumulator {
    kind: EventKind,
    count: u64,
    expected_count: u64,
    sum: f64,
    min: f64,
    max: f64,
    last: f64,
    weighted_sum: f64,
    weighted_seconds: f64,
    histogram: [u64; 8],
}

impl BucketAccumulator {
    fn new(kind: EventKind, value: f64, weight_seconds: f64) -> Self {
        let mut accumulator = Self {
            kind,
            count: 0,
            expected_count: 0,
            sum: 0.0,
            min: value,
            max: value,
            last: value,
            weighted_sum: 0.0,
            weighted_seconds: 0.0,
            histogram: [0; 8],
        };
        accumulator.add(value, weight_seconds);
        accumulator
    }

    fn add(&mut self, value: f64, weight_seconds: f64) {
        self.count = self.count.saturating_add(1);
        self.expected_count = self.expected_count.saturating_add(1);
        self.sum += value;
        self.min = self.min.min(value);
        self.max = self.max.max(value);
        self.last = value;
        if weight_seconds > 0.0 {
            self.weighted_sum += value * weight_seconds;
            self.weighted_seconds += weight_seconds;
        }
        if matches!(self.kind, EventKind::Duration) {
            self.histogram[duration_histogram_index(value)] =
                self.histogram[duration_histogram_index(value)].saturating_add(1);
        }
    }

    fn add_expected_only(&mut self, count: u64) {
        self.expected_count = self.expected_count.saturating_add(count);
    }
}

fn duration_histogram_index(value: f64) -> usize {
    [1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0]
        .iter()
        .position(|limit| value <= *limit)
        .unwrap_or(7)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerformanceTelemetryHealth {
    pub(crate) state: String,
    pub(crate) enabled: bool,
    pub(crate) path: String,
    pub(crate) epoch: String,
    pub(crate) queue_depth: usize,
    pub(crate) queue_capacity: usize,
    pub(crate) dropped_samples: u64,
    pub(crate) flush_failure_count: u64,
    pub(crate) last_successful_flush: Option<String>,
    pub(crate) last_error: Option<String>,
}

#[derive(Debug)]
pub(crate) struct PerformanceTelemetryRuntime {
    enabled: bool,
    pool: OnceCell<Pool<Sqlite>>,
    sender: mpsc::Sender<TelemetryEvent>,
    health: Mutex<PerformanceTelemetryHealth>,
    dropped_samples: AtomicU64,
    dropped_expectations: std::sync::Mutex<HashMap<(i64, &'static str, &'static str), u64>>,
    flush_failure_count: AtomicU64,
    active_http_requests: AtomicU64,
    browser_rate_limiter: Mutex<BrowserRateLimiter>,
    shutdown: CancellationToken,
    init_handle: std::sync::Mutex<Option<JoinHandle<()>>>,
    writer_handle: std::sync::Mutex<Option<JoinHandle<()>>>,
}

#[derive(Debug)]
struct BrowserRateLimiter {
    window_started: Instant,
    global_count: u32,
    clients: HashMap<String, (Instant, u32)>,
}

impl BrowserRateLimiter {
    fn new() -> Self {
        Self {
            window_started: Instant::now(),
            global_count: 0,
            clients: HashMap::new(),
        }
    }

    fn allow(&mut self, client: &str) -> bool {
        let now = Instant::now();
        if now.duration_since(self.window_started) >= TELEMETRY_BROWSER_RATE_WINDOW {
            self.window_started = now;
            self.global_count = 0;
            self.clients.clear();
        }
        self.clients
            .retain(|_, (started, _)| now.duration_since(*started) < TELEMETRY_BROWSER_RATE_WINDOW);
        if self.global_count >= TELEMETRY_BROWSER_GLOBAL_RATE_LIMIT {
            return false;
        }
        let entry = self.clients.entry(client.to_string()).or_insert((now, 0));
        if entry.1 >= TELEMETRY_BROWSER_CLIENT_RATE_LIMIT {
            return false;
        }
        entry.1 += 1;
        self.global_count += 1;
        if self.clients.len() > 128
            && let Some(oldest) = self
                .clients
                .iter()
                .min_by_key(|(_, (started, _))| *started)
                .map(|(key, _)| key.clone())
        {
            self.clients.remove(&oldest);
        }
        true
    }
}

impl PerformanceTelemetryRuntime {
    #[cfg(test)]
    pub(crate) fn disabled_for_tests() -> Arc<Self> {
        let (sender, _receiver) = mpsc::channel(TELEMETRY_QUEUE_CAPACITY);
        Arc::new(Self {
            enabled: false,
            pool: OnceCell::new(),
            sender,
            health: Mutex::new(PerformanceTelemetryHealth {
                state: "disabled".to_string(),
                enabled: false,
                path: String::new(),
                epoch: "test".to_string(),
                queue_depth: 0,
                queue_capacity: TELEMETRY_QUEUE_CAPACITY,
                dropped_samples: 0,
                flush_failure_count: 0,
                last_successful_flush: None,
                last_error: None,
            }),
            dropped_samples: AtomicU64::new(0),
            dropped_expectations: std::sync::Mutex::new(HashMap::new()),
            flush_failure_count: AtomicU64::new(0),
            active_http_requests: AtomicU64::new(0),
            browser_rate_limiter: Mutex::new(BrowserRateLimiter::new()),
            shutdown: CancellationToken::new(),
            init_handle: std::sync::Mutex::new(None),
            writer_handle: std::sync::Mutex::new(None),
        })
    }

    pub(crate) fn start(
        config: &AppConfig,
        process_started_at_utc: DateTime<Utc>,
        shutdown: CancellationToken,
    ) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel(TELEMETRY_QUEUE_CAPACITY);
        let epoch = format!(
            "{}-{}",
            process_started_at_utc.timestamp_millis(),
            std::process::id()
        );
        let health = PerformanceTelemetryHealth {
            state: if config.performance_telemetry_enabled {
                "starting".to_string()
            } else {
                "disabled".to_string()
            },
            enabled: config.performance_telemetry_enabled,
            path: config.performance_database_path.display().to_string(),
            epoch: epoch.clone(),
            queue_depth: 0,
            queue_capacity: TELEMETRY_QUEUE_CAPACITY,
            dropped_samples: 0,
            flush_failure_count: 0,
            last_successful_flush: None,
            last_error: None,
        };

        let runtime = Arc::new(Self {
            enabled: config.performance_telemetry_enabled,
            pool: OnceCell::new(),
            sender,
            health: Mutex::new(health),
            dropped_samples: AtomicU64::new(0),
            dropped_expectations: std::sync::Mutex::new(HashMap::new()),
            flush_failure_count: AtomicU64::new(0),
            active_http_requests: AtomicU64::new(0),
            browser_rate_limiter: Mutex::new(BrowserRateLimiter::new()),
            shutdown: shutdown.clone(),
            init_handle: std::sync::Mutex::new(None),
            writer_handle: std::sync::Mutex::new(None),
        });
        if config.performance_telemetry_enabled {
            let runtime_for_init = runtime.clone();
            let main_path = config.database_path.clone();
            let telemetry_path = config.performance_database_path.clone();
            let init_handle = tokio::spawn(async move {
                initialize_telemetry_runtime(runtime_for_init, main_path, telemetry_path).await;
            });
            if let Ok(mut slot) = runtime.init_handle.lock() {
                *slot = Some(init_handle);
            } else {
                init_handle.abort();
            }
        }
        if config.performance_telemetry_enabled {
            let writer_handle =
                tokio::spawn(run_telemetry_writer(runtime.clone(), receiver, epoch));
            if let Ok(mut slot) = runtime.writer_handle.lock() {
                *slot = Some(writer_handle);
            } else {
                writer_handle.abort();
            }
        }
        runtime
    }

    pub(crate) async fn shutdown_and_drain(&self) {
        let init_handle = self
            .init_handle
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        if let Some(init_handle) = init_handle {
            let _ = init_handle.await;
        }
        let writer_handle = self
            .writer_handle
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        if let Some(writer_handle) = writer_handle {
            let _ = writer_handle.await;
        }
    }

    pub(crate) fn record_counter(
        &self,
        metric_id: &'static str,
        dimension: &'static str,
        value: u64,
    ) {
        if value == 0 {
            return;
        }
        self.record(metric_id, dimension, EventKind::Counter, value as f64);
    }

    pub(crate) fn record_duration_ms(
        &self,
        metric_id: &'static str,
        dimension: &'static str,
        value: f64,
    ) {
        self.record(metric_id, dimension, EventKind::Duration, value);
    }

    pub(crate) fn record_gauge(
        &self,
        metric_id: &'static str,
        dimension: &'static str,
        value: f64,
    ) {
        self.record(metric_id, dimension, EventKind::Gauge, value);
    }

    pub(crate) fn http_request_started(&self) {
        if self.enabled {
            self.active_http_requests.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn http_request_finished(&self) {
        if self.enabled {
            self.active_http_requests.fetch_sub(1, Ordering::Relaxed);
        }
    }

    fn record(
        &self,
        metric_id: &'static str,
        dimension: &'static str,
        kind: EventKind,
        value: f64,
    ) {
        if !self.enabled {
            return;
        }
        let Some(spec) = metric_spec(metric_id) else {
            return;
        };
        let kind_matches = matches!(
            (spec.kind, kind),
            (MetricKind::Counter, EventKind::Counter)
                | (MetricKind::Duration, EventKind::Duration)
                | (MetricKind::Gauge, EventKind::Gauge)
        );
        if !value.is_finite()
            || value < 0.0
            || !kind_matches
            || !metric_dimension_allowed(metric_id, dimension)
        {
            return;
        }
        if self
            .sender
            .try_send(TelemetryEvent {
                metric_id,
                dimension,
                kind,
                value,
                weight_seconds: if matches!(kind, EventKind::Gauge) {
                    gauge_weight_seconds(metric_id)
                } else {
                    0.0
                },
                at: Utc::now(),
            })
            .is_err()
        {
            self.note_dropped_expectation(metric_id, dimension, Utc::now(), 1);
            self.dropped_samples.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn note_dropped_expectation(
        &self,
        metric_id: &'static str,
        dimension: &'static str,
        at: DateTime<Utc>,
        count: u64,
    ) {
        let key = (at.timestamp().div_euclid(60) * 60, metric_id, dimension);
        if let Ok(mut dropped) = self.dropped_expectations.lock()
            && (dropped.len() < TELEMETRY_QUEUE_CAPACITY || dropped.contains_key(&key))
        {
            let entry = dropped.entry(key).or_default();
            *entry = entry.saturating_add(count);
        }
    }

    fn take_dropped_expectations(&self) -> HashMap<(i64, &'static str, &'static str), u64> {
        self.dropped_expectations
            .lock()
            .map(|mut dropped| std::mem::take(&mut *dropped))
            .unwrap_or_default()
    }

    pub(crate) async fn health_snapshot(&self) -> PerformanceTelemetryHealth {
        let mut health = self.health.lock().await.clone();
        health.queue_depth = TELEMETRY_QUEUE_CAPACITY.saturating_sub(self.sender.capacity());
        health.dropped_samples = self.dropped_samples.load(Ordering::Relaxed);
        health.flush_failure_count = self.flush_failure_count.load(Ordering::Relaxed);
        health
    }

    async fn update_health(&self, update: impl FnOnce(&mut PerformanceTelemetryHealth)) {
        let mut health = self.health.lock().await;
        update(&mut health);
        health.queue_depth = TELEMETRY_QUEUE_CAPACITY.saturating_sub(self.sender.capacity());
        health.dropped_samples = self.dropped_samples.load(Ordering::Relaxed);
        health.flush_failure_count = self.flush_failure_count.load(Ordering::Relaxed);
    }

    async fn allow_browser_ingest(&self, client: &str) -> bool {
        self.browser_rate_limiter.lock().await.allow(client)
    }
}

fn metric_spec(metric_id: &str) -> Option<&'static MetricSpec> {
    METRIC_SPECS.iter().find(|spec| spec.id == metric_id)
}

fn metric_dimension_allowed(metric_id: &str, dimension: &str) -> bool {
    match metric_id {
        "http.in_flight" => dimension == "other",
        id if id.starts_with("p1.") => dimension == "p1",
        id if id.starts_with("p2.") => dimension == "p2",
        "sqlite.coordinator_waiters" | "sqlite.coordinator_wait_duration_ms" => matches!(
            dimension,
            "p1_terminal" | "interactive_proxy" | "p2_derived" | "maintenance_retention"
        ),
        "sqlite.coordinator_bypass_count" | "sqlite.maintenance_fairness_count" => {
            dimension == "coordinator"
        }
        id if id.starts_with("sqlite.") => dimension == "main",
        "projection.reconcile_defer_count" => {
            matches!(dimension, "writer_pressure" | "background_busy")
        }
        id if id.starts_with("projection.") => {
            matches!(dimension, "dashboard" | "current" | "network" | "terminal")
        }
        "sse.active_subscribers" => dimension == "dashboard",
        id if id.starts_with("sse.") => {
            matches!(dimension, "dashboard" | "current" | "network" | "terminal")
        }
        id if id.starts_with("maintenance.") => dimension == "maintenance",
        id if id.starts_with("process.") => dimension == "process",
        "storage.main_db_bytes" => dimension == "main_db",
        "storage.telemetry_db_bytes" => dimension == "telemetry_db",
        "storage.telemetry_wal_bytes" => dimension == "telemetry_wal",
        id if id.starts_with("telemetry.") => dimension == "collector",
        id if id.starts_with("browser.") => matches!(
            dimension,
            "dashboard"
                | "records"
                | "system"
                | "dashboard:mobile"
                | "dashboard:desktop"
                | "records:mobile"
                | "records:desktop"
                | "system:mobile"
                | "system:desktop"
        ),
        _ => false,
    }
}

fn registered_metric_dimension(
    metric_id: &str,
    dimension: &str,
) -> Option<(&'static str, &'static str)> {
    let spec = metric_spec(metric_id)?;
    metric_dimensions(spec.id)
        .iter()
        .copied()
        .find(|candidate| *candidate == dimension)
        .map(|dimension| (spec.id, dimension))
}

fn metric_dimensions(metric_id: &str) -> &'static [&'static str] {
    match metric_id {
        "http.in_flight" => &["other"],
        id if id.starts_with("p1.") => &["p1"],
        id if id.starts_with("p2.") => &["p2"],
        "sqlite.coordinator_waiters" | "sqlite.coordinator_wait_duration_ms" => &[
            "p1_terminal",
            "interactive_proxy",
            "p2_derived",
            "maintenance_retention",
        ],
        "sqlite.coordinator_bypass_count" | "sqlite.maintenance_fairness_count" => &["coordinator"],
        id if id.starts_with("sqlite.") => &["main"],
        "projection.last_good_age_ms"
        | "projection.build_count"
        | "projection.live_db_read_count" => &["dashboard"],
        "projection.cadence_miss_count"
        | "projection.revision_count"
        | "projection.publish_duration_ms"
        | "projection.publish_count"
        | "projection.snapshot_bytes" => &["current", "network", "terminal"],
        "projection.reconcile_duration_ms" | "projection.reconcile_count" => &["dashboard"],
        "projection.reconcile_defer_count" => &["writer_pressure", "background_busy"],
        "projection.reconcile_failure_count" => &["dashboard"],
        "sse.active_subscribers" => &["dashboard"],
        "sse.publish_error_count" | "sse.publish_duration_ms" | "sse.frame_bytes" => {
            &["current", "network", "terminal"]
        }
        id if id.starts_with("maintenance.") => &["maintenance"],
        id if id.starts_with("process.") => &["process"],
        "storage.main_db_bytes" => &["main_db"],
        "storage.telemetry_db_bytes" => &["telemetry_db"],
        "storage.telemetry_wal_bytes" => &["telemetry_wal"],
        id if id.starts_with("telemetry.") => &["collector"],
        "browser.data_ready_ms" | "browser.update_to_paint_ms" => &[
            "dashboard:mobile",
            "dashboard:desktop",
            "records:mobile",
            "records:desktop",
            "system:mobile",
            "system:desktop",
        ],
        id if id.starts_with("browser.") => &["dashboard", "records", "system"],
        _ => &[],
    }
}

fn empty_performance_series(spec: &MetricSpec, dimension: &str) -> PerformanceSeries {
    PerformanceSeries {
        metric_id: spec.id.to_string(),
        section: spec.section.to_string(),
        dimension: dimension.to_string(),
        kind: match spec.kind {
            MetricKind::Counter => "counter",
            MetricKind::Duration => "duration",
            MetricKind::Gauge => "gauge",
        }
        .to_string(),
        unit: if matches!(spec.kind, MetricKind::Duration) || spec.id.ends_with("_ms") {
            "milliseconds"
        } else if spec.id.contains("_bytes") {
            "bytes"
        } else if spec.id.ends_with("_percent") {
            "percent"
        } else {
            "count"
        }
        .to_string(),
        points: Vec::new(),
    }
}

fn gauge_weight_seconds(metric_id: &str) -> f64 {
    match metric_id {
        "p1.queue_depth"
        | "p1.queue_bytes"
        | "p2.queue_depth"
        | "p2.next_attempt_delay_ms"
        | "p2.deferred_age_ms"
        | "projection.last_good_age_ms"
        | "sse.active_subscribers"
        | "maintenance.backlog_age_ms"
        | "telemetry.queue_depth"
        | "http.in_flight"
        | "sqlite.coordinator_waiters"
        | "process.cpu_percent" => 10.0,
        "process.rss_bytes"
        | "process.rss_anon_bytes"
        | "process.swap_bytes"
        | "process.managed_bytes"
        | "process.unattributed_anon_bytes"
        | "storage.main_db_bytes"
        | "storage.telemetry_db_bytes"
        | "storage.telemetry_wal_bytes"
        | "process.thread_count"
        | "process.disk_free_bytes"
        | "sqlite.wal_bytes"
        | "maintenance.raw_bytes_before"
        | "maintenance.raw_bytes_after"
        | "projection.snapshot_bytes" => 30.0,
        _ => 0.0,
    }
}

fn telemetry_paths_conflict(main_path: &Path, telemetry_path: &Path) -> bool {
    fn normalize_path(path: &Path) -> PathBuf {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        };
        let mut unresolved = Vec::new();
        let mut probe = absolute.clone();
        while !probe.exists() {
            let Some(name) = probe.file_name().map(ToOwned::to_owned) else {
                break;
            };
            unresolved.push(name);
            if !probe.pop() {
                break;
            }
        }
        let mut normalized = std::fs::canonicalize(&probe).unwrap_or(probe);
        for name in unresolved.into_iter().rev() {
            normalized.push(name);
        }
        normalized
    }

    fn sqlite_path_family(path: &Path) -> [PathBuf; 3] {
        let mut wal = path.to_path_buf();
        let mut shm = path.to_path_buf();
        let file_name = path.file_name().unwrap_or_default().to_string_lossy();
        wal.set_file_name(format!("{file_name}-wal"));
        shm.set_file_name(format!("{file_name}-shm"));
        [path.to_path_buf(), wal, shm]
    }

    let main_family = sqlite_path_family(main_path).map(|path| normalize_path(&path));
    let telemetry_family = sqlite_path_family(telemetry_path).map(|path| normalize_path(&path));
    if main_family
        .iter()
        .any(|main| telemetry_family.iter().any(|telemetry| main == telemetry))
    {
        return true;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        for main in sqlite_path_family(main_path) {
            for telemetry in sqlite_path_family(telemetry_path) {
                if let (Ok(main_metadata), Ok(telemetry_metadata)) =
                    (std::fs::metadata(&main), std::fs::metadata(&telemetry))
                    && main_metadata.dev() == telemetry_metadata.dev()
                    && main_metadata.ino() == telemetry_metadata.ino()
                {
                    return true;
                }
            }
        }
    }
    false
}

async fn open_telemetry_pool(path: &Path) -> Result<Pool<Sqlite>> {
    let url = format!("sqlite://{}", path.to_string_lossy());
    let options = build_sqlite_connect_options(&url, Duration::from_secs(5))?;
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .context("failed to open performance telemetry database")
}

async fn initialize_telemetry_runtime(
    runtime: Arc<PerformanceTelemetryRuntime>,
    main_path: PathBuf,
    telemetry_path: PathBuf,
) {
    if telemetry_paths_conflict(&main_path, &telemetry_path) {
        runtime
            .update_health(|health| {
                health.state = "unavailable".to_string();
                health.last_error =
                    Some("performance database path must differ from main database".to_string());
            })
            .await;
        return;
    }
    if let Err(error) = ensure_db_directory(&telemetry_path) {
        runtime
            .update_health(|health| {
                health.state = "unavailable".to_string();
                health.last_error = Some(error.to_string());
            })
            .await;
        return;
    }
    let pool = match open_telemetry_pool(&telemetry_path).await {
        Ok(pool) => pool,
        Err(error) => {
            runtime
                .update_health(|health| {
                    health.state = "unavailable".to_string();
                    health.last_error = Some(error.to_string());
                })
                .await;
            return;
        }
    };
    if let Err(error) = ensure_telemetry_schema(&pool).await {
        runtime
            .update_health(|health| {
                health.state = "unavailable".to_string();
                health.last_error = Some(error.to_string());
            })
            .await;
        return;
    }
    if let Err(error) = reset_persisted_collector_health(&pool).await {
        runtime
            .update_health(|health| {
                health.state = "unavailable".to_string();
                health.last_error = Some(error.to_string());
            })
            .await;
        return;
    }
    if runtime.shutdown.is_cancelled() {
        return;
    }
    let epoch = runtime.health.lock().await.epoch.clone();
    if let Err(error) = sqlx::query(
        "INSERT INTO performance_epochs(epoch, started_at) VALUES (?, ?)
         ON CONFLICT(epoch) DO NOTHING",
    )
    .bind(&epoch)
    .bind(Utc::now().to_rfc3339())
    .execute(&pool)
    .await
    {
        runtime
            .update_health(|health| {
                health.state = "unavailable".to_string();
                health.last_error = Some(error.to_string());
            })
            .await;
        return;
    }
    runtime
        .update_health(|health| {
            health.state = "healthy".to_string();
            health.last_error = None;
        })
        .await;
    if let Err(error) = persist_collector_health(&runtime, &pool).await {
        runtime
            .update_health(|health| {
                health.state = "unavailable".to_string();
                health.last_error = Some(error.to_string());
            })
            .await;
        let _ = close_telemetry_epoch(&pool, &epoch).await;
        return;
    }
    if runtime.pool.set(pool.clone()).is_err() {
        runtime
            .update_health(|health| {
                health.state = "unavailable".to_string();
                health.last_error = Some("performance database initialized twice".to_string());
            })
            .await;
        let _ = close_telemetry_epoch(&pool, &epoch).await;
        return;
    }
    if runtime.shutdown.is_cancelled() {
        let _ = close_telemetry_epoch(&pool, &epoch).await;
    }
}

async fn ensure_telemetry_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let current_version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(pool)
        .await?;
    if current_version != 0 && current_version != TELEMETRY_SCHEMA_VERSION {
        bail!(
            "unrecognized performance telemetry schema marker {current_version}; expected final marker {TELEMETRY_SCHEMA_VERSION}; recreate the disposable telemetry database"
        );
    }
    let existing_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name LIKE 'performance_%'",
    )
    .fetch_one(pool)
    .await?;
    let existing_object_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE name NOT LIKE 'sqlite_%' AND type IN ('table', 'index', 'view', 'trigger')",
    )
    .fetch_one(pool)
    .await?;
    if current_version == 0 && existing_table_count > 0 {
        bail!(
            "partial performance telemetry schema found without final schema marker; recreate the disposable telemetry database"
        );
    }
    if current_version == 0 && existing_object_count > 0 {
        bail!(
            "unknown database schema at performance telemetry path; use an empty disposable database"
        );
    }
    if current_version == TELEMETRY_SCHEMA_VERSION {
        let unexpected_objects: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master
             WHERE name NOT LIKE 'sqlite_%'
               AND name NOT IN (
                    'performance_meta', 'performance_epochs', 'performance_buckets',
                    'performance_collector_health', 'idx_performance_buckets_range'
               )
             ORDER BY name",
        )
        .fetch_all(pool)
        .await?;
        if !unexpected_objects.is_empty() {
            bail!(
                "unexpected objects in marked performance telemetry schema ({}); recreate the disposable telemetry database",
                unexpected_objects.join(", ")
            );
        }
        let expected_tables: &[TelemetryTableContract] = &[
            (
                "performance_meta",
                &[("key", "TEXT", 0, 1), ("value", "TEXT", 1, 0)],
            ),
            (
                "performance_epochs",
                &[
                    ("epoch", "TEXT", 0, 1),
                    ("started_at", "TEXT", 1, 0),
                    ("ended_at", "TEXT", 0, 0),
                ],
            ),
            (
                "performance_buckets",
                &[
                    ("bucket_start", "INTEGER", 1, 1),
                    ("resolution_seconds", "INTEGER", 1, 2),
                    ("metric_id", "TEXT", 1, 3),
                    ("dimension_code", "TEXT", 1, 4),
                    ("sample_count", "INTEGER", 1, 0),
                    ("expected_count", "INTEGER", 1, 0),
                    ("sum_value", "REAL", 1, 0),
                    ("min_value", "REAL", 0, 0),
                    ("max_value", "REAL", 0, 0),
                    ("last_value", "REAL", 0, 0),
                    ("weighted_sum", "REAL", 1, 0),
                    ("weighted_seconds", "REAL", 1, 0),
                    ("histogram_json", "TEXT", 1, 0),
                    ("epoch", "TEXT", 1, 0),
                ],
            ),
            (
                "performance_collector_health",
                &[
                    ("id", "INTEGER", 0, 1),
                    ("state", "TEXT", 1, 0),
                    ("last_successful_flush", "TEXT", 0, 0),
                    ("dropped_samples", "INTEGER", 1, 0),
                    ("flush_failure_count", "INTEGER", 1, 0),
                    ("last_error", "TEXT", 0, 0),
                ],
            ),
        ];
        for (table, expected_columns) in expected_tables {
            let columns = sqlx::query(&format!("PRAGMA table_info(\"{table}\")"))
                .fetch_all(pool)
                .await?
                .into_iter()
                .map(|row| {
                    Ok::<_, sqlx::Error>((
                        row.try_get::<String, _>("name")?,
                        row.try_get::<String, _>("type")?,
                        row.try_get::<i64, _>("notnull")?,
                        row.try_get::<i64, _>("pk")?,
                    ))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let schema_matches = columns.len() == expected_columns.len()
                && columns
                    .iter()
                    .zip(expected_columns.iter())
                    .all(|(actual, expected)| {
                        actual.0 == expected.0
                            && actual.1 == expected.1
                            && actual.2 == expected.2
                            && actual.3 == expected.3
                    });
            if !schema_matches {
                bail!(
                    "malformed performance telemetry table {table}; recreate the disposable telemetry database"
                );
            }
        }
        let index_exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_performance_buckets_range'",
        )
        .fetch_one(pool)
        .await?;
        if index_exists != 1 {
            bail!(
                "malformed performance telemetry index; recreate the disposable telemetry database"
            );
        }
        let index_columns = sqlx::query("PRAGMA index_info(\"idx_performance_buckets_range\")")
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|row| {
                Ok::<_, sqlx::Error>((
                    row.try_get::<i64, _>("seqno")?,
                    row.try_get::<String, _>("name")?,
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if index_columns
            != [
                (0, "resolution_seconds".to_string()),
                (1, "bucket_start".to_string()),
            ]
        {
            bail!(
                "malformed performance telemetry range index; recreate the disposable telemetry database"
            );
        }
        let health_sql: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'performance_collector_health'",
        )
        .fetch_one(pool)
        .await?;
        let normalized_health_sql: String = health_sql
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>()
            .to_ascii_uppercase();
        if !normalized_health_sql.contains("CHECK(ID=1)") {
            bail!(
                "malformed performance collector health constraint; recreate the disposable telemetry database"
            );
        }
        return Ok(());
    }
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS performance_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS performance_epochs (epoch TEXT PRIMARY KEY, started_at TEXT NOT NULL, ended_at TEXT)",
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS performance_buckets (
            bucket_start INTEGER NOT NULL,
            resolution_seconds INTEGER NOT NULL,
            metric_id TEXT NOT NULL,
            dimension_code TEXT NOT NULL,
            sample_count INTEGER NOT NULL,
            expected_count INTEGER NOT NULL,
            sum_value REAL NOT NULL,
            min_value REAL,
            max_value REAL,
            last_value REAL,
            weighted_sum REAL NOT NULL DEFAULT 0,
            weighted_seconds REAL NOT NULL DEFAULT 0,
            histogram_json TEXT NOT NULL,
            epoch TEXT NOT NULL,
            PRIMARY KEY(bucket_start, resolution_seconds, metric_id, dimension_code)
        )",
    )
    .execute(&mut *transaction)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_performance_buckets_range ON performance_buckets(resolution_seconds, bucket_start)")
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS performance_collector_health (
            id INTEGER PRIMARY KEY CHECK(id = 1), state TEXT NOT NULL, last_successful_flush TEXT,
            dropped_samples INTEGER NOT NULL, flush_failure_count INTEGER NOT NULL, last_error TEXT
        )",
    )
    .execute(&mut *transaction)
    .await?;
    if current_version == 0 {
        let set_version = format!("PRAGMA user_version = {TELEMETRY_SCHEMA_VERSION}");
        sqlx::query(&set_version).execute(&mut *transaction).await?;
    }
    sqlx::query(
        "INSERT INTO performance_meta(key, value) VALUES ('schema_version', ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(TELEMETRY_SCHEMA_VERSION.to_string())
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn reset_persisted_collector_health(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        "INSERT INTO performance_collector_health(
            id, state, last_successful_flush, dropped_samples, flush_failure_count, last_error
        ) VALUES (1, 'starting', NULL, 0, 0, NULL)
        ON CONFLICT(id) DO UPDATE SET state = excluded.state,
            last_successful_flush = excluded.last_successful_flush,
            dropped_samples = excluded.dropped_samples,
            flush_failure_count = excluded.flush_failure_count,
            last_error = excluded.last_error",
    )
    .execute(pool)
    .await
    .context("failed to initialize performance collector health")?;
    Ok(())
}

async fn persist_collector_health(
    runtime: &PerformanceTelemetryRuntime,
    pool: &Pool<Sqlite>,
) -> Result<()> {
    let health = runtime.health_snapshot().await;
    sqlx::query(
        "INSERT INTO performance_collector_health(
            id, state, last_successful_flush, dropped_samples, flush_failure_count, last_error
        ) VALUES (1, ?, ?, ?, ?, ?)
        ON CONFLICT(id) DO UPDATE SET state = excluded.state,
            last_successful_flush = excluded.last_successful_flush,
            dropped_samples = excluded.dropped_samples,
            flush_failure_count = excluded.flush_failure_count,
            last_error = excluded.last_error",
    )
    .bind(health.state)
    .bind(health.last_successful_flush)
    .bind(health.dropped_samples as i64)
    .bind(health.flush_failure_count as i64)
    .bind(health.last_error)
    .execute(pool)
    .await
    .context("failed to persist performance collector health")?;
    Ok(())
}

async fn run_telemetry_writer(
    runtime: Arc<PerformanceTelemetryRuntime>,
    mut receiver: mpsc::Receiver<TelemetryEvent>,
    epoch: String,
) {
    let mut accumulators: HashMap<(i64, String, String), BucketAccumulator> = HashMap::new();
    let mut ticker = tokio::time::interval(TELEMETRY_FLUSH_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    ticker.tick().await;
    loop {
        tokio::select! {
            _ = runtime.shutdown.cancelled() => {
                let dropped = drain_events(&runtime, &mut receiver, &mut accumulators);
                runtime
                    .dropped_samples
                    .fetch_add(dropped, Ordering::Relaxed);
                merge_dropped_expectations(&runtime, &mut accumulators);
                if flush_accumulators(&runtime, &mut accumulators, &epoch)
                    .await
                    .is_err()
                {
                    let dropped = drop_accumulators_with_runtime(&runtime, &mut accumulators);
                    runtime
                        .dropped_samples
                        .fetch_add(dropped, Ordering::Relaxed);
                }
                if let Some(pool) = runtime.pool.get()
                    && let Err(error) = close_telemetry_epoch(pool, &epoch).await
                {
                    runtime.flush_failure_count.fetch_add(1, Ordering::Relaxed);
                    runtime
                        .update_health(|health| {
                            health.state = "degraded".to_string();
                            health.last_error = Some(error.to_string());
                        })
                        .await;
                }
                return;
            }
            event = receiver.recv() => match event {
                Some(event) => {
                    let event_at = event.at;
                    let event_metric_id = event.metric_id;
                    let event_dimension = event.dimension;
                    if !add_event(&mut accumulators, event) {
                        runtime.note_dropped_expectation(
                            event_metric_id,
                            event_dimension,
                            event_at,
                            1,
                        );
                        runtime.dropped_samples.fetch_add(1, Ordering::Relaxed);
                    }
                    if runtime.pool.get().is_none() {
                        let dropped =
                            trim_unflushed_accumulators_with_runtime(&runtime, &mut accumulators);
                        runtime.dropped_samples.fetch_add(dropped, Ordering::Relaxed);
                    }
                }
                None => return,
            },
            _ = ticker.tick() => {
                if !accumulators.is_empty()
                    || !runtime.dropped_expectations.lock().is_ok_and(|dropped| dropped.is_empty())
                {
                    merge_dropped_expectations(&runtime, &mut accumulators);
                }
                if !accumulators.is_empty()
                    && flush_accumulators(&runtime, &mut accumulators, &epoch)
                        .await
                        .is_err()
                {
                    let dropped = trim_unflushed_accumulators_with_runtime(&runtime, &mut accumulators);
                    runtime.dropped_samples.fetch_add(dropped, Ordering::Relaxed);
                }
                if let Some(pool) = runtime.pool.get() {
                    if let Err(error) = rollup_and_prune(pool).await {
                        runtime.flush_failure_count.fetch_add(1, Ordering::Relaxed);
                        runtime.update_health(|health| {
                            health.state = "degraded".to_string();
                            health.last_error = Some(error.to_string());
                        }).await;
                    } else if let Err(error) = checkpoint_telemetry(pool).await {
                        runtime.flush_failure_count.fetch_add(1, Ordering::Relaxed);
                        runtime.update_health(|health| {
                            health.state = "degraded".to_string();
                            health.last_error = Some(error.to_string());
                        }).await;
                    }
                }
            }
        }
    }
}

fn drain_events(
    runtime: &PerformanceTelemetryRuntime,
    receiver: &mut mpsc::Receiver<TelemetryEvent>,
    accumulators: &mut HashMap<(i64, String, String), BucketAccumulator>,
) -> u64 {
    let mut dropped = 0_u64;
    while let Ok(event) = receiver.try_recv() {
        let event_at = event.at;
        let event_metric_id = event.metric_id;
        let event_dimension = event.dimension;
        if !add_event(accumulators, event) {
            runtime.note_dropped_expectation(event_metric_id, event_dimension, event_at, 1);
            dropped = dropped.saturating_add(1);
        }
    }
    dropped
}

fn drop_accumulators_with_runtime(
    runtime: &PerformanceTelemetryRuntime,
    accumulators: &mut HashMap<(i64, String, String), BucketAccumulator>,
) -> u64 {
    let dropped = accumulators
        .values()
        .map(|accumulator| accumulator.count)
        .sum::<u64>();
    for ((bucket_start, metric_id, dimension), accumulator) in accumulators.iter() {
        if let Some((metric_id, dimension)) = registered_metric_dimension(metric_id, dimension) {
            runtime.note_dropped_expectation(
                metric_id,
                dimension,
                DateTime::from_timestamp(*bucket_start, 0).unwrap_or_else(Utc::now),
                accumulator.expected_count,
            );
        }
    }
    accumulators.clear();
    dropped
}

fn add_event(
    accumulators: &mut HashMap<(i64, String, String), BucketAccumulator>,
    event: TelemetryEvent,
) -> bool {
    let bucket_start = event.at.timestamp() / 60 * 60;
    let key = (
        bucket_start,
        event.metric_id.to_string(),
        event.dimension.to_string(),
    );
    if let Some(accumulator) = accumulators.get_mut(&key) {
        accumulator.add(event.value, event.weight_seconds);
        true
    } else if accumulators.len() < 512 {
        accumulators.insert(
            key,
            BucketAccumulator::new(event.kind, event.value, event.weight_seconds),
        );
        true
    } else {
        false
    }
}

fn merge_dropped_expectations(
    runtime: &PerformanceTelemetryRuntime,
    accumulators: &mut HashMap<(i64, String, String), BucketAccumulator>,
) {
    for ((bucket_start, metric_id, dimension), expected_count) in
        runtime.take_dropped_expectations()
    {
        let Some(spec) = metric_spec(metric_id) else {
            continue;
        };
        let key = (bucket_start, metric_id.to_string(), dimension.to_string());
        if let Some(accumulator) = accumulators.get_mut(&key) {
            accumulator.add_expected_only(expected_count);
        } else if accumulators.len() < 512 {
            let mut accumulator = BucketAccumulator {
                kind: match spec.kind {
                    MetricKind::Counter => EventKind::Counter,
                    MetricKind::Duration => EventKind::Duration,
                    MetricKind::Gauge => EventKind::Gauge,
                },
                count: 0,
                expected_count: 0,
                sum: 0.0,
                min: 0.0,
                max: 0.0,
                last: 0.0,
                weighted_sum: 0.0,
                weighted_seconds: 0.0,
                histogram: [0; 8],
            };
            accumulator.add_expected_only(expected_count);
            accumulators.insert(key, accumulator);
        } else {
            runtime
                .dropped_samples
                .fetch_add(expected_count, Ordering::Relaxed);
        }
    }
}

fn trim_unflushed_accumulators(
    accumulators: &mut HashMap<(i64, String, String), BucketAccumulator>,
) -> u64 {
    trim_unflushed_accumulators_with(accumulators, |_, _, _, _| {})
}

fn trim_unflushed_accumulators_with_runtime(
    runtime: &PerformanceTelemetryRuntime,
    accumulators: &mut HashMap<(i64, String, String), BucketAccumulator>,
) -> u64 {
    trim_unflushed_accumulators_with(
        accumulators,
        |bucket_start, metric_id, dimension, expected_count| {
            if expected_count == 0 {
                return;
            }
            runtime.note_dropped_expectation(
                metric_id,
                dimension,
                DateTime::from_timestamp(bucket_start, 0).unwrap_or_else(Utc::now),
                expected_count,
            );
        },
    )
}

fn trim_unflushed_accumulators_with(
    accumulators: &mut HashMap<(i64, String, String), BucketAccumulator>,
    mut on_drop: impl FnMut(i64, &'static str, &'static str, u64),
) -> u64 {
    let mut bucket_starts = accumulators.keys().map(|key| key.0).collect::<Vec<_>>();
    bucket_starts.sort_unstable();
    bucket_starts.dedup();
    if bucket_starts.len() <= TELEMETRY_MAX_PENDING_MINUTE_BUCKETS {
        return 0;
    }
    let cutoff = bucket_starts[bucket_starts.len() - TELEMETRY_MAX_PENDING_MINUTE_BUCKETS];
    let stale_keys = accumulators
        .keys()
        .filter(|key| key.0 < cutoff)
        .cloned()
        .collect::<Vec<_>>();
    let dropped = stale_keys
        .iter()
        .filter_map(|key| accumulators.get(key))
        .map(|accumulator| accumulator.count)
        .sum();
    for key in stale_keys {
        if let Some((metric_id, dimension)) = registered_metric_dimension(&key.1, &key.2) {
            let expected_count = accumulators
                .get(&key)
                .map(|accumulator| accumulator.expected_count)
                .unwrap_or_default();
            on_drop(key.0, metric_id, dimension, expected_count);
        }
        accumulators.remove(&key);
    }
    dropped
}

async fn flush_accumulators(
    runtime: &PerformanceTelemetryRuntime,
    accumulators: &mut HashMap<(i64, String, String), BucketAccumulator>,
    epoch: &str,
) -> Result<()> {
    let Some(pool) = runtime.pool.get() else {
        let state = runtime.health.lock().await.state.clone();
        if state == "starting" && !runtime.shutdown.is_cancelled() {
            return Ok(());
        }
        let dropped = accumulators
            .values()
            .map(|accumulator| accumulator.count)
            .sum::<u64>();
        for ((bucket_start, metric_id, dimension), accumulator) in accumulators.iter() {
            if let Some((metric_id, dimension)) = registered_metric_dimension(metric_id, dimension)
            {
                runtime.note_dropped_expectation(
                    metric_id,
                    dimension,
                    DateTime::from_timestamp(*bucket_start, 0).unwrap_or_else(Utc::now),
                    accumulator.expected_count,
                );
            }
        }
        runtime
            .dropped_samples
            .fetch_add(dropped, Ordering::Relaxed);
        accumulators.clear();
        return Ok(());
    };
    let batch_events = accumulators
        .values()
        .map(|accumulator| accumulator.count)
        .sum::<u64>();
    let batch_buckets = accumulators.len() as u64;
    let started = Instant::now();
    let result = async {
        let mut transaction = pool.begin().await?;
        for ((bucket_start, metric_id, dimension), accumulator) in accumulators.iter() {
            let existing = sqlx::query(
                "SELECT sample_count, expected_count, sum_value, min_value, max_value, last_value,
                        weighted_sum, weighted_seconds, histogram_json, epoch
                 FROM performance_buckets
                 WHERE bucket_start = ? AND resolution_seconds = 60 AND metric_id = ? AND dimension_code = ?",
            )
            .bind(*bucket_start)
            .bind(metric_id)
            .bind(dimension)
            .fetch_optional(&mut *transaction)
            .await?;
            let (
                mut count,
                mut expected,
                mut sum,
                mut min,
                mut max,
                mut last,
                mut weighted_sum,
                mut weighted_seconds,
                mut histogram,
                mut epochs,
            ) = if let Some(row) = existing {
                    let parsed = serde_json::from_str::<Vec<u64>>(row.try_get::<String, _>("histogram_json")?.as_str()).unwrap_or_else(|_| vec![0; 8]);
                    let mut epochs = BTreeSet::new();
                    record_epoch_values(&mut epochs, row.try_get::<String, _>("epoch")?.as_str());
                    (row.try_get::<i64, _>("sample_count")? as u64, row.try_get::<i64, _>("expected_count")? as u64, row.try_get::<f64, _>("sum_value")?, row.try_get::<Option<f64>, _>("min_value")?, row.try_get::<Option<f64>, _>("max_value")?, row.try_get::<Option<f64>, _>("last_value")?, row.try_get::<f64, _>("weighted_sum")?, row.try_get::<f64, _>("weighted_seconds")?, parsed, epochs)
                } else {
                    (0, 0, 0.0, None, None, None, 0.0, 0.0, vec![0; 8], BTreeSet::new())
                };
            epochs.insert(epoch.to_string());
            count = count.saturating_add(accumulator.count);
            expected = expected.saturating_add(accumulator.expected_count);
            sum += accumulator.sum;
            if accumulator.count > 0 {
                min = Some(min.map_or(accumulator.min, |value| value.min(accumulator.min)));
                max = Some(max.map_or(accumulator.max, |value| value.max(accumulator.max)));
                last = Some(accumulator.last);
            }
            weighted_sum += accumulator.weighted_sum;
            weighted_seconds += accumulator.weighted_seconds;
            for (index, value) in accumulator.histogram.iter().enumerate() {
                if index < histogram.len() { histogram[index] = histogram[index].saturating_add(*value); }
            }
            let histogram_json = serde_json::to_string(&histogram)?;
            let encoded_epoch = if epochs.len() == 1 {
                epochs.into_iter().next().expect("epoch set is non-empty")
            } else {
                serde_json::to_string(&epochs.into_iter().collect::<Vec<_>>())?
            };
            sqlx::query(
                "INSERT INTO performance_buckets(
                    bucket_start, resolution_seconds, metric_id, dimension_code,
                    sample_count, expected_count, sum_value, min_value, max_value, last_value,
                    weighted_sum, weighted_seconds, histogram_json, epoch
                ) VALUES (?, 60, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(bucket_start, resolution_seconds, metric_id, dimension_code)
                DO UPDATE SET sample_count=excluded.sample_count,
                    expected_count=excluded.expected_count, sum_value=excluded.sum_value,
                    min_value=excluded.min_value, max_value=excluded.max_value,
                    last_value=excluded.last_value, weighted_sum=excluded.weighted_sum,
                    weighted_seconds=excluded.weighted_seconds,
                    histogram_json=excluded.histogram_json, epoch=excluded.epoch",
            )
            .bind(*bucket_start)
            .bind(metric_id)
            .bind(dimension)
            .bind(count as i64)
            .bind(expected as i64)
            .bind(sum)
            .bind(min)
            .bind(max)
            .bind(last)
            .bind(weighted_sum)
            .bind(weighted_seconds)
            .bind(histogram_json)
            .bind(encoded_epoch)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query("INSERT INTO performance_epochs(epoch, started_at) VALUES (?, ?) ON CONFLICT(epoch) DO NOTHING")
            .bind(epoch).bind(Utc::now().to_rfc3339()).execute(&mut *transaction).await?;
        transaction.commit().await?;
        Ok::<(), anyhow::Error>(())
    }.await;
    match result {
        Ok(()) => {
            accumulators.clear();
            runtime
                .update_health(|health| {
                    health.state = "healthy".to_string();
                    health.last_successful_flush = Some(Utc::now().to_rfc3339());
                    health.last_error = None;
                })
                .await;
            if let Some(pool) = runtime.pool.get() {
                let _ = persist_collector_health(runtime, pool).await;
            }
            runtime.record_duration_ms(
                "telemetry.flush_duration_ms",
                "collector",
                started.elapsed().as_secs_f64() * 1000.0,
            );
            runtime.record_counter("telemetry.flush_batch_events", "collector", batch_events);
            runtime.record_counter("telemetry.flush_batch_buckets", "collector", batch_buckets);
            Ok(())
        }
        Err(error) => {
            runtime.flush_failure_count.fetch_add(1, Ordering::Relaxed);
            runtime.record_counter("telemetry.backoff_count", "collector", 1);
            runtime
                .update_health(|health| {
                    health.state = "degraded".to_string();
                    health.last_error = Some(error.to_string());
                })
                .await;
            if let Some(pool) = runtime.pool.get() {
                let _ = persist_collector_health(runtime, pool).await;
            }
            Err(error)
        }
    }
}

async fn close_telemetry_epoch(pool: &Pool<Sqlite>, epoch: &str) -> Result<()> {
    sqlx::query("UPDATE performance_epochs SET ended_at = ? WHERE epoch = ?")
        .bind(Utc::now().to_rfc3339())
        .bind(epoch)
        .execute(pool)
        .await
        .context("failed to close performance telemetry epoch")?;
    Ok(())
}

async fn rollup_and_prune(pool: &Pool<Sqlite>) -> Result<()> {
    let now = Utc::now().timestamp();
    let seven_days = now - 7 * 24 * 60 * 60;
    let thirty_days = now - 30 * 24 * 60 * 60;
    let thirteen_months = now - TELEMETRY_RETENTION_HOURS * 60 * 60;
    rollup_resolution(pool, 60, 300, seven_days).await?;
    rollup_resolution(pool, 300, 3600, thirty_days).await?;
    sqlx::query(
        "DELETE FROM performance_buckets WHERE resolution_seconds = 3600 AND bucket_start < ?",
    )
    .bind(thirteen_months)
    .execute(pool)
    .await?;
    Ok(())
}

async fn checkpoint_telemetry(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(pool)
        .await
        .context("failed to checkpoint performance telemetry WAL")?;
    Ok(())
}

async fn rollup_resolution(
    pool: &Pool<Sqlite>,
    source: i64,
    target: i64,
    source_before: i64,
) -> Result<()> {
    // Only roll complete target buckets. A moving cutoff can otherwise merge a partial
    // target bucket first and overwrite it with the remaining source rows on the next run.
    let aligned_source_before = source_before.div_euclid(target) * target;
    #[derive(Debug)]
    struct RollupAccumulator {
        sample_count: u64,
        expected_count: u64,
        sum_value: f64,
        min_value: Option<f64>,
        max_value: Option<f64>,
        last_value: Option<f64>,
        weighted_sum: f64,
        weighted_seconds: f64,
        last_bucket: i64,
        histogram: Vec<u64>,
        epochs: BTreeSet<String>,
    }

    let mut transaction = pool.begin().await?;
    let source_rows = sqlx::query(
        "SELECT bucket_start, metric_id, dimension_code, sample_count, expected_count,
                sum_value, min_value, max_value, last_value, weighted_sum, weighted_seconds,
                histogram_json, epoch
         FROM performance_buckets
         WHERE resolution_seconds = ? AND bucket_start < ?
         ORDER BY bucket_start",
    )
    .bind(source)
    .bind(aligned_source_before)
    .fetch_all(&mut *transaction)
    .await?;
    let mut grouped: HashMap<(i64, String, String), RollupAccumulator> = HashMap::new();
    for row in source_rows {
        let bucket_start: i64 = row.try_get("bucket_start")?;
        let metric_id: String = row.try_get("metric_id")?;
        let Some(spec) = metric_spec(&metric_id) else {
            continue;
        };
        if target >= 3600 && !spec.long_term {
            continue;
        }
        let dimension: String = row.try_get("dimension_code")?;
        if !metric_dimension_allowed(&metric_id, &dimension) {
            continue;
        }
        let key = ((bucket_start / target) * target, metric_id, dimension);
        let histogram =
            serde_json::from_str::<Vec<u64>>(row.try_get::<String, _>("histogram_json")?.as_str())
                .unwrap_or_else(|_| vec![0; 8]);
        let entry = grouped.entry(key).or_insert_with(|| RollupAccumulator {
            sample_count: 0,
            expected_count: 0,
            sum_value: 0.0,
            min_value: None,
            max_value: None,
            last_value: None,
            weighted_sum: 0.0,
            weighted_seconds: 0.0,
            last_bucket: i64::MIN,
            histogram: vec![0; 8],
            epochs: BTreeSet::new(),
        });
        let encoded_epoch: String = row.try_get("epoch")?;
        record_epoch_values(&mut entry.epochs, &encoded_epoch);
        entry.sample_count = entry
            .sample_count
            .saturating_add(row.try_get::<i64, _>("sample_count")?.max(0) as u64);
        entry.expected_count = entry
            .expected_count
            .saturating_add(row.try_get::<i64, _>("expected_count")?.max(0) as u64);
        entry.sum_value += row.try_get::<f64, _>("sum_value")?;
        entry.weighted_sum += row.try_get::<f64, _>("weighted_sum")?;
        entry.weighted_seconds += row.try_get::<f64, _>("weighted_seconds")?;
        let min_value = row.try_get::<Option<f64>, _>("min_value")?;
        let max_value = row.try_get::<Option<f64>, _>("max_value")?;
        entry.min_value = match (entry.min_value, min_value) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (None, value) => value,
            (value, None) => value,
        };
        entry.max_value = match (entry.max_value, max_value) {
            (Some(left), Some(right)) => Some(left.max(right)),
            (None, value) => value,
            (value, None) => value,
        };
        if bucket_start >= entry.last_bucket {
            entry.last_bucket = bucket_start;
            entry.last_value = row.try_get("last_value")?;
        }
        for (index, value) in histogram.into_iter().enumerate().take(8) {
            entry.histogram[index] = entry.histogram[index].saturating_add(value);
        }
    }
    for ((bucket_start, metric_id, dimension), entry) in grouped {
        let histogram_json = serde_json::to_string(&entry.histogram)?;
        let epoch = serde_json::to_string(&entry.epochs.into_iter().collect::<Vec<_>>())?;
        sqlx::query(
            "INSERT INTO performance_buckets(bucket_start, resolution_seconds, metric_id,
                dimension_code, sample_count, expected_count, sum_value, min_value, max_value,
                last_value, weighted_sum, weighted_seconds, histogram_json, epoch)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(bucket_start, resolution_seconds, metric_id, dimension_code)
             DO UPDATE SET sample_count=excluded.sample_count,
                expected_count=excluded.expected_count, sum_value=excluded.sum_value,
                min_value=excluded.min_value, max_value=excluded.max_value,
                last_value=excluded.last_value, weighted_sum=excluded.weighted_sum,
                weighted_seconds=excluded.weighted_seconds,
                histogram_json=excluded.histogram_json,
                epoch=excluded.epoch",
        )
        .bind(bucket_start)
        .bind(target)
        .bind(metric_id)
        .bind(dimension)
        .bind(entry.sample_count as i64)
        .bind(entry.expected_count as i64)
        .bind(entry.sum_value)
        .bind(entry.min_value)
        .bind(entry.max_value)
        .bind(entry.last_value)
        .bind(entry.weighted_sum)
        .bind(entry.weighted_seconds)
        .bind(histogram_json)
        .bind(epoch)
        .execute(&mut *transaction)
        .await?;
    }
    sqlx::query(
        "DELETE FROM performance_buckets WHERE resolution_seconds = ? AND bucket_start < ?",
    )
    .bind(source)
    .bind(aligned_source_before)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

pub(crate) fn spawn_performance_telemetry_sampler(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(async move {
        if !state.config.performance_telemetry_enabled {
            return;
        }
        let mut ticker = tokio::time::interval(TELEMETRY_SAMPLE_INTERVAL);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut resource_tick = 0u8;
        let mut cursors = SamplerCursors::default();
        loop {
            tokio::select! {
                _ = state.shutdown.cancelled() => return,
                _ = ticker.tick() => {
                    resource_tick = resource_tick.wrapping_add(1);
                    sample_runtime_health(&state, &mut cursors).await;
                    if resource_tick.is_multiple_of(3) { sample_process_health(&state).await; }
                }
            }
        }
    })
}

#[derive(Debug, Default)]
struct SamplerCursors {
    p1_ack_sequence: u64,
    p1_retry_count: u64,
    p2_retry_count: u64,
    p1_transfer_bytes: u64,
    p2_pressure_defer_count: u64,
    p2_lock_retry_count: u64,
    p2_flush_attempt_count: u64,
    sqlite_busy_count: u64,
    sqlite_locked_count: u64,
    sqlite_pool_timeout_count: u64,
    sqlite_background_skip_count: u64,
    telemetry_dropped_samples: u64,
    telemetry_flush_failure_count: u64,
    projection_build_count: u64,
    projection_live_db_read_count: u64,
    projection_current_cadence_miss_count: u64,
    projection_network_cadence_miss_count: u64,
    projection_terminal_cadence_miss_count: u64,
    projection_current_revision_count: u64,
    projection_network_revision_count: u64,
    projection_terminal_revision_count: u64,
    write_batch_count: u64,
    write_rows: u64,
    write_bytes: u64,
    coordinator_p1_waiter_admissions: u64,
    coordinator_p1_waiter_total_ms: u64,
    coordinator_interactive_admissions: u64,
    coordinator_interactive_total_ms: u64,
    coordinator_p2_admissions: u64,
    coordinator_p2_total_ms: u64,
    coordinator_maintenance_admissions: u64,
    coordinator_maintenance_total_ms: u64,
    coordinator_bypass_count: u64,
    coordinator_fairness_count: u64,
    cpu_process_ticks: u64,
    cpu_total_ticks: u64,
}

fn counter_delta(current: u64, previous: &mut u64) -> u64 {
    let delta = current.saturating_sub(*previous);
    *previous = current;
    delta
}

fn record_coordinator_wait_average(
    telemetry: &PerformanceTelemetryRuntime,
    dimension: &'static str,
    admission_count: u64,
    total_wait_ms: u64,
    previous_count: &mut u64,
    previous_total_ms: &mut u64,
) {
    let count_delta = counter_delta(admission_count, previous_count);
    let total_delta = counter_delta(total_wait_ms, previous_total_ms);
    if count_delta > 0 {
        telemetry.record_duration_ms(
            "sqlite.coordinator_wait_duration_ms",
            dimension,
            total_delta as f64 / count_delta as f64,
        );
    }
}

async fn sample_runtime_health(state: &AppState, cursors: &mut SamplerCursors) {
    let telemetry = &state.performance_telemetry;
    telemetry.record_gauge(
        "http.in_flight",
        "other",
        telemetry.active_http_requests.load(Ordering::Relaxed) as f64,
    );
    if let Some((process_ticks, total_ticks)) = read_process_cpu_ticks() {
        if cursors.cpu_total_ticks > 0 {
            let process_delta = process_ticks.saturating_sub(cursors.cpu_process_ticks);
            let total_delta = total_ticks.saturating_sub(cursors.cpu_total_ticks);
            if total_delta > 0 {
                let cpu_count = std::thread::available_parallelism()
                    .map(|value| value.get() as f64)
                    .unwrap_or(1.0);
                telemetry.record_gauge(
                    "process.cpu_percent",
                    "process",
                    (process_delta as f64 / total_delta as f64 * cpu_count * 100.0).min(100.0),
                );
            }
        }
        cursors.cpu_process_ticks = process_ticks;
        cursors.cpu_total_ticks = total_ticks;
    }
    let accounting = state.sqlite_batch_writer.accounting_snapshot();
    telemetry.record_gauge("p1.queue_depth", "p1", accounting.pending_depth as f64);
    telemetry.record_gauge("p1.queue_bytes", "p1", accounting.pending_bytes as f64);
    telemetry.record_gauge(
        "p2.queue_depth",
        "p2",
        accounting
            .pending_depth
            .saturating_sub(state.sqlite_batch_writer.queued_p1_count()) as f64,
    );
    if accounting.p1_ack_sequence > cursors.p1_ack_sequence {
        cursors.p1_ack_sequence = accounting.p1_ack_sequence;
        telemetry.record_duration_ms(
            "p1.ack_duration_ms",
            "p1",
            accounting.p1_ack_duration_ms as f64,
        );
    }
    telemetry.record_counter(
        "p1.retry_count",
        "p1",
        counter_delta(accounting.p1_retry_count, &mut cursors.p1_retry_count),
    );
    telemetry.record_counter(
        "p2.retry_count",
        "p2",
        counter_delta(accounting.p2_retry_count, &mut cursors.p2_retry_count),
    );
    telemetry.record_counter(
        "p1.transfer_bytes",
        "p1",
        counter_delta(
            accounting.transfer_bytes as u64,
            &mut cursors.p1_transfer_bytes,
        ),
    );
    telemetry.record_counter(
        "p2.pressure_defer_count",
        "p2",
        counter_delta(
            accounting.p2_pressure_defer_count,
            &mut cursors.p2_pressure_defer_count,
        ),
    );
    telemetry.record_counter(
        "p2.lock_retry_count",
        "p2",
        counter_delta(
            accounting.p2_lock_retry_count,
            &mut cursors.p2_lock_retry_count,
        ),
    );
    telemetry.record_gauge(
        "p2.next_attempt_delay_ms",
        "p2",
        accounting.p2_next_attempt_in_ms as f64,
    );
    telemetry.record_gauge(
        "p2.deferred_age_ms",
        "p2",
        accounting.p2_deferred_age_ms as f64,
    );
    telemetry.record_counter(
        "p2.flush_attempt_count",
        "p2",
        counter_delta(
            accounting.p2_flush_attempt_count,
            &mut cursors.p2_flush_attempt_count,
        ),
    );
    if accounting.write_batch_count > cursors.write_batch_count {
        cursors.write_batch_count = accounting.write_batch_count;
        telemetry.record_duration_ms(
            "sqlite.write_duration_ms",
            "main",
            accounting.write_duration_ms as f64,
        );
    }
    telemetry.record_counter(
        "sqlite.write_rows",
        "main",
        counter_delta(accounting.write_rows, &mut cursors.write_rows),
    );
    telemetry.record_counter(
        "sqlite.write_bytes",
        "main",
        counter_delta(accounting.write_bytes, &mut cursors.write_bytes),
    );
    let coordinator = proxy_sqlite_write_coordinator().snapshot().await;
    telemetry.record_gauge(
        "sqlite.coordinator_waiters",
        "p1_terminal",
        coordinator.p1_waiter_count as f64,
    );
    telemetry.record_gauge(
        "sqlite.coordinator_waiters",
        "interactive_proxy",
        coordinator.interactive_waiter_count as f64,
    );
    telemetry.record_gauge(
        "sqlite.coordinator_waiters",
        "p2_derived",
        coordinator.p2_waiter_count as f64,
    );
    telemetry.record_gauge(
        "sqlite.coordinator_waiters",
        "maintenance_retention",
        coordinator.maintenance_waiter_count as f64,
    );
    record_coordinator_wait_average(
        telemetry,
        "p1_terminal",
        coordinator.write_admission.p1_terminal.admission_count,
        coordinator.write_admission.p1_terminal.total_wait_ms,
        &mut cursors.coordinator_p1_waiter_admissions,
        &mut cursors.coordinator_p1_waiter_total_ms,
    );
    record_coordinator_wait_average(
        telemetry,
        "interactive_proxy",
        coordinator
            .write_admission
            .interactive_proxy
            .admission_count,
        coordinator.write_admission.interactive_proxy.total_wait_ms,
        &mut cursors.coordinator_interactive_admissions,
        &mut cursors.coordinator_interactive_total_ms,
    );
    record_coordinator_wait_average(
        telemetry,
        "p2_derived",
        coordinator.write_admission.p2_derived.admission_count,
        coordinator.write_admission.p2_derived.total_wait_ms,
        &mut cursors.coordinator_p2_admissions,
        &mut cursors.coordinator_p2_total_ms,
    );
    record_coordinator_wait_average(
        telemetry,
        "maintenance_retention",
        coordinator
            .write_admission
            .maintenance_retention
            .admission_count,
        coordinator
            .write_admission
            .maintenance_retention
            .total_wait_ms,
        &mut cursors.coordinator_maintenance_admissions,
        &mut cursors.coordinator_maintenance_total_ms,
    );
    telemetry.record_counter(
        "sqlite.coordinator_bypass_count",
        "coordinator",
        counter_delta(
            coordinator.direct_write_bypass_count,
            &mut cursors.coordinator_bypass_count,
        ),
    );
    telemetry.record_counter(
        "sqlite.maintenance_fairness_count",
        "coordinator",
        counter_delta(
            coordinator.maintenance_fairness_admission_count,
            &mut cursors.coordinator_fairness_count,
        ),
    );
    let pressure = global_db_pressure_gate().snapshot();
    telemetry.record_counter(
        "sqlite.busy_count",
        "main",
        counter_delta(pressure.sqlite_busy_events, &mut cursors.sqlite_busy_count),
    );
    telemetry.record_counter(
        "sqlite.locked_count",
        "main",
        counter_delta(
            pressure.sqlite_locked_events,
            &mut cursors.sqlite_locked_count,
        ),
    );
    telemetry.record_counter(
        "sqlite.pool_timeout_count",
        "main",
        counter_delta(
            pressure.pool_acquire_timeout_events,
            &mut cursors.sqlite_pool_timeout_count,
        ),
    );
    telemetry.record_counter(
        "sqlite.background_skip_count",
        "main",
        counter_delta(
            pressure.background_skips,
            &mut cursors.sqlite_background_skip_count,
        ),
    );
    telemetry.record_gauge(
        "telemetry.queue_depth",
        "collector",
        TELEMETRY_QUEUE_CAPACITY.saturating_sub(telemetry.sender.capacity()) as f64,
    );
    telemetry.record_counter(
        "telemetry.dropped_samples",
        "collector",
        counter_delta(
            telemetry.dropped_samples.load(Ordering::Relaxed),
            &mut cursors.telemetry_dropped_samples,
        ),
    );
    telemetry.record_counter(
        "telemetry.flush_failure_count",
        "collector",
        counter_delta(
            telemetry.flush_failure_count.load(Ordering::Relaxed),
            &mut cursors.telemetry_flush_failure_count,
        ),
    );
    let active_subscribers = state
        .subscription_hub
        .dashboard_activity_live_subscriber_count()
        .await;
    let projection = state
        .proxy_runtime_invocations
        .health_snapshot(active_subscribers);
    telemetry.record_counter(
        "projection.build_count",
        "dashboard",
        counter_delta(projection.build_count, &mut cursors.projection_build_count),
    );
    telemetry.record_counter(
        "projection.live_db_read_count",
        "dashboard",
        counter_delta(
            projection.live_path_db_read_count,
            &mut cursors.projection_live_db_read_count,
        ),
    );
    telemetry.record_counter(
        "projection.cadence_miss_count",
        "current",
        counter_delta(
            projection.slice_counters.current.cadence_miss_count,
            &mut cursors.projection_current_cadence_miss_count,
        ),
    );
    telemetry.record_counter(
        "projection.cadence_miss_count",
        "network",
        counter_delta(
            projection.slice_counters.network.cadence_miss_count,
            &mut cursors.projection_network_cadence_miss_count,
        ),
    );
    telemetry.record_counter(
        "projection.cadence_miss_count",
        "terminal",
        counter_delta(
            projection.slice_counters.terminal.cadence_miss_count,
            &mut cursors.projection_terminal_cadence_miss_count,
        ),
    );
    telemetry.record_counter(
        "projection.revision_count",
        "current",
        counter_delta(
            projection.slice_counters.current.revision_count,
            &mut cursors.projection_current_revision_count,
        ),
    );
    telemetry.record_counter(
        "projection.revision_count",
        "network",
        counter_delta(
            projection.slice_counters.network.revision_count,
            &mut cursors.projection_network_revision_count,
        ),
    );
    telemetry.record_counter(
        "projection.revision_count",
        "terminal",
        counter_delta(
            projection.slice_counters.terminal.revision_count,
            &mut cursors.projection_terminal_revision_count,
        ),
    );
    if let Some(age) = projection.last_good_age_ms {
        telemetry.record_gauge("projection.last_good_age_ms", "dashboard", age as f64);
    }
    telemetry.record_gauge(
        "sse.active_subscribers",
        "dashboard",
        projection.active_subscriber_count as f64,
    );
    if let Some(age_secs) = retention_recovery_health_snapshot().oldest_backlog_age_secs {
        telemetry.record_gauge(
            "maintenance.backlog_age_ms",
            "maintenance",
            age_secs as f64 * 1_000.0,
        );
    }
}

#[cfg(target_os = "linux")]
fn read_process_cpu_ticks() -> Option<(u64, u64)> {
    let process_stat = fs::read_to_string("/proc/self/stat").ok()?;
    let command_end = process_stat.rfind(')')?;
    let mut fields = process_stat.get(command_end + 2..)?.split_whitespace();
    let user_ticks = fields.nth(11)?.parse::<u64>().ok()?;
    let system_ticks = fields.next()?.parse::<u64>().ok()?;
    let total_ticks = fs::read_to_string("/proc/stat")
        .ok()?
        .lines()
        .next()?
        .split_whitespace()
        .skip(1)
        .filter_map(|value| value.parse::<u64>().ok())
        .sum();
    Some((user_ticks.saturating_add(system_ticks), total_ticks))
}

#[cfg(not(target_os = "linux"))]
fn read_process_cpu_ticks() -> Option<(u64, u64)> {
    None
}

async fn sample_process_health(state: &AppState) {
    let telemetry = &state.performance_telemetry;
    let snapshot = state.memory_diagnostics.runtime_pressure_snapshot();
    telemetry.record_gauge(
        "process.rss_bytes",
        "process",
        snapshot.process.rss_bytes as f64,
    );
    telemetry.record_gauge(
        "process.rss_anon_bytes",
        "process",
        snapshot.process.rss_anon_bytes as f64,
    );
    telemetry.record_gauge(
        "process.swap_bytes",
        "process",
        snapshot.process.swap_bytes as f64,
    );
    telemetry.record_gauge(
        "process.managed_bytes",
        "process",
        snapshot.managed_bytes as f64,
    );
    telemetry.record_gauge(
        "process.unattributed_anon_bytes",
        "process",
        snapshot.unattributed_anon_bytes as f64,
    );
    if let Some(thread_count) = read_process_thread_count() {
        telemetry.record_gauge("process.thread_count", "process", thread_count as f64);
    }
    if let Some(available_bytes) = crate::proxy::filesystem_available_bytes(
        state
            .config
            .database_path
            .parent()
            .unwrap_or_else(|| Path::new(".")),
    ) {
        telemetry.record_gauge("process.disk_free_bytes", "process", available_bytes as f64);
    }
    if let Ok(metadata) = std::fs::metadata(&state.config.database_path) {
        telemetry.record_gauge("storage.main_db_bytes", "main_db", metadata.len() as f64);
    }
    if let Ok(metadata) = std::fs::metadata(&state.config.performance_database_path) {
        telemetry.record_gauge(
            "storage.telemetry_db_bytes",
            "telemetry_db",
            metadata.len() as f64,
        );
    }
    let telemetry_wal_path = PathBuf::from(format!(
        "{}-wal",
        state.config.performance_database_path.display()
    ));
    let telemetry_wal_bytes = std::fs::metadata(&telemetry_wal_path)
        .map(|metadata| metadata.len())
        .unwrap_or_default();
    telemetry.record_gauge(
        "storage.telemetry_wal_bytes",
        "telemetry_wal",
        telemetry_wal_bytes as f64,
    );
    let main_wal_path = PathBuf::from(format!("{}-wal", state.config.database_path.display()));
    let main_wal_bytes = std::fs::metadata(main_wal_path)
        .map(|metadata| metadata.len())
        .unwrap_or_default();
    telemetry.record_gauge("sqlite.wal_bytes", "main", main_wal_bytes as f64);
}

#[cfg(target_os = "linux")]
fn read_process_thread_count() -> Option<u64> {
    fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("Threads:")?.trim().parse().ok())
}

#[cfg(not(target_os = "linux"))]
fn read_process_thread_count() -> Option<u64> {
    None
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerformanceQuery {
    pub(crate) range: Option<String>,
    pub(crate) section: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BrowserTelemetryEvent {
    pub(crate) page: String,
    pub(crate) device: String,
    pub(crate) metric: String,
    pub(crate) value: f64,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct BrowserTelemetryRequest {
    pub(crate) events: Vec<BrowserTelemetryEvent>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PerformancePoint {
    bucket_start: i64,
    sample_count: u64,
    expected_count: u64,
    sum: f64,
    min: Option<f64>,
    max: Option<f64>,
    last: Option<f64>,
    #[serde(skip)]
    weighted_sum: f64,
    #[serde(skip)]
    weighted_seconds: f64,
    weighted_average: Option<f64>,
    histogram: Option<Vec<u64>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PerformanceSeries {
    metric_id: String,
    section: String,
    dimension: String,
    kind: String,
    unit: String,
    points: Vec<PerformancePoint>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PerformanceResponse {
    from: String,
    to: String,
    step_seconds: i64,
    coverage: f64,
    epochs: Vec<String>,
    series: Vec<PerformanceSeries>,
}

fn range_config(raw: Option<&str>) -> Option<(i64, i64)> {
    match raw.unwrap_or("24h") {
        "6h" => Some((6 * 60 * 60, 60)),
        "24h" => Some((24 * 60 * 60, 120)),
        "7d" => Some((7 * 24 * 60 * 60, 900)),
        "30d" => Some((30 * 24 * 60 * 60, 3600)),
        "13mo" => Some((13 * 30 * 24 * 60 * 60, 86_400)),
        _ => None,
    }
}

fn valid_section(section: Option<&str>) -> bool {
    section.is_none()
        || matches!(
            section,
            Some("overview" | "storage" | "projection" | "maintenance" | "process" | "browser")
        )
}

pub(crate) async fn fetch_performance_metrics(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PerformanceQuery>,
) -> Response {
    let Some((range_seconds, step_seconds)) = range_config(query.range.as_deref()) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"code":"invalid_range"})),
        )
            .into_response();
    };
    if !valid_section(query.section.as_deref()) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"code":"invalid_section"})),
        )
            .into_response();
    }
    let Some(pool) = state.performance_telemetry.pool.get() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"code":"performance_telemetry_unavailable"})),
        )
            .into_response();
    };
    let to = Utc::now();
    let from = to - ChronoDuration::seconds(range_seconds);
    match query_performance(
        pool,
        from.timestamp(),
        to.timestamp(),
        step_seconds,
        query.section.as_deref(),
    )
    .await
    {
        Ok(response) => Json(response).into_response(),
        Err(error) => {
            warn!(?error, "failed to query performance telemetry");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"code":"performance_telemetry_query_failed"})),
            )
                .into_response()
        }
    }
}

pub(crate) async fn fetch_performance_health(
    State(state): State<Arc<AppState>>,
) -> Json<PerformanceTelemetryHealth> {
    Json(state.performance_telemetry.health_snapshot().await)
}

pub(crate) async fn ingest_browser_performance(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<BrowserTelemetryRequest>,
) -> Response {
    if !is_same_origin_settings_write(&headers)
        || payload.events.len() > TELEMETRY_BROWSER_MAX_EVENTS
    {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"code":"browser_telemetry_rejected"})),
        )
            .into_response();
    }
    let encoded_size = serde_json::to_vec(&payload)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX);
    if encoded_size > TELEMETRY_BROWSER_MAX_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({"code":"browser_telemetry_too_large"})),
        )
            .into_response();
    }
    let client_key = headers
        .get("x-real-ip")
        .or_else(|| headers.get("x-forwarded-for"))
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown");
    if !state
        .performance_telemetry
        .allow_browser_ingest(client_key)
        .await
    {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"code":"browser_telemetry_rate_limited"})),
        )
            .into_response();
    }
    for event in payload.events {
        let Some((metric_id, dimension)) = browser_metric_mapping(&event) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"code":"invalid_browser_metric"})),
            )
                .into_response();
        };
        match metric_id {
            "browser.data_ready_ms"
            | "browser.update_to_paint_ms"
            | "browser.long_task_ms"
            | "browser.api_request_duration_ms"
            | "browser.sse_duration_ms" => {
                state
                    .performance_telemetry
                    .record_duration_ms(metric_id, dimension, event.value)
            }
            "browser.long_task_count"
            | "browser.api_request_count"
            | "browser.sse_disconnect_count"
            | "browser.unsupported_count"
            | "browser.visibility_hidden_count" => state.performance_telemetry.record_counter(
                metric_id,
                dimension,
                event.value.max(0.0) as u64,
            ),
            _ => {}
        }
    }
    StatusCode::NO_CONTENT.into_response()
}

fn browser_metric_mapping(event: &BrowserTelemetryEvent) -> Option<(&'static str, &'static str)> {
    let page = match event.page.as_str() {
        "dashboard" => "dashboard",
        "records" => "records",
        "system" => "system",
        _ => return None,
    };
    let device = match event.device.as_str() {
        "mobile" => "mobile",
        "desktop" => "desktop",
        _ => return None,
    };
    if !event.value.is_finite() || event.value < 0.0 || event.value > 3_600_000.0 {
        return None;
    }
    let metric_id = match event.metric.as_str() {
        "data_ready_ms" => "browser.data_ready_ms",
        "update_to_paint_ms" => "browser.update_to_paint_ms",
        "long_task_ms" => "browser.long_task_ms",
        "long_task_count" => "browser.long_task_count",
        "api_request_duration_ms" => "browser.api_request_duration_ms",
        "api_request_count" => "browser.api_request_count",
        "sse_duration_ms" => "browser.sse_duration_ms",
        "sse_disconnect_count" => "browser.sse_disconnect_count",
        "unsupported_count" => "browser.unsupported_count",
        "visibility_hidden_count" => "browser.visibility_hidden_count",
        _ => return None,
    };
    let dimension = match metric_id {
        "browser.data_ready_ms" | "browser.update_to_paint_ms" => match (page, device) {
            ("dashboard", "mobile") => "dashboard:mobile",
            ("dashboard", "desktop") => "dashboard:desktop",
            ("records", "mobile") => "records:mobile",
            ("records", "desktop") => "records:desktop",
            ("system", "mobile") => "system:mobile",
            ("system", "desktop") => "system:desktop",
            _ => return None,
        },
        _ => page,
    };
    Some((metric_id, dimension))
}

async fn query_performance(
    pool: &Pool<Sqlite>,
    from: i64,
    to: i64,
    step_seconds: i64,
    section: Option<&str>,
) -> Result<PerformanceResponse> {
    let long_term_only = to.saturating_sub(from) > 30 * 24 * 60 * 60;
    let mut grouped: HashMap<(String, String, i64), PerformancePoint> = HashMap::new();
    let mut epochs = BTreeSet::new();
    let mut covered_series_buckets = BTreeSet::new();
    let mut observed_series = BTreeSet::new();
    let total_buckets = ((to - from + step_seconds - 1) / step_seconds).max(1) as usize;
    for (resolution, segment_from, segment_to) in query_resolution_segments(from, to) {
        let rows = sqlx::query("SELECT bucket_start, metric_id, dimension_code, sample_count, expected_count, sum_value, min_value, max_value, last_value, weighted_sum, weighted_seconds, histogram_json, epoch FROM performance_buckets WHERE resolution_seconds = ? AND bucket_start >= ? AND bucket_start < ? ORDER BY bucket_start, metric_id, dimension_code")
            .bind(resolution)
            .bind(segment_from)
            .bind(segment_to)
            .fetch_all(pool)
            .await?;
        for row in rows {
            let metric_id: String = row.try_get("metric_id")?;
            let Some(spec) = metric_spec(&metric_id) else {
                continue;
            };
            if long_term_only && !spec.long_term {
                continue;
            }
            if section.is_some_and(|selected| selected != spec.section) {
                continue;
            }
            let bucket_start: i64 = row.try_get("bucket_start")?;
            let output_bucket = bucket_start / step_seconds * step_seconds;
            let dimension: String = row.try_get("dimension_code")?;
            if !metric_dimension_allowed(&metric_id, &dimension) {
                continue;
            }
            let key = (metric_id.clone(), dimension.clone(), output_bucket);
            let sample_count = row.try_get::<i64, _>("sample_count")?.max(0) as u64;
            let expected_count = row.try_get::<i64, _>("expected_count")?.max(0) as u64;
            let encoded_epoch: String = row.try_get("epoch")?;
            record_epoch_values(&mut epochs, &encoded_epoch);
            let point = grouped.entry(key).or_insert_with(|| PerformancePoint {
                bucket_start: output_bucket,
                sample_count: 0,
                expected_count: 0,
                sum: 0.0,
                min: None,
                max: None,
                last: None,
                weighted_sum: 0.0,
                weighted_seconds: 0.0,
                weighted_average: None,
                histogram: matches!(spec.kind, MetricKind::Duration).then(|| vec![0; 8]),
            });
            point.sample_count = point.sample_count.saturating_add(sample_count);
            point.expected_count = point.expected_count.saturating_add(expected_count);
            point.sum += row.try_get::<f64, _>("sum_value")?;
            let min_value = row.try_get::<Option<f64>, _>("min_value")?;
            let max_value = row.try_get::<Option<f64>, _>("max_value")?;
            point.min = match (point.min, min_value) {
                (Some(left), Some(right)) => Some(left.min(right)),
                (None, value) => value,
                (value, None) => value,
            };
            point.max = match (point.max, max_value) {
                (Some(left), Some(right)) => Some(left.max(right)),
                (None, value) => value,
                (value, None) => value,
            };
            point.last = row.try_get::<Option<f64>, _>("last_value")?;
            point.weighted_sum += row.try_get::<f64, _>("weighted_sum")?;
            point.weighted_seconds += row.try_get::<f64, _>("weighted_seconds")?;
            point.weighted_average =
                (point.weighted_seconds > 0.0).then(|| point.weighted_sum / point.weighted_seconds);
            if let Some(histogram) = point.histogram.as_mut() {
                let source_histogram = serde_json::from_str::<Vec<u64>>(
                    row.try_get::<String, _>("histogram_json")?.as_str(),
                )
                .unwrap_or_default();
                for (index, value) in source_histogram.into_iter().enumerate().take(8) {
                    histogram[index] = histogram[index].saturating_add(value);
                }
            }
        }
    }
    for ((metric_id, dimension, bucket_start), point) in &grouped {
        if point.expected_count > 0 || point.sample_count > 0 {
            observed_series.insert((metric_id.clone(), dimension.clone()));
        }
        if point.expected_count > 0 && point.sample_count >= point.expected_count {
            covered_series_buckets.insert((metric_id.clone(), dimension.clone(), *bucket_start));
        }
    }
    let mut series_map: BTreeMap<(String, String), PerformanceSeries> = METRIC_SPECS
        .iter()
        .filter(|spec| {
            (!long_term_only || spec.long_term)
                && section.is_none_or(|selected| selected == spec.section)
        })
        .flat_map(|spec| {
            metric_dimensions(spec.id).iter().map(move |dimension| {
                (
                    (spec.id.to_string(), (*dimension).to_string()),
                    empty_performance_series(spec, dimension),
                )
            })
        })
        .collect();
    for ((metric_id, dimension, _), point) in grouped {
        let spec = metric_spec(&metric_id).expect("metric registry filtered rows");
        let series = series_map
            .entry((metric_id.clone(), dimension.clone()))
            .or_insert_with(|| empty_performance_series(spec, &dimension));
        if series.points.len() < TELEMETRY_MAX_QUERY_POINTS {
            series.points.push(point);
        }
    }
    let first_output_bucket = from / step_seconds * step_seconds;
    let output_bucket_count = total_buckets.min(TELEMETRY_MAX_QUERY_POINTS);
    for series in series_map.values_mut() {
        if series.points.is_empty() {
            continue;
        }
        let mut points_by_bucket = series
            .points
            .drain(..)
            .map(|point| (point.bucket_start, point))
            .collect::<HashMap<_, _>>();
        series.points = (0..output_bucket_count)
            .map(|index| {
                let bucket_start = first_output_bucket + index as i64 * step_seconds;
                points_by_bucket
                    .remove(&bucket_start)
                    .unwrap_or(PerformancePoint {
                        bucket_start,
                        sample_count: 0,
                        expected_count: 0,
                        sum: 0.0,
                        min: None,
                        max: None,
                        last: None,
                        weighted_sum: 0.0,
                        weighted_seconds: 0.0,
                        weighted_average: None,
                        histogram: (series.kind == "duration").then(|| vec![0; 8]),
                    })
            })
            .collect();
    }
    Ok(PerformanceResponse {
        from: DateTime::<Utc>::from_timestamp(from, 0)
            .unwrap_or_else(Utc::now)
            .to_rfc3339(),
        to: DateTime::<Utc>::from_timestamp(to, 0)
            .unwrap_or_else(Utc::now)
            .to_rfc3339(),
        step_seconds,
        coverage: (covered_series_buckets.len() as f64
            / (observed_series.len().max(1) * total_buckets.min(TELEMETRY_MAX_QUERY_POINTS))
                as f64)
            .min(1.0),
        epochs: epochs.into_iter().collect(),
        series: series_map.into_values().collect(),
    })
}

fn query_resolution_segments(from: i64, to: i64) -> Vec<(i64, i64, i64)> {
    const SEVEN_DAYS: i64 = 7 * 24 * 60 * 60;
    const THIRTY_DAYS: i64 = 30 * 24 * 60 * 60;
    let range_seconds = to.saturating_sub(from);
    if range_seconds <= SEVEN_DAYS {
        return vec![(60, from, to)];
    }

    let minute_cutoff = (to - SEVEN_DAYS).max(from);
    if range_seconds <= THIRTY_DAYS {
        return [(300, from, minute_cutoff), (60, minute_cutoff, to)]
            .into_iter()
            .filter(|(_, segment_from, segment_to)| segment_from < segment_to)
            .collect();
    }

    let five_minute_cutoff = (to - THIRTY_DAYS).max(from);
    [
        (3600, from, five_minute_cutoff),
        (300, five_minute_cutoff, minute_cutoff),
        (60, minute_cutoff, to),
    ]
    .into_iter()
    .filter(|(_, segment_from, segment_to)| segment_from < segment_to)
    .collect()
}

fn record_epoch_values(epochs: &mut BTreeSet<String>, encoded_epoch: &str) {
    if let Ok(values) = serde_json::from_str::<Vec<String>>(encoded_epoch) {
        epochs.extend(values);
    } else {
        epochs.insert(encoded_epoch.to_string());
    }
}

pub(crate) async fn performance_http_middleware(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    state.performance_telemetry.http_request_started();
    let response = next.run(request).await;
    state.performance_telemetry.http_request_finished();
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_registry_stays_bounded_and_rejects_dynamic_dimensions() {
        let recent_count = METRIC_SPECS.iter().filter(|spec| !spec.long_term).count();
        let long_term_count = METRIC_SPECS.iter().filter(|spec| spec.long_term).count();
        assert_eq!(recent_count, 55);
        assert_eq!(long_term_count, 22);
        let recent_series_count = METRIC_SPECS
            .iter()
            .filter(|spec| !spec.long_term)
            .map(|spec| metric_dimensions(spec.id).len())
            .sum::<usize>();
        let long_term_series_count = METRIC_SPECS
            .iter()
            .filter(|spec| spec.long_term)
            .map(|spec| metric_dimensions(spec.id).len())
            .sum::<usize>();
        assert_eq!(
            recent_series_count, 104,
            "recent series: {recent_series_count}"
        );
        assert_eq!(
            long_term_series_count, 22,
            "long-term series: {long_term_series_count}"
        );
        assert!(metric_dimension_allowed("http.in_flight", "other"));
        assert!(!metric_dimension_allowed(
            "http.in_flight",
            "account:secret"
        ));
        assert!(!metric_dimension_allowed(
            "browser.data_ready_ms",
            "dashboard:/private"
        ));
        assert!(metric_dimension_allowed(
            "projection.publish_count",
            "terminal"
        ));
        assert!(metric_dimension_allowed("sqlite.write_bytes", "main"));
        assert!(metric_dimension_allowed("p1.queue_depth", "p1"));
        assert!(
            browser_metric_mapping(&BrowserTelemetryEvent {
                page: "dashboard".to_string(),
                device: "desktop".to_string(),
                metric: "data_ready_ms".to_string(),
                value: 12.0,
            })
            .is_some()
        );
        assert!(
            browser_metric_mapping(&BrowserTelemetryEvent {
                page: "system".to_string(),
                device: "mobile".to_string(),
                metric: "sse_duration_ms".to_string(),
                value: 18.0,
            })
            .is_some()
        );
    }

    #[test]
    fn metric_units_preserve_millisecond_gauges() {
        for metric_id in [
            "p2.next_attempt_delay_ms",
            "p2.deferred_age_ms",
            "projection.last_good_age_ms",
            "maintenance.backlog_age_ms",
        ] {
            let spec = metric_spec(metric_id).expect("metric should be registered");
            assert_eq!(empty_performance_series(spec, "test").unit, "milliseconds");
        }
    }

    #[test]
    fn range_contract_is_fixed() {
        assert_eq!(range_config(Some("6h")), Some((21_600, 60)));
        assert_eq!(range_config(Some("24h")), Some((86_400, 120)));
        assert_eq!(range_config(Some("7d")), Some((604_800, 900)));
        assert_eq!(range_config(Some("30d")), Some((2_592_000, 3_600)));
        assert_eq!(range_config(Some("13mo")), Some((33_696_000, 86_400)));
        assert_eq!(range_config(Some("all")), None);
        assert!(valid_section(Some("storage")));
        assert!(!valid_section(Some("proxy")));
        assert!(!valid_section(Some("history")));
    }

    #[test]
    fn query_resolution_segments_cover_recent_fine_buckets() {
        let to = 40 * 24 * 60 * 60;
        let from = 0;
        assert_eq!(
            query_resolution_segments(from, to),
            vec![
                (3600, 0, 10 * 24 * 60 * 60),
                (300, 10 * 24 * 60 * 60, 33 * 24 * 60 * 60),
                (60, 33 * 24 * 60 * 60, 40 * 24 * 60 * 60),
            ]
        );
    }

    #[tokio::test]
    async fn query_merges_five_minute_and_recent_minute_buckets() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory telemetry database");
        ensure_telemetry_schema(&pool)
            .await
            .expect("initialize telemetry schema");
        let to = Utc::now().timestamp();
        let from = to - 30 * 24 * 60 * 60;
        let old_bucket = (to - 20 * 24 * 60 * 60).div_euclid(300) * 300;
        let recent_bucket = (to - 60 * 60).div_euclid(60) * 60;
        for (bucket, resolution, count, sum) in [
            (old_bucket, 300_i64, 2_i64, 2.0_f64),
            (recent_bucket, 60_i64, 1_i64, 1.0_f64),
        ] {
            sqlx::query(
                "INSERT INTO performance_buckets(
                    bucket_start, resolution_seconds, metric_id, dimension_code,
                    sample_count, expected_count, sum_value, min_value, max_value,
                    last_value, weighted_sum, weighted_seconds, histogram_json, epoch
                ) VALUES (?, ?, 'http.in_flight', 'other', ?, ?, ?, 1, 1, 1, 0, 0, ?, 'query-merge')",
            )
            .bind(bucket)
            .bind(resolution)
            .bind(count)
            .bind(count)
            .bind(sum)
            .bind("[0,0,0,0,0,0,0,0]")
            .execute(&pool)
            .await
            .expect("insert query source bucket");
        }
        sqlx::query(
            "INSERT INTO performance_buckets(
                bucket_start, resolution_seconds, metric_id, dimension_code,
                sample_count, expected_count, sum_value, min_value, max_value,
                last_value, weighted_sum, weighted_seconds, histogram_json, epoch
            ) VALUES (?, 60, 'http.in_flight', 'untrusted:dimension', 99, 99, 99, 99, 99, 99, 0, 0, ?, 'query-merge')",
        )
        .bind(recent_bucket)
        .bind("[0,0,0,0,0,0,0,0]")
        .execute(&pool)
        .await
        .expect("insert invalid dimension fixture");

        let response = query_performance(&pool, from, to, 3600, None)
            .await
            .expect("query merged telemetry buckets");
        let in_flight_series = response
            .series
            .iter()
            .find(|series| series.metric_id == "http.in_flight" && series.dimension == "other")
            .expect("seeded in-flight series");
        assert_eq!(
            in_flight_series
                .points
                .iter()
                .map(|point| point.sample_count)
                .sum::<u64>(),
            3
        );
        assert!(response.coverage > 0.0);
        assert!(
            response
                .series
                .iter()
                .all(|series| series.dimension != "untrusted:dimension")
        );

        let overview = query_performance(&pool, from, to, 3600, Some("overview"))
            .await
            .expect("query overview telemetry buckets");
        assert!(
            overview
                .series
                .iter()
                .any(|series| series.metric_id == "http.in_flight")
        );
        assert!(
            overview
                .series
                .iter()
                .all(|series| series.section == "overview")
        );
    }

    #[tokio::test]
    async fn query_coverage_waits_for_all_rows_in_an_output_bucket() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory telemetry database");
        ensure_telemetry_schema(&pool)
            .await
            .expect("initialize telemetry schema");
        let to = Utc::now().timestamp().div_euclid(300) * 300 + 300;
        let from = to - 600;
        for (bucket, sample_count, expected_count) in [(to - 300, 1_i64, 1_i64), (to - 240, 0, 1)] {
            sqlx::query(
                "INSERT INTO performance_buckets(
                    bucket_start, resolution_seconds, metric_id, dimension_code,
                    sample_count, expected_count, sum_value, min_value, max_value,
                    last_value, weighted_sum, weighted_seconds, histogram_json, epoch
                ) VALUES (?, 60, 'http.in_flight', 'other', ?, ?, ?, NULL, NULL, NULL, 0, 0, ?, 'coverage')",
            )
            .bind(bucket)
            .bind(sample_count)
            .bind(expected_count)
            .bind(sample_count as f64)
            .bind("[0,0,0,0,0,0,0,0]")
            .execute(&pool)
            .await
            .expect("insert mixed-coverage source bucket");
        }

        let response = query_performance(&pool, from, to, 300, None)
            .await
            .expect("query mixed-coverage telemetry buckets");
        assert_eq!(response.coverage, 0.0);
    }

    #[test]
    fn browser_rate_limiter_has_global_and_client_bounds() {
        let mut limiter = BrowserRateLimiter::new();
        for _ in 0..TELEMETRY_BROWSER_CLIENT_RATE_LIMIT {
            assert!(limiter.allow("client-a"));
        }
        assert!(!limiter.allow("client-a"));
        assert!(limiter.allow("client-b"));
    }

    #[tokio::test]
    async fn disabled_runtime_does_not_enqueue_samples() {
        let runtime = PerformanceTelemetryRuntime::disabled_for_tests();
        runtime.record_gauge("p1.queue_depth", "p1", 1.0);
        runtime.record_duration_ms("p1.ack_duration_ms", "p1", 12.0);
        let health = runtime.health_snapshot().await;
        assert_eq!(health.queue_depth, 0);
        assert_eq!(health.dropped_samples, 0);
    }

    #[test]
    fn unavailable_collector_keeps_only_two_pending_minute_buckets() {
        let mut accumulators = HashMap::new();
        for minute in 1..=3 {
            assert!(add_event(
                &mut accumulators,
                TelemetryEvent {
                    metric_id: "p1.queue_depth",
                    dimension: "p1",
                    kind: EventKind::Counter,
                    value: 1.0,
                    weight_seconds: 0.0,
                    at: Utc.timestamp_opt(minute * 60, 0).single().unwrap(),
                },
            ));
        }
        let dropped = trim_unflushed_accumulators(&mut accumulators);
        assert_eq!(dropped, 1);
        assert_eq!(accumulators.len(), 2);
        assert!(accumulators.keys().all(|key| key.0 >= 120));
    }

    #[test]
    fn gauge_accumulator_tracks_fixed_sampling_weight() {
        let mut accumulators = HashMap::new();
        for (minute, value) in [(1, 2.0), (1, 4.0)] {
            assert!(add_event(
                &mut accumulators,
                TelemetryEvent {
                    metric_id: "p1.queue_depth",
                    dimension: "p1",
                    kind: EventKind::Gauge,
                    value,
                    weight_seconds: 10.0,
                    at: Utc.timestamp_opt(minute * 60, 0).single().unwrap(),
                },
            ));
        }
        let accumulator = accumulators
            .values()
            .next()
            .expect("gauge accumulator should exist");
        assert_eq!(accumulator.weighted_sum, 60.0);
        assert_eq!(accumulator.weighted_seconds, 20.0);
    }

    #[tokio::test]
    async fn schema_initialization_creates_one_final_version() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory telemetry database");
        ensure_telemetry_schema(&pool)
            .await
            .expect("initialize telemetry schema");
        ensure_telemetry_schema(&pool)
            .await
            .expect("re-enter telemetry schema");
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .expect("read telemetry schema version");
        assert_eq!(version, TELEMETRY_SCHEMA_VERSION);
        let table_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name LIKE 'performance_%'",
        )
        .fetch_one(&pool)
        .await
        .expect("count telemetry tables");
        assert_eq!(table_count, 4);
    }

    #[tokio::test]
    async fn schema_marker_mismatch_is_rejected_without_modifying_file() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory telemetry database");
        sqlx::query("PRAGMA user_version = 42")
            .execute(&pool)
            .await
            .expect("mark an incompatible telemetry schema");
        let error = ensure_telemetry_schema(&pool)
            .await
            .expect_err("incompatible telemetry schema must not be converted");
        assert!(
            error
                .to_string()
                .contains("recreate the disposable telemetry database")
        );
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await
            .expect("read incompatible telemetry marker");
        assert_eq!(version, 42);
    }

    #[tokio::test]
    async fn marked_schema_with_missing_table_is_rejected() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory telemetry database");
        ensure_telemetry_schema(&pool)
            .await
            .expect("initialize telemetry schema");
        sqlx::query("DROP TABLE performance_buckets")
            .execute(&pool)
            .await
            .expect("remove a required telemetry table");
        let error = ensure_telemetry_schema(&pool)
            .await
            .expect_err("marked partial schema must be rejected");
        assert!(
            error
                .to_string()
                .contains("malformed performance telemetry table")
        );
    }

    #[tokio::test]
    async fn unrelated_existing_database_is_rejected_without_conversion() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open existing database fixture");
        sqlx::query("CREATE TABLE application_state (id INTEGER PRIMARY KEY, value TEXT)")
            .execute(&pool)
            .await
            .expect("create unrelated table");

        let error = ensure_telemetry_schema(&pool)
            .await
            .expect_err("unrelated database must remain untouched");
        assert!(error.to_string().contains("unknown database schema"));
        let telemetry_tables: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'performance_%'",
        )
        .fetch_one(&pool)
        .await
        .expect("count untouched telemetry tables");
        assert_eq!(telemetry_tables, 0);
    }

    #[tokio::test]
    async fn marked_schema_without_bucket_primary_key_is_rejected() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open telemetry database fixture");
        ensure_telemetry_schema(&pool)
            .await
            .expect("initialize telemetry schema");
        sqlx::query("DROP TABLE performance_buckets")
            .execute(&pool)
            .await
            .expect("remove valid bucket table");
        sqlx::query(
            "CREATE TABLE performance_buckets (
                bucket_start INTEGER NOT NULL,
                resolution_seconds INTEGER NOT NULL,
                metric_id TEXT NOT NULL,
                dimension_code TEXT NOT NULL,
                sample_count INTEGER NOT NULL,
                expected_count INTEGER NOT NULL,
                sum_value REAL NOT NULL,
                min_value REAL,
                max_value REAL,
                last_value REAL,
                weighted_sum REAL NOT NULL DEFAULT 0,
                weighted_seconds REAL NOT NULL DEFAULT 0,
                histogram_json TEXT NOT NULL,
                epoch TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("create malformed bucket table");
        let error = ensure_telemetry_schema(&pool)
            .await
            .expect_err("bucket primary key must be required");
        assert!(
            error
                .to_string()
                .contains("malformed performance telemetry table")
        );
    }

    #[tokio::test]
    async fn marked_schema_with_extra_object_is_rejected() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open telemetry database fixture");
        ensure_telemetry_schema(&pool)
            .await
            .expect("initialize telemetry schema");
        sqlx::query("CREATE TABLE performance_extra (value TEXT)")
            .execute(&pool)
            .await
            .expect("create unexpected telemetry object");
        let error = ensure_telemetry_schema(&pool)
            .await
            .expect_err("unexpected marked-schema object must be rejected");
        assert!(error.to_string().contains("unexpected objects"));
    }

    #[tokio::test]
    async fn marked_schema_without_health_check_is_rejected() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open telemetry database fixture");
        ensure_telemetry_schema(&pool)
            .await
            .expect("initialize telemetry schema");
        sqlx::query("DROP TABLE performance_collector_health")
            .execute(&pool)
            .await
            .expect("remove valid collector health table");
        sqlx::query(
            "CREATE TABLE performance_collector_health (
                id INTEGER PRIMARY KEY,
                state TEXT NOT NULL,
                last_successful_flush TEXT,
                dropped_samples INTEGER NOT NULL,
                flush_failure_count INTEGER NOT NULL,
                last_error TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("create malformed collector health table");
        let error = ensure_telemetry_schema(&pool)
            .await
            .expect_err("collector health check must be required");
        assert!(error.to_string().contains("health constraint"));
    }

    #[tokio::test]
    async fn epoch_close_records_termination_time() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open telemetry database fixture");
        ensure_telemetry_schema(&pool)
            .await
            .expect("initialize telemetry schema");
        sqlx::query("INSERT INTO performance_epochs(epoch, started_at) VALUES ('epoch-test', ?)")
            .bind(Utc::now().to_rfc3339())
            .execute(&pool)
            .await
            .expect("insert epoch");

        close_telemetry_epoch(&pool, "epoch-test")
            .await
            .expect("close telemetry epoch");
        let ended_at: Option<String> = sqlx::query_scalar(
            "SELECT ended_at FROM performance_epochs WHERE epoch = 'epoch-test'",
        )
        .fetch_one(&pool)
        .await
        .expect("load closed epoch");
        assert!(ended_at.is_some());
    }

    #[tokio::test]
    async fn failed_shutdown_flush_accounts_buffered_events() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open telemetry database fixture");
        ensure_telemetry_schema(&pool)
            .await
            .expect("initialize telemetry schema");
        let runtime = PerformanceTelemetryRuntime::disabled_for_tests();
        runtime
            .pool
            .set(pool.clone())
            .expect("install test telemetry pool");
        let mut accumulators = HashMap::new();
        assert!(add_event(
            &mut accumulators,
            TelemetryEvent {
                metric_id: "p1.queue_depth",
                dimension: "p1",
                kind: EventKind::Gauge,
                value: 3.0,
                weight_seconds: 10.0,
                at: Utc::now(),
            },
        ));
        pool.close().await;
        assert!(
            flush_accumulators(&runtime, &mut accumulators, "epoch-test")
                .await
                .is_err()
        );
        let dropped = drop_accumulators_with_runtime(&runtime, &mut accumulators);
        runtime
            .dropped_samples
            .fetch_add(dropped, Ordering::Relaxed);
        assert_eq!(runtime.dropped_samples.load(Ordering::Relaxed), 1);
        assert_eq!(runtime.take_dropped_expectations().values().sum::<u64>(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn hard_link_alias_to_main_database_is_rejected() {
        let root = std::env::temp_dir().join(format!(
            "cvm-performance-path-test-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).expect("create path test directory");
        let main_path = root.join("main.sqlite");
        let telemetry_path = root.join("telemetry.sqlite");
        std::fs::write(&main_path, b"sqlite").expect("create main database fixture");
        std::fs::hard_link(&main_path, &telemetry_path).expect("create hard-link alias");
        assert!(telemetry_paths_conflict(&main_path, &telemetry_path));
        std::fs::remove_file(&telemetry_path).expect("remove telemetry alias");
        std::fs::remove_file(&main_path).expect("remove main fixture");
        std::fs::remove_dir(&root).expect("remove path test directory");
    }

    #[test]
    fn sqlite_sidecar_paths_are_rejected_as_conflicts() {
        let root = std::env::temp_dir().join(format!(
            "cvm-performance-sidecar-test-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&root).expect("create sidecar test directory");
        let main_path = root.join("main.sqlite");
        assert!(telemetry_paths_conflict(
            &main_path,
            &root.join("main.sqlite-wal")
        ));
        assert!(telemetry_paths_conflict(
            &main_path,
            &root.join("main.sqlite-shm")
        ));
        std::fs::remove_dir(&root).expect("remove sidecar test directory");
    }

    #[tokio::test]
    async fn flush_persists_weighted_stock_values() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory telemetry database");
        ensure_telemetry_schema(&pool)
            .await
            .expect("initialize telemetry schema");
        let runtime = PerformanceTelemetryRuntime::disabled_for_tests();
        runtime
            .pool
            .set(pool.clone())
            .expect("install test telemetry pool");
        let mut accumulators = HashMap::new();
        assert!(add_event(
            &mut accumulators,
            TelemetryEvent {
                metric_id: "p1.queue_depth",
                dimension: "p1",
                kind: EventKind::Gauge,
                value: 3.0,
                weight_seconds: 10.0,
                at: Utc::now(),
            },
        ));
        flush_accumulators(&runtime, &mut accumulators, "test-epoch")
            .await
            .expect("flush weighted stock value");
        let (weighted_sum, weighted_seconds): (f64, f64) = sqlx::query_as(
            "SELECT weighted_sum, weighted_seconds FROM performance_buckets
             WHERE metric_id = 'p1.queue_depth' AND dimension_code = 'p1'",
        )
        .fetch_one(&pool)
        .await
        .expect("load weighted stock value");
        assert_eq!(weighted_sum, 30.0);
        assert_eq!(weighted_seconds, 10.0);
        let persisted_state: String =
            sqlx::query_scalar("SELECT state FROM performance_collector_health WHERE id = 1")
                .fetch_one(&pool)
                .await
                .expect("load persisted collector health");
        assert_eq!(persisted_state, "healthy");
    }

    #[tokio::test]
    async fn flush_merges_epoch_provenance_for_existing_minute_bucket() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory telemetry database");
        ensure_telemetry_schema(&pool)
            .await
            .expect("initialize telemetry schema");
        let runtime = PerformanceTelemetryRuntime::disabled_for_tests();
        runtime
            .pool
            .set(pool.clone())
            .expect("install test telemetry pool");

        for epoch in ["epoch-a", "epoch-b"] {
            let mut accumulators = HashMap::new();
            assert!(add_event(
                &mut accumulators,
                TelemetryEvent {
                    metric_id: "p1.queue_depth",
                    dimension: "p1",
                    kind: EventKind::Gauge,
                    value: 3.0,
                    weight_seconds: 10.0,
                    at: Utc::now(),
                },
            ));
            flush_accumulators(&runtime, &mut accumulators, epoch)
                .await
                .expect("flush epoch sample");
        }

        let encoded_epoch: String = sqlx::query_scalar(
            "SELECT epoch FROM performance_buckets
             WHERE metric_id = 'p1.queue_depth' AND dimension_code = 'p1'",
        )
        .fetch_one(&pool)
        .await
        .expect("load merged epoch provenance");
        let epochs: Vec<String> =
            serde_json::from_str(&encoded_epoch).expect("decode merged epoch provenance");
        assert_eq!(epochs, vec!["epoch-a", "epoch-b"]);
    }

    #[tokio::test]
    async fn rollup_moves_each_source_bucket_once() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory telemetry database");
        ensure_telemetry_schema(&pool)
            .await
            .expect("initialize telemetry schema");

        let now = Utc::now().timestamp();
        let cutoff = now - 7 * 24 * 60 * 60;
        let old_bucket = (cutoff - 24 * 60 * 60) / 300 * 300;
        for (bucket, epoch) in [(old_bucket, "epoch-a"), (old_bucket + 60, "epoch-b")] {
            sqlx::query(
                "INSERT INTO performance_buckets(
                    bucket_start, resolution_seconds, metric_id, dimension_code,
                    sample_count, expected_count, sum_value, min_value, max_value,
                    last_value, histogram_json, epoch
                ) VALUES (?, 60, 'p1.queue_depth', 'p1', 1, 1, 1, 1, 1, 1, ?, ?)",
            )
            .bind(bucket)
            .bind("[0,0,0,0,0,0,0,0]")
            .bind(epoch)
            .execute(&pool)
            .await
            .expect("insert source bucket");
        }

        rollup_resolution(&pool, 60, 300, cutoff)
            .await
            .expect("roll up minute buckets");
        rollup_resolution(&pool, 60, 300, cutoff)
            .await
            .expect("re-run roll up without duplication");

        let source_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM performance_buckets WHERE resolution_seconds = 60 AND bucket_start < ?",
        )
        .bind(cutoff)
        .fetch_one(&pool)
        .await
        .expect("count source buckets");
        assert_eq!(source_count, 0);

        let (rolled_count, rolled_sum, rolled_epoch): (i64, f64, String) = sqlx::query_as(
            "SELECT sample_count, sum_value, epoch FROM performance_buckets
             WHERE resolution_seconds = 300 AND metric_id = 'p1.queue_depth' AND dimension_code = 'p1'",
        )
        .fetch_one(&pool)
        .await
        .expect("load rolled bucket");
        assert_eq!(rolled_count, 2);
        assert_eq!(rolled_sum, 2.0);
        let epochs: Vec<String> =
            serde_json::from_str(&rolled_epoch).expect("decode rolled epochs");
        assert_eq!(epochs, vec!["epoch-a", "epoch-b"]);
    }

    #[tokio::test]
    async fn rollup_keeps_recent_only_metrics_out_of_hourly_retention() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory telemetry database");
        ensure_telemetry_schema(&pool)
            .await
            .expect("initialize telemetry schema");

        let now = Utc::now().timestamp();
        let cutoff = (now - 31 * 24 * 60 * 60).div_euclid(3600) * 3600;
        let source_bucket = cutoff - 3600;
        for metric_id in ["sqlite.write_rows", "p1.queue_depth"] {
            sqlx::query(
                "INSERT INTO performance_buckets(
                    bucket_start, resolution_seconds, metric_id, dimension_code,
                    sample_count, expected_count, sum_value, min_value, max_value,
                    last_value, weighted_sum, weighted_seconds, histogram_json, epoch
                ) VALUES (?, 300, ?, ?, 1, 1, 1, 1, 1, 1, 0, 0, ?, 'epoch-a')",
            )
            .bind(source_bucket)
            .bind(metric_id)
            .bind(if metric_id == "sqlite.write_rows" {
                "main"
            } else {
                "p1"
            })
            .bind("[0,0,0,0,0,0,0,0]")
            .execute(&pool)
            .await
            .expect("insert source bucket");
        }

        rollup_resolution(&pool, 300, 3600, cutoff)
            .await
            .expect("roll up complete five-minute buckets");

        let recent_hourly: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM performance_buckets
             WHERE resolution_seconds = 3600 AND metric_id = 'sqlite.write_rows'",
        )
        .fetch_one(&pool)
        .await
        .expect("count recent-only hourly buckets");
        let core_hourly: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM performance_buckets
             WHERE resolution_seconds = 3600 AND metric_id = 'p1.queue_depth'",
        )
        .fetch_one(&pool)
        .await
        .expect("count core hourly buckets");
        assert_eq!(recent_hourly, 0);
        assert_eq!(core_hourly, 1);
    }

    #[tokio::test]
    async fn rollup_waits_for_complete_target_buckets_at_a_moving_boundary() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory telemetry database");
        ensure_telemetry_schema(&pool)
            .await
            .expect("initialize telemetry schema");

        let now = Utc::now().timestamp();
        let target_start = (now - 8 * 24 * 60 * 60).div_euclid(300) * 300;
        for bucket in [target_start - 300, target_start]
            .into_iter()
            .flat_map(|start| (0..5).map(move |offset| start + offset * 60))
        {
            sqlx::query(
                "INSERT INTO performance_buckets(
                    bucket_start, resolution_seconds, metric_id, dimension_code,
                    sample_count, expected_count, sum_value, min_value, max_value,
                    last_value, histogram_json, epoch
                ) VALUES (?, 60, 'p1.queue_depth', 'p1', 1, 1, 1, 1, 1, 1, ?, 'epoch-a')",
            )
            .bind(bucket)
            .bind("[0,0,0,0,0,0,0,0]")
            .execute(&pool)
            .await
            .expect("insert source bucket");
        }

        rollup_resolution(&pool, 60, 300, target_start + 120)
            .await
            .expect("roll up only the complete bucket before the boundary");
        rollup_resolution(&pool, 60, 300, target_start + 300)
            .await
            .expect("roll up the next bucket after it becomes complete");

        let rolled: Vec<(i64, i64, f64)> = sqlx::query_as(
            "SELECT bucket_start, sample_count, sum_value FROM performance_buckets
             WHERE resolution_seconds = 300 AND metric_id = 'p1.queue_depth' AND dimension_code = 'p1'
             ORDER BY bucket_start",
        )
        .fetch_all(&pool)
        .await
        .expect("load complete rolled buckets");
        assert_eq!(
            rolled,
            vec![(target_start - 300, 5, 5.0), (target_start, 5, 5.0)]
        );
    }

    #[tokio::test]
    async fn unavailable_collector_counts_buffered_drops() {
        let runtime = PerformanceTelemetryRuntime::disabled_for_tests();
        let mut accumulators = HashMap::new();
        assert!(add_event(
            &mut accumulators,
            TelemetryEvent {
                metric_id: "p1.queue_depth",
                dimension: "p1",
                kind: EventKind::Counter,
                value: 1.0,
                weight_seconds: 0.0,
                at: Utc::now(),
            },
        ));
        flush_accumulators(&runtime, &mut accumulators, "test-epoch")
            .await
            .expect("drop buffered event when collector is unavailable");
        assert!(accumulators.is_empty());
        assert_eq!(runtime.dropped_samples.load(Ordering::Relaxed), 1);
    }
}
