use super::*;

pub(crate) const DASHBOARD_NETWORK_BUCKET_SECONDS: i64 = 300;
pub(crate) const DASHBOARD_NETWORK_RECENT_WINDOW_SECONDS: i64 = 300;
const DASHBOARD_NETWORK_SECOND_BUCKET_RETENTION_SECONDS: i64 =
    DASHBOARD_NETWORK_RECENT_WINDOW_SECONDS;
pub(crate) const DASHBOARD_ACTIVITY_REALTIME_WINDOW_SECONDS: i64 = 60;
const DASHBOARD_ACTIVITY_SECOND_BUCKET_RETENTION_SECONDS: i64 = 180;
pub(crate) const DASHBOARD_NETWORK_UNKNOWN_UPSTREAM_HOST: &str = "__unknown__";
pub(crate) const DASHBOARD_NETWORK_REALTIME_SAMPLE_SECONDS: i64 = 1;
const DASHBOARD_NETWORK_LIVE_STREAM_KEEPALIVE_SECONDS: i64 = 15;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct DashboardNetworkRateSnapshot {
    pub(crate) upload_bytes_per_second: f64,
    pub(crate) download_bytes_per_second: f64,
}

impl DashboardNetworkRateSnapshot {
    fn from_totals(totals: DashboardNetworkByteTotals, sample_seconds: i64) -> Self {
        let divisor = sample_seconds.max(1) as f64;
        Self {
            upload_bytes_per_second: totals.upload_bytes.max(0) as f64 / divisor,
            download_bytes_per_second: totals.download_bytes.max(0) as f64 / divisor,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DashboardNetworkByteTotals {
    pub(crate) upload_bytes: i64,
    pub(crate) download_bytes: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DashboardNetworkRealtimeByteSnapshot {
    pub(crate) sample_start_epoch_second: i64,
    pub(crate) sample_end_epoch_second: i64,
    pub(crate) sample_seconds: i64,
    pub(crate) totals: DashboardNetworkByteTotals,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DashboardRecentNetworkWindowPointSnapshot {
    pub(crate) sample_start_epoch_second: i64,
    pub(crate) sample_end_epoch_second: i64,
    pub(crate) totals: DashboardNetworkByteTotals,
    pub(crate) is_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DashboardRecentNetworkWindowSnapshot {
    pub(crate) range_start_epoch_second: i64,
    pub(crate) range_end_epoch_second: i64,
    pub(crate) window_seconds: i64,
    pub(crate) sample_seconds: i64,
    pub(crate) is_warming_up: bool,
    pub(crate) points: Vec<DashboardRecentNetworkWindowPointSnapshot>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct DashboardActivityCurrentSnapshot {
    pub(crate) qualified_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) first_response_byte_total_sample_count: i64,
    pub(crate) first_response_byte_total_sum_ms: f64,
    pub(crate) total_latency_sample_count: i64,
    pub(crate) total_latency_sum_ms: f64,
}

impl DashboardActivityCurrentSnapshot {
    pub(crate) fn add_assign(&mut self, other: Self) {
        self.qualified_tokens = self
            .qualified_tokens
            .saturating_add(other.qualified_tokens.max(0));
        self.total_cost += other.total_cost.max(0.0);
        self.first_response_byte_total_sample_count = self
            .first_response_byte_total_sample_count
            .saturating_add(other.first_response_byte_total_sample_count.max(0));
        self.first_response_byte_total_sum_ms += other.first_response_byte_total_sum_ms.max(0.0);
        self.total_latency_sample_count = self
            .total_latency_sample_count
            .saturating_add(other.total_latency_sample_count.max(0));
        self.total_latency_sum_ms += other.total_latency_sum_ms.max(0.0);
    }

    fn subtract_assign(&mut self, other: Self) {
        self.qualified_tokens = self
            .qualified_tokens
            .saturating_sub(other.qualified_tokens.max(0));
        self.total_cost = (self.total_cost - other.total_cost.max(0.0)).max(0.0);
        self.first_response_byte_total_sample_count = self
            .first_response_byte_total_sample_count
            .saturating_sub(other.first_response_byte_total_sample_count.max(0));
        self.first_response_byte_total_sum_ms = (self.first_response_byte_total_sum_ms
            - other.first_response_byte_total_sum_ms.max(0.0))
        .max(0.0);
        self.total_latency_sample_count = self
            .total_latency_sample_count
            .saturating_sub(other.total_latency_sample_count.max(0));
        self.total_latency_sum_ms =
            (self.total_latency_sum_ms - other.total_latency_sum_ms.max(0.0)).max(0.0);
    }

    fn is_zero(self) -> bool {
        self.qualified_tokens <= 0
            && self.total_cost <= 0.0
            && self.first_response_byte_total_sample_count <= 0
            && self.first_response_byte_total_sum_ms <= 0.0
            && self.total_latency_sample_count <= 0
            && self.total_latency_sum_ms <= 0.0
    }

    pub(crate) fn first_response_byte_total_avg_ms(self) -> Option<f64> {
        (self.first_response_byte_total_sample_count > 0).then_some(
            self.first_response_byte_total_sum_ms
                / self.first_response_byte_total_sample_count as f64,
        )
    }

    pub(crate) fn avg_total_ms(self) -> Option<f64> {
        (self.total_latency_sample_count > 0)
            .then_some(self.total_latency_sum_ms / self.total_latency_sample_count as f64)
    }
}

impl DashboardNetworkByteTotals {
    pub(crate) fn add_assign(&mut self, other: DashboardNetworkByteTotals) {
        self.upload_bytes = self.upload_bytes.saturating_add(other.upload_bytes.max(0));
        self.download_bytes = self
            .download_bytes
            .saturating_add(other.download_bytes.max(0));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DashboardNetworkScopeKey {
    Global,
    Account(i64),
    Unassigned,
}

impl DashboardNetworkScopeKey {
    pub(crate) fn account_scope(upstream_account_id: Option<i64>) -> Self {
        match upstream_account_id {
            Some(id) => Self::Account(id),
            None => Self::Unassigned,
        }
    }

    pub(crate) fn upstream_account_id(self) -> Option<Option<i64>> {
        match self {
            Self::Global => None,
            Self::Account(id) => Some(Some(id)),
            Self::Unassigned => Some(None),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DashboardNetworkOpenBucketReadState {
    pub(crate) bucket_start: DateTime<Utc>,
    pub(crate) bucket_end: DateTime<Utc>,
    pub(crate) needs_seed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DashboardNetworkOpenBucketSnapshot {
    pub(crate) bucket_start: DateTime<Utc>,
    pub(crate) bucket_end: DateTime<Utc>,
    pub(crate) elapsed_seconds: i64,
    pub(crate) totals: DashboardNetworkByteTotals,
}

#[derive(Debug, Clone, Copy)]
struct DashboardNetworkSecondBucket {
    epoch_second: i64,
    totals: DashboardNetworkByteTotals,
}

#[derive(Debug, Clone, Copy)]
struct DashboardActivitySecondBucket {
    epoch_second: i64,
    snapshot: DashboardActivityCurrentSnapshot,
}

#[derive(Debug, Clone, Copy)]
struct DashboardNetworkOpenBucketState {
    bucket_start_epoch_second: i64,
    seed_totals: DashboardNetworkByteTotals,
    delta_totals: DashboardNetworkByteTotals,
    seeded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DashboardNetworkMinuteBucketKey {
    bucket_start_epoch_second: i64,
    source: String,
    upstream_base_url_host: String,
    upstream_account_id: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, FromRow)]
pub(crate) struct DashboardNetworkMinuteRollupRow {
    pub(crate) bucket_start_epoch: i64,
    pub(crate) source: String,
    pub(crate) upstream_base_url_host: String,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upload_bytes: i64,
    pub(crate) download_bytes: i64,
}

#[derive(Debug, Default)]
struct DashboardTrackedInvocationTraffic {
    upstream_account_id: Option<i64>,
    upstream_base_url_host: Option<String>,
    live_first_response_byte_epoch_second: Option<i64>,
    live_first_response_byte_total_ms: Option<f64>,
}

#[derive(Debug, Default)]
struct DashboardNetworkSpeedCacheInner {
    second_buckets: HashMap<DashboardNetworkScopeKey, VecDeque<DashboardNetworkSecondBucket>>,
    host_second_buckets: HashMap<String, VecDeque<DashboardNetworkSecondBucket>>,
    activity_second_buckets:
        HashMap<DashboardNetworkScopeKey, VecDeque<DashboardActivitySecondBucket>>,
    open_buckets: HashMap<DashboardNetworkScopeKey, DashboardNetworkOpenBucketState>,
    host_open_buckets: HashMap<String, DashboardNetworkOpenBucketState>,
    minute_buckets: HashMap<DashboardNetworkMinuteBucketKey, DashboardNetworkByteTotals>,
    tracked_invocations: HashMap<(String, String), DashboardTrackedInvocationTraffic>,
    latest_speed_epoch_second: Option<i64>,
    latest_activity_epoch_second: Option<i64>,
}

#[derive(Debug)]
pub(crate) struct DashboardNetworkSpeedCache {
    process_started_at_utc: DateTime<Utc>,
    inner: std::sync::Mutex<DashboardNetworkSpeedCacheInner>,
}

impl DashboardNetworkSpeedCache {
    pub(crate) fn new(process_started_at_utc: DateTime<Utc>) -> Self {
        Self {
            process_started_at_utc,
            inner: std::sync::Mutex::new(DashboardNetworkSpeedCacheInner::default()),
        }
    }

    pub(crate) fn process_started_at_utc(&self) -> DateTime<Utc> {
        self.process_started_at_utc
    }

    pub(crate) fn record_request_bytes(
        &self,
        invoke_id: &str,
        occurred_at: &str,
        upstream_account_id: Option<i64>,
        upstream_base_url_host: Option<&str>,
        bytes: usize,
        observed_at: DateTime<Utc>,
    ) {
        let Ok(bytes) = i64::try_from(bytes) else {
            return;
        };
        if bytes <= 0 {
            return;
        }
        let mut inner = self
            .inner
            .lock()
            .expect("dashboard network speed cache should not be poisoned");
        let tracked = inner
            .tracked_invocations
            .entry((invoke_id.to_string(), occurred_at.to_string()))
            .or_default();
        tracked.upstream_account_id = upstream_account_id.or(tracked.upstream_account_id);
        let normalized_upstream_base_url_host =
            normalize_dashboard_network_upstream_host(upstream_base_url_host);
        tracked.upstream_base_url_host =
            normalized_upstream_base_url_host.or(tracked.upstream_base_url_host.clone());
        let tracked_upstream_account_id = tracked.upstream_account_id;
        let tracked_upstream_base_url_host = tracked
            .upstream_base_url_host
            .clone()
            .unwrap_or_else(|| DASHBOARD_NETWORK_UNKNOWN_UPSTREAM_HOST.to_string());
        record_scope_delta_locked(
            &mut inner,
            tracked_upstream_account_id,
            observed_at.timestamp(),
            DashboardNetworkByteTotals {
                upload_bytes: bytes,
                download_bytes: 0,
            },
        );
        record_host_delta_locked(
            &mut inner,
            tracked_upstream_base_url_host.as_str(),
            observed_at.timestamp(),
            DashboardNetworkByteTotals {
                upload_bytes: bytes,
                download_bytes: 0,
            },
        );
        record_minute_delta_locked(
            &mut inner,
            SOURCE_PROXY,
            tracked_upstream_account_id,
            tracked_upstream_base_url_host.as_str(),
            observed_at.timestamp(),
            DashboardNetworkByteTotals {
                upload_bytes: bytes,
                download_bytes: 0,
            },
        );
    }

    pub(crate) fn record_response_chunk_bytes(
        &self,
        invoke_id: &str,
        occurred_at: &str,
        upstream_account_id: Option<i64>,
        upstream_base_url_host: Option<&str>,
        bytes: usize,
        observed_at: DateTime<Utc>,
    ) {
        let Ok(bytes) = i64::try_from(bytes) else {
            return;
        };
        if bytes <= 0 {
            return;
        }
        let mut inner = self
            .inner
            .lock()
            .expect("dashboard network speed cache should not be poisoned");
        let tracked = inner
            .tracked_invocations
            .entry((invoke_id.to_string(), occurred_at.to_string()))
            .or_default();
        tracked.upstream_account_id = upstream_account_id.or(tracked.upstream_account_id);
        let normalized_upstream_base_url_host =
            normalize_dashboard_network_upstream_host(upstream_base_url_host);
        tracked.upstream_base_url_host =
            normalized_upstream_base_url_host.or(tracked.upstream_base_url_host.clone());
        let tracked_upstream_account_id = tracked.upstream_account_id;
        let tracked_upstream_base_url_host = tracked
            .upstream_base_url_host
            .clone()
            .unwrap_or_else(|| DASHBOARD_NETWORK_UNKNOWN_UPSTREAM_HOST.to_string());
        record_scope_delta_locked(
            &mut inner,
            tracked_upstream_account_id,
            observed_at.timestamp(),
            DashboardNetworkByteTotals {
                upload_bytes: 0,
                download_bytes: bytes,
            },
        );
        record_host_delta_locked(
            &mut inner,
            tracked_upstream_base_url_host.as_str(),
            observed_at.timestamp(),
            DashboardNetworkByteTotals {
                upload_bytes: 0,
                download_bytes: bytes,
            },
        );
        record_minute_delta_locked(
            &mut inner,
            SOURCE_PROXY,
            tracked_upstream_account_id,
            tracked_upstream_base_url_host.as_str(),
            observed_at.timestamp(),
            DashboardNetworkByteTotals {
                upload_bytes: 0,
                download_bytes: bytes,
            },
        );
    }

    pub(crate) fn finish_invocation(&self, invoke_id: &str, occurred_at: &str) {
        let mut inner = self
            .inner
            .lock()
            .expect("dashboard network speed cache should not be poisoned");
        inner
            .tracked_invocations
            .remove(&(invoke_id.to_string(), occurred_at.to_string()));
    }

    pub(crate) fn observe_dashboard_activity_runtime_snapshot(
        &self,
        record: &ApiInvocation,
        observed_at: DateTime<Utc>,
    ) {
        let live_first_response_byte_total_ms = crate::stats::resolve_first_response_byte_total_ms(
            record.t_req_read_ms,
            record.t_req_parse_ms,
            record.t_upstream_connect_ms,
            record.t_upstream_ttfb_ms,
        );
        let observed_epoch_second = observed_at.timestamp();
        let mut inner = self
            .inner
            .lock()
            .expect("dashboard network speed cache should not be poisoned");
        prune_activity_second_buckets_locked(&mut inner, observed_epoch_second);
        let key = (record.invoke_id.clone(), record.occurred_at.clone());
        let mut tracked = inner.tracked_invocations.remove(&key).unwrap_or_default();
        let previous_upstream_account_id = tracked.upstream_account_id;
        tracked.upstream_account_id = record.upstream_account_id.or(tracked.upstream_account_id);

        if previous_upstream_account_id != tracked.upstream_account_id
            && let (Some(epoch_second), Some(first_response_byte_total_ms)) = (
                tracked.live_first_response_byte_epoch_second,
                tracked.live_first_response_byte_total_ms,
            )
        {
            remove_activity_sample_for_scope_locked(
                &mut inner,
                previous_upstream_account_id,
                epoch_second,
                first_response_byte_total_ms,
            );
            add_activity_sample_for_scope_locked(
                &mut inner,
                tracked.upstream_account_id,
                epoch_second,
                first_response_byte_total_ms,
            );
        }

        match (
            tracked.live_first_response_byte_epoch_second,
            tracked.live_first_response_byte_total_ms,
            live_first_response_byte_total_ms,
        ) {
            (None, _, Some(first_response_byte_total_ms)) => {
                tracked.live_first_response_byte_epoch_second = Some(observed_epoch_second);
                tracked.live_first_response_byte_total_ms = Some(first_response_byte_total_ms);
                add_activity_sample_for_scope_locked(
                    &mut inner,
                    tracked.upstream_account_id,
                    observed_epoch_second,
                    first_response_byte_total_ms,
                );
            }
            (Some(epoch_second), Some(previous_ms), Some(next_ms))
                if (previous_ms - next_ms).abs() >= 0.5 =>
            {
                remove_activity_sample_for_scope_locked(
                    &mut inner,
                    tracked.upstream_account_id,
                    epoch_second,
                    previous_ms,
                );
                tracked.live_first_response_byte_total_ms = Some(next_ms);
                add_activity_sample_for_scope_locked(
                    &mut inner,
                    tracked.upstream_account_id,
                    epoch_second,
                    next_ms,
                );
            }
            _ => {}
        }
        inner.tracked_invocations.insert(key, tracked);
    }

    pub(crate) fn finalize_dashboard_activity_invocation(
        &self,
        record: &ApiInvocation,
        observed_at: DateTime<Utc>,
    ) {
        let observed_epoch_second = observed_at.timestamp();
        let success_like =
            prompt_cache_and_timeseries_shared::prompt_invocation_status_is_success_like(
                record.status.as_deref(),
                record.error_message.as_deref(),
            );
        let classification = resolve_failure_classification(
            record.status.as_deref(),
            record.error_message.as_deref(),
            record.failure_kind.as_deref(),
            record.failure_class.as_deref(),
            record.is_actionable.map(i64::from),
        );
        let is_success = success_like && classification.failure_class == FailureClass::None;
        let terminal_first_response_byte_total_ms =
            crate::stats::resolve_first_response_byte_total_ms(
                record.t_req_read_ms,
                record.t_req_parse_ms,
                record.t_upstream_connect_ms,
                record.t_upstream_ttfb_ms,
            );
        let terminal_total_ms = record
            .t_total_ms
            .filter(|value| value.is_finite() && *value >= 0.0);
        let terminal_qualified_tokens = if is_success && record.cost.is_some() {
            record.total_tokens.unwrap_or_default().max(0)
        } else {
            0
        };
        let terminal_cost = record.cost.unwrap_or_default().max(0.0);

        let mut inner = self
            .inner
            .lock()
            .expect("dashboard network speed cache should not be poisoned");
        prune_activity_second_buckets_locked(&mut inner, observed_epoch_second);
        let tracked = inner
            .tracked_invocations
            .remove(&(record.invoke_id.clone(), record.occurred_at.clone()))
            .unwrap_or_default();
        let upstream_account_id = record.upstream_account_id.or(tracked.upstream_account_id);

        if let (Some(epoch_second), Some(first_response_byte_total_ms)) = (
            tracked.live_first_response_byte_epoch_second,
            tracked.live_first_response_byte_total_ms,
        ) && !is_success
        {
            remove_activity_sample_for_scope_locked(
                &mut inner,
                upstream_account_id,
                epoch_second,
                first_response_byte_total_ms,
            );
        }

        if is_success {
            if tracked.live_first_response_byte_total_ms.is_none()
                && let Some(first_response_byte_total_ms) = terminal_first_response_byte_total_ms
            {
                add_activity_sample_for_scope_locked(
                    &mut inner,
                    upstream_account_id,
                    observed_epoch_second,
                    first_response_byte_total_ms,
                );
            }
            let mut terminal_snapshot = DashboardActivityCurrentSnapshot {
                qualified_tokens: terminal_qualified_tokens,
                total_cost: terminal_cost,
                ..DashboardActivityCurrentSnapshot::default()
            };
            if let Some(total_ms) = terminal_total_ms {
                terminal_snapshot.total_latency_sample_count = 1;
                terminal_snapshot.total_latency_sum_ms = total_ms;
            }
            record_activity_scope_delta_locked(
                &mut inner,
                upstream_account_id,
                observed_epoch_second,
                terminal_snapshot,
            );
        } else if terminal_cost > 0.0 {
            record_activity_scope_delta_locked(
                &mut inner,
                upstream_account_id,
                observed_epoch_second,
                DashboardActivityCurrentSnapshot {
                    total_cost: terminal_cost,
                    ..DashboardActivityCurrentSnapshot::default()
                },
            );
        }
    }

    pub(crate) fn drop_dashboard_activity_invocation(&self, invoke_id: &str, occurred_at: &str) {
        let mut inner = self
            .inner
            .lock()
            .expect("dashboard network speed cache should not be poisoned");
        let now_epoch_second = Utc::now().timestamp();
        prune_activity_second_buckets_locked(&mut inner, now_epoch_second);
        let Some(tracked) = inner
            .tracked_invocations
            .remove(&(invoke_id.to_string(), occurred_at.to_string()))
        else {
            return;
        };
        if let (Some(epoch_second), Some(first_response_byte_total_ms)) = (
            tracked.live_first_response_byte_epoch_second,
            tracked.live_first_response_byte_total_ms,
        ) {
            remove_activity_sample_for_scope_locked(
                &mut inner,
                tracked.upstream_account_id,
                epoch_second,
                first_response_byte_total_ms,
            );
        }
    }

    pub(crate) fn snapshot_dashboard_activity_accounts(
        &self,
        now: DateTime<Utc>,
    ) -> HashMap<Option<i64>, DashboardActivityCurrentSnapshot> {
        let now_epoch_second = now.timestamp();
        let mut inner = self
            .inner
            .lock()
            .expect("dashboard network speed cache should not be poisoned");
        prune_activity_second_buckets_locked(&mut inner, now_epoch_second);

        let mut snapshots = HashMap::new();
        for (scope, buckets) in &inner.activity_second_buckets {
            let Some(upstream_account_id) = scope.upstream_account_id() else {
                continue;
            };
            let mut snapshot = DashboardActivityCurrentSnapshot::default();
            for bucket in buckets {
                if bucket.epoch_second
                    < now_epoch_second.saturating_sub(
                        DASHBOARD_ACTIVITY_REALTIME_WINDOW_SECONDS.saturating_sub(1),
                    )
                {
                    continue;
                }
                if bucket.epoch_second > now_epoch_second {
                    continue;
                }
                snapshot.add_assign(bucket.snapshot);
            }
            snapshots.insert(upstream_account_id, snapshot);
        }
        snapshots
    }

    pub(crate) fn snapshot_account_rates(
        &self,
        now: DateTime<Utc>,
    ) -> HashMap<Option<i64>, DashboardNetworkRateSnapshot> {
        let now_epoch_second = now.timestamp();
        let mut inner = self
            .inner
            .lock()
            .expect("dashboard network speed cache should not be poisoned");
        prune_second_buckets_locked(&mut inner, now_epoch_second);

        let mut rates = HashMap::new();
        for (scope, buckets) in &inner.second_buckets {
            let Some(upstream_account_id) = scope.upstream_account_id() else {
                continue;
            };
            let snapshot = snapshot_complete_second_rate_from_buckets(buckets, now_epoch_second);
            rates.insert(
                upstream_account_id,
                DashboardNetworkRateSnapshot::from_totals(snapshot.totals, snapshot.sample_seconds),
            );
        }
        rates
    }

    pub(crate) fn snapshot_scope_realtime_bytes(
        &self,
        scope: DashboardNetworkScopeKey,
        now: DateTime<Utc>,
    ) -> DashboardNetworkRealtimeByteSnapshot {
        let now_epoch_second = now.timestamp();
        let mut inner = self
            .inner
            .lock()
            .expect("dashboard network speed cache should not be poisoned");
        prune_second_buckets_locked(&mut inner, now_epoch_second);
        let buckets = inner.second_buckets.get(&scope);
        snapshot_complete_second_rate_from_optional_buckets(buckets, now_epoch_second)
    }

    pub(crate) fn snapshot_recent_global_window(
        &self,
        now: DateTime<Utc>,
    ) -> DashboardRecentNetworkWindowSnapshot {
        let now_epoch_second = now.timestamp();
        let mut inner = self
            .inner
            .lock()
            .expect("dashboard network speed cache should not be poisoned");
        prune_second_buckets_locked(&mut inner, now_epoch_second);

        let range_end_epoch_second = now_epoch_second;
        let range_start_epoch_second =
            range_end_epoch_second.saturating_sub(DASHBOARD_NETWORK_RECENT_WINDOW_SECONDS);
        let first_available_epoch_second = self.process_started_at_utc.timestamp();
        let recent_buckets = inner
            .second_buckets
            .get(&DashboardNetworkScopeKey::Global)
            .map(|buckets| {
                buckets
                    .iter()
                    .map(|bucket| (bucket.epoch_second, bucket.totals))
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        let mut points = Vec::with_capacity(DASHBOARD_NETWORK_RECENT_WINDOW_SECONDS as usize);
        for sample_start_epoch_second in range_start_epoch_second..range_end_epoch_second {
            let is_available = sample_start_epoch_second >= first_available_epoch_second;
            points.push(DashboardRecentNetworkWindowPointSnapshot {
                sample_start_epoch_second,
                sample_end_epoch_second: sample_start_epoch_second
                    .saturating_add(DASHBOARD_NETWORK_REALTIME_SAMPLE_SECONDS),
                totals: if is_available {
                    recent_buckets
                        .get(&sample_start_epoch_second)
                        .copied()
                        .unwrap_or_default()
                } else {
                    DashboardNetworkByteTotals::default()
                },
                is_available,
            });
        }

        DashboardRecentNetworkWindowSnapshot {
            range_start_epoch_second,
            range_end_epoch_second,
            window_seconds: DASHBOARD_NETWORK_RECENT_WINDOW_SECONDS,
            sample_seconds: DASHBOARD_NETWORK_REALTIME_SAMPLE_SECONDS,
            is_warming_up: range_start_epoch_second < first_available_epoch_second,
            points,
        }
    }

    pub(crate) fn should_keep_dashboard_activity_live_stream(&self, now: DateTime<Utc>) -> bool {
        let now_epoch_second = now.timestamp();
        let inner = self
            .inner
            .lock()
            .expect("dashboard network speed cache should not be poisoned");
        if inner.latest_speed_epoch_second.is_some_and(|latest| {
            now_epoch_second.saturating_sub(latest)
                < DASHBOARD_NETWORK_LIVE_STREAM_KEEPALIVE_SECONDS
        }) {
            return true;
        }
        if inner.latest_activity_epoch_second.is_some_and(|latest| {
            now_epoch_second.saturating_sub(latest) < DASHBOARD_ACTIVITY_REALTIME_WINDOW_SECONDS
        }) {
            return true;
        }

        let current_bucket_start_epoch_second = current_bucket_start_epoch_second(now_epoch_second);
        inner
            .open_buckets
            .get(&DashboardNetworkScopeKey::Global)
            .filter(|bucket| bucket.bucket_start_epoch_second == current_bucket_start_epoch_second)
            .is_some_and(|bucket| {
                bucket.seed_totals.upload_bytes > 0
                    || bucket.seed_totals.download_bytes > 0
                    || bucket.delta_totals.upload_bytes > 0
                    || bucket.delta_totals.download_bytes > 0
            })
    }

    pub(crate) fn open_bucket_read_state(
        &self,
        scope: DashboardNetworkScopeKey,
        now: DateTime<Utc>,
    ) -> DashboardNetworkOpenBucketReadState {
        let now_epoch_second = now.timestamp();
        let bucket_start_epoch_second = current_bucket_start_epoch_second(now_epoch_second);
        let mut inner = self
            .inner
            .lock()
            .expect("dashboard network speed cache should not be poisoned");
        ensure_open_bucket_locked(&mut inner, scope, bucket_start_epoch_second);
        let bucket = inner
            .open_buckets
            .get(&scope)
            .expect("open bucket should exist after ensure");
        DashboardNetworkOpenBucketReadState {
            bucket_start: epoch_second_to_utc(bucket.bucket_start_epoch_second),
            bucket_end: epoch_second_to_utc(
                bucket
                    .bucket_start_epoch_second
                    .saturating_add(DASHBOARD_NETWORK_BUCKET_SECONDS),
            ),
            needs_seed: !bucket.seeded,
        }
    }

    pub(crate) fn seed_open_bucket(
        &self,
        scope: DashboardNetworkScopeKey,
        bucket_start: DateTime<Utc>,
        seed_totals: DashboardNetworkByteTotals,
        now: DateTime<Utc>,
    ) -> DashboardNetworkOpenBucketSnapshot {
        let mut inner = self
            .inner
            .lock()
            .expect("dashboard network speed cache should not be poisoned");
        let now_epoch_second = now.timestamp();
        let bucket_start_epoch_second = current_bucket_start_epoch_second(now_epoch_second);
        ensure_open_bucket_locked(&mut inner, scope, bucket_start_epoch_second);

        if let Some(bucket) = inner.open_buckets.get_mut(&scope)
            && bucket.bucket_start_epoch_second == bucket_start.timestamp()
        {
            bucket.seed_totals = seed_totals;
            bucket.seeded = true;
        }

        snapshot_open_bucket_locked(&inner, scope, now_epoch_second)
    }

    pub(crate) fn snapshot_open_bucket(
        &self,
        scope: DashboardNetworkScopeKey,
        now: DateTime<Utc>,
    ) -> DashboardNetworkOpenBucketSnapshot {
        let now_epoch_second = now.timestamp();
        let bucket_start_epoch_second = current_bucket_start_epoch_second(now_epoch_second);
        let mut inner = self
            .inner
            .lock()
            .expect("dashboard network speed cache should not be poisoned");
        ensure_open_bucket_locked(&mut inner, scope, bucket_start_epoch_second);
        snapshot_open_bucket_locked(&inner, scope, now_epoch_second)
    }

    pub(crate) fn drain_completed_socket_minute_rows(
        &self,
        now: DateTime<Utc>,
    ) -> Vec<DashboardNetworkMinuteRollupRow> {
        let current_minute_start_epoch_second = current_minute_start_epoch_second(now.timestamp());
        let mut inner = self
            .inner
            .lock()
            .expect("dashboard network speed cache should not be poisoned");
        let mut drained = Vec::new();
        inner.minute_buckets.retain(|key, totals| {
            if key.bucket_start_epoch_second < current_minute_start_epoch_second {
                drained.push(DashboardNetworkMinuteRollupRow {
                    bucket_start_epoch: key.bucket_start_epoch_second,
                    source: key.source.clone(),
                    upstream_base_url_host: key.upstream_base_url_host.clone(),
                    upstream_account_id: key.upstream_account_id,
                    upload_bytes: totals.upload_bytes.max(0),
                    download_bytes: totals.download_bytes.max(0),
                });
                false
            } else {
                true
            }
        });
        drained.sort_by(|left, right| {
            (
                left.bucket_start_epoch,
                left.source.as_str(),
                left.upstream_base_url_host.as_str(),
                left.upstream_account_id,
            )
                .cmp(&(
                    right.bucket_start_epoch,
                    right.source.as_str(),
                    right.upstream_base_url_host.as_str(),
                    right.upstream_account_id,
                ))
        });
        drained
    }

    pub(crate) fn restore_completed_socket_minute_rows(
        &self,
        rows: Vec<DashboardNetworkMinuteRollupRow>,
    ) {
        if rows.is_empty() {
            return;
        }
        let mut inner = self
            .inner
            .lock()
            .expect("dashboard network speed cache should not be poisoned");
        for row in rows {
            let key = DashboardNetworkMinuteBucketKey {
                bucket_start_epoch_second: row.bucket_start_epoch,
                source: row.source,
                upstream_base_url_host: row.upstream_base_url_host,
                upstream_account_id: row.upstream_account_id,
            };
            let entry = inner.minute_buckets.entry(key).or_default();
            entry.add_assign(DashboardNetworkByteTotals {
                upload_bytes: row.upload_bytes,
                download_bytes: row.download_bytes,
            });
        }
    }
}

fn record_scope_delta_locked(
    inner: &mut DashboardNetworkSpeedCacheInner,
    upstream_account_id: Option<i64>,
    observed_epoch_second: i64,
    totals: DashboardNetworkByteTotals,
) {
    if totals.upload_bytes <= 0 && totals.download_bytes <= 0 {
        return;
    }
    inner.latest_speed_epoch_second = Some(observed_epoch_second);
    for scope in [
        DashboardNetworkScopeKey::Global,
        DashboardNetworkScopeKey::account_scope(upstream_account_id),
    ] {
        record_bucket_delta_locked(inner, scope, observed_epoch_second, totals);
    }
}

fn record_host_delta_locked(
    inner: &mut DashboardNetworkSpeedCacheInner,
    upstream_base_url_host: &str,
    observed_epoch_second: i64,
    totals: DashboardNetworkByteTotals,
) {
    if totals.upload_bytes <= 0 && totals.download_bytes <= 0 {
        return;
    }
    let normalized_upstream_base_url_host =
        normalize_dashboard_network_upstream_host(Some(upstream_base_url_host))
            .unwrap_or_else(|| DASHBOARD_NETWORK_UNKNOWN_UPSTREAM_HOST.to_string());
    let buckets = inner
        .host_second_buckets
        .entry(normalized_upstream_base_url_host.clone())
        .or_default();
    if let Some(last) = buckets.back_mut()
        && last.epoch_second == observed_epoch_second
    {
        last.totals.add_assign(totals);
    } else {
        buckets.push_back(DashboardNetworkSecondBucket {
            epoch_second: observed_epoch_second,
            totals,
        });
    }

    let bucket_start_epoch_second = current_bucket_start_epoch_second(observed_epoch_second);
    let open_bucket = inner
        .host_open_buckets
        .entry(normalized_upstream_base_url_host)
        .or_insert(DashboardNetworkOpenBucketState {
            bucket_start_epoch_second,
            seed_totals: DashboardNetworkByteTotals::default(),
            delta_totals: DashboardNetworkByteTotals::default(),
            seeded: false,
        });
    if open_bucket.bucket_start_epoch_second != bucket_start_epoch_second {
        *open_bucket = DashboardNetworkOpenBucketState {
            bucket_start_epoch_second,
            seed_totals: DashboardNetworkByteTotals::default(),
            delta_totals: DashboardNetworkByteTotals::default(),
            seeded: false,
        };
    }
    open_bucket.delta_totals.add_assign(totals);
    prune_second_buckets_locked(inner, observed_epoch_second);
}

fn record_minute_delta_locked(
    inner: &mut DashboardNetworkSpeedCacheInner,
    source: &str,
    upstream_account_id: Option<i64>,
    upstream_base_url_host: &str,
    observed_epoch_second: i64,
    totals: DashboardNetworkByteTotals,
) {
    if totals.upload_bytes <= 0 && totals.download_bytes <= 0 {
        return;
    }
    let bucket_start_epoch_second = current_minute_start_epoch_second(observed_epoch_second);
    let normalized_upstream_base_url_host =
        normalize_dashboard_network_upstream_host(Some(upstream_base_url_host))
            .unwrap_or_else(|| DASHBOARD_NETWORK_UNKNOWN_UPSTREAM_HOST.to_string());
    let key = DashboardNetworkMinuteBucketKey {
        bucket_start_epoch_second,
        source: source.to_string(),
        upstream_base_url_host: normalized_upstream_base_url_host,
        upstream_account_id,
    };
    let entry = inner.minute_buckets.entry(key).or_default();
    entry.add_assign(totals);
}

fn record_bucket_delta_locked(
    inner: &mut DashboardNetworkSpeedCacheInner,
    scope: DashboardNetworkScopeKey,
    observed_epoch_second: i64,
    totals: DashboardNetworkByteTotals,
) {
    let buckets = inner.second_buckets.entry(scope).or_default();
    if let Some(last) = buckets.back_mut()
        && last.epoch_second == observed_epoch_second
    {
        last.totals.add_assign(totals);
    } else {
        buckets.push_back(DashboardNetworkSecondBucket {
            epoch_second: observed_epoch_second,
            totals,
        });
    }

    let bucket_start_epoch_second = current_bucket_start_epoch_second(observed_epoch_second);
    let open_bucket = inner
        .open_buckets
        .entry(scope)
        .or_insert(DashboardNetworkOpenBucketState {
            bucket_start_epoch_second,
            seed_totals: DashboardNetworkByteTotals::default(),
            delta_totals: DashboardNetworkByteTotals::default(),
            seeded: false,
        });
    if open_bucket.bucket_start_epoch_second != bucket_start_epoch_second {
        *open_bucket = DashboardNetworkOpenBucketState {
            bucket_start_epoch_second,
            seed_totals: DashboardNetworkByteTotals::default(),
            delta_totals: DashboardNetworkByteTotals::default(),
            seeded: false,
        };
    }
    open_bucket.delta_totals.add_assign(totals);
    prune_second_buckets_locked(inner, observed_epoch_second);
}

fn add_activity_sample_for_scope_locked(
    inner: &mut DashboardNetworkSpeedCacheInner,
    upstream_account_id: Option<i64>,
    observed_epoch_second: i64,
    first_response_byte_total_ms: f64,
) {
    if !first_response_byte_total_ms.is_finite() || first_response_byte_total_ms < 0.0 {
        return;
    }
    record_activity_scope_delta_locked(
        inner,
        upstream_account_id,
        observed_epoch_second,
        DashboardActivityCurrentSnapshot {
            first_response_byte_total_sample_count: 1,
            first_response_byte_total_sum_ms: first_response_byte_total_ms,
            ..DashboardActivityCurrentSnapshot::default()
        },
    );
}

fn remove_activity_sample_for_scope_locked(
    inner: &mut DashboardNetworkSpeedCacheInner,
    upstream_account_id: Option<i64>,
    observed_epoch_second: i64,
    first_response_byte_total_ms: f64,
) {
    if !first_response_byte_total_ms.is_finite() || first_response_byte_total_ms < 0.0 {
        return;
    }
    remove_activity_scope_delta_locked(
        inner,
        upstream_account_id,
        observed_epoch_second,
        DashboardActivityCurrentSnapshot {
            first_response_byte_total_sample_count: 1,
            first_response_byte_total_sum_ms: first_response_byte_total_ms,
            ..DashboardActivityCurrentSnapshot::default()
        },
    );
}

fn record_activity_scope_delta_locked(
    inner: &mut DashboardNetworkSpeedCacheInner,
    upstream_account_id: Option<i64>,
    observed_epoch_second: i64,
    snapshot: DashboardActivityCurrentSnapshot,
) {
    if snapshot.is_zero() {
        return;
    }
    inner.latest_activity_epoch_second = Some(
        inner
            .latest_activity_epoch_second
            .map_or(observed_epoch_second, |current| {
                current.max(observed_epoch_second)
            }),
    );
    for scope in [
        DashboardNetworkScopeKey::Global,
        DashboardNetworkScopeKey::account_scope(upstream_account_id),
    ] {
        record_activity_bucket_delta_locked(inner, scope, observed_epoch_second, snapshot);
    }
}

fn snapshot_complete_second_rate_from_buckets(
    buckets: &VecDeque<DashboardNetworkSecondBucket>,
    now_epoch_second: i64,
) -> DashboardNetworkRealtimeByteSnapshot {
    snapshot_complete_second_rate_from_optional_buckets(Some(buckets), now_epoch_second)
}

fn snapshot_complete_second_rate_from_optional_buckets(
    buckets: Option<&VecDeque<DashboardNetworkSecondBucket>>,
    now_epoch_second: i64,
) -> DashboardNetworkRealtimeByteSnapshot {
    let sample_end_epoch_second = now_epoch_second;
    let sample_start_epoch_second =
        sample_end_epoch_second.saturating_sub(DASHBOARD_NETWORK_REALTIME_SAMPLE_SECONDS);
    let target_epoch_second = sample_start_epoch_second;
    let totals = buckets
        .and_then(|rows| {
            rows.iter()
                .find(|bucket| bucket.epoch_second == target_epoch_second)
                .map(|bucket| bucket.totals)
        })
        .unwrap_or_default();
    DashboardNetworkRealtimeByteSnapshot {
        sample_start_epoch_second,
        sample_end_epoch_second,
        sample_seconds: DASHBOARD_NETWORK_REALTIME_SAMPLE_SECONDS,
        totals,
    }
}

fn record_activity_bucket_delta_locked(
    inner: &mut DashboardNetworkSpeedCacheInner,
    scope: DashboardNetworkScopeKey,
    observed_epoch_second: i64,
    snapshot: DashboardActivityCurrentSnapshot,
) {
    let buckets = inner.activity_second_buckets.entry(scope).or_default();
    if let Some(last) = buckets.back_mut()
        && last.epoch_second == observed_epoch_second
    {
        last.snapshot.add_assign(snapshot);
    } else {
        buckets.push_back(DashboardActivitySecondBucket {
            epoch_second: observed_epoch_second,
            snapshot,
        });
    }
    prune_activity_second_buckets_locked(inner, observed_epoch_second);
}

fn remove_activity_scope_delta_locked(
    inner: &mut DashboardNetworkSpeedCacheInner,
    upstream_account_id: Option<i64>,
    observed_epoch_second: i64,
    snapshot: DashboardActivityCurrentSnapshot,
) {
    if snapshot.is_zero() {
        return;
    }
    for scope in [
        DashboardNetworkScopeKey::Global,
        DashboardNetworkScopeKey::account_scope(upstream_account_id),
    ] {
        remove_activity_bucket_delta_locked(inner, scope, observed_epoch_second, snapshot);
    }
}

fn remove_activity_bucket_delta_locked(
    inner: &mut DashboardNetworkSpeedCacheInner,
    scope: DashboardNetworkScopeKey,
    observed_epoch_second: i64,
    snapshot: DashboardActivityCurrentSnapshot,
) {
    let Some(buckets) = inner.activity_second_buckets.get_mut(&scope) else {
        return;
    };
    if let Some(bucket) = buckets
        .iter_mut()
        .find(|bucket| bucket.epoch_second == observed_epoch_second)
    {
        bucket.snapshot.subtract_assign(snapshot);
    }
    while buckets
        .front()
        .is_some_and(|bucket| bucket.snapshot.is_zero())
    {
        buckets.pop_front();
    }
    while buckets
        .back()
        .is_some_and(|bucket| bucket.snapshot.is_zero())
    {
        buckets.pop_back();
    }
    buckets.retain(|bucket| !bucket.snapshot.is_zero());
}

fn prune_second_buckets_locked(inner: &mut DashboardNetworkSpeedCacheInner, now_epoch_second: i64) {
    let cutoff_epoch_second =
        now_epoch_second.saturating_sub(DASHBOARD_NETWORK_SECOND_BUCKET_RETENTION_SECONDS);
    inner.second_buckets.retain(|_, buckets| {
        while let Some(front) = buckets.front() {
            if front.epoch_second < cutoff_epoch_second {
                buckets.pop_front();
            } else {
                break;
            }
        }
        !buckets.is_empty()
    });
    inner.host_second_buckets.retain(|_, buckets| {
        while let Some(front) = buckets.front() {
            if front.epoch_second < cutoff_epoch_second {
                buckets.pop_front();
            } else {
                break;
            }
        }
        !buckets.is_empty()
    });
}

fn prune_activity_second_buckets_locked(
    inner: &mut DashboardNetworkSpeedCacheInner,
    now_epoch_second: i64,
) {
    let cutoff_epoch_second =
        now_epoch_second.saturating_sub(DASHBOARD_ACTIVITY_SECOND_BUCKET_RETENTION_SECONDS);
    inner.activity_second_buckets.retain(|_, buckets| {
        while let Some(front) = buckets.front() {
            if front.epoch_second < cutoff_epoch_second {
                buckets.pop_front();
            } else {
                break;
            }
        }
        buckets.retain(|bucket| !bucket.snapshot.is_zero());
        !buckets.is_empty()
    });
}

fn ensure_open_bucket_locked(
    inner: &mut DashboardNetworkSpeedCacheInner,
    scope: DashboardNetworkScopeKey,
    bucket_start_epoch_second: i64,
) {
    let bucket = inner
        .open_buckets
        .entry(scope)
        .or_insert(DashboardNetworkOpenBucketState {
            bucket_start_epoch_second,
            seed_totals: DashboardNetworkByteTotals::default(),
            delta_totals: DashboardNetworkByteTotals::default(),
            seeded: false,
        });
    if bucket.bucket_start_epoch_second != bucket_start_epoch_second {
        *bucket = DashboardNetworkOpenBucketState {
            bucket_start_epoch_second,
            seed_totals: DashboardNetworkByteTotals::default(),
            delta_totals: DashboardNetworkByteTotals::default(),
            seeded: false,
        };
    }
}

fn snapshot_open_bucket_locked(
    inner: &DashboardNetworkSpeedCacheInner,
    scope: DashboardNetworkScopeKey,
    now_epoch_second: i64,
) -> DashboardNetworkOpenBucketSnapshot {
    let bucket = inner
        .open_buckets
        .get(&scope)
        .expect("open bucket should exist before snapshot");
    let bucket_start = epoch_second_to_utc(bucket.bucket_start_epoch_second);
    let bucket_end_epoch_second = bucket
        .bucket_start_epoch_second
        .saturating_add(DASHBOARD_NETWORK_BUCKET_SECONDS);
    let bucket_end = epoch_second_to_utc(bucket_end_epoch_second);
    let elapsed_seconds = now_epoch_second
        .saturating_sub(bucket.bucket_start_epoch_second)
        .saturating_add(1)
        .clamp(1, DASHBOARD_NETWORK_BUCKET_SECONDS);
    let mut totals = bucket.seed_totals;
    totals.add_assign(bucket.delta_totals);
    DashboardNetworkOpenBucketSnapshot {
        bucket_start,
        bucket_end,
        elapsed_seconds,
        totals,
    }
}

fn current_bucket_start_epoch_second(epoch_second: i64) -> i64 {
    epoch_second - epoch_second.rem_euclid(DASHBOARD_NETWORK_BUCKET_SECONDS)
}

fn current_minute_start_epoch_second(epoch_second: i64) -> i64 {
    epoch_second - epoch_second.rem_euclid(60)
}

fn epoch_second_to_utc(epoch_second: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(epoch_second, 0)
        .single()
        .expect("valid UTC second bucket")
}

fn normalize_dashboard_network_upstream_host(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

async fn upsert_dashboard_network_socket_minute_rows_tx(
    tx: &mut SqliteConnection,
    rows: &[DashboardNetworkMinuteRollupRow],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }

    let mut aggregates =
        BTreeMap::<(i64, String, String, Option<i64>), DashboardNetworkByteTotals>::new();
    for row in rows {
        let entry = aggregates
            .entry((
                row.bucket_start_epoch,
                row.source.clone(),
                row.upstream_base_url_host.clone(),
                row.upstream_account_id,
            ))
            .or_default();
        entry.upload_bytes = entry.upload_bytes.saturating_add(row.upload_bytes.max(0));
        entry.download_bytes = entry
            .download_bytes
            .saturating_add(row.download_bytes.max(0));
    }

    for ((bucket_start_epoch, source, upstream_base_url_host, upstream_account_id), totals) in
        aggregates
    {
        sqlx::query(
            r#"
            INSERT INTO upstream_socket_network_minute (
                bucket_start_epoch,
                source,
                upstream_base_url_host,
                upstream_account_id,
                upload_bytes,
                download_bytes,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
            ON CONFLICT(bucket_start_epoch, source, upstream_base_url_host, upstream_account_id) DO UPDATE SET
                upload_bytes = upstream_socket_network_minute.upload_bytes + excluded.upload_bytes,
                download_bytes = upstream_socket_network_minute.download_bytes + excluded.download_bytes,
                updated_at = datetime('now')
            "#,
        )
        .bind(bucket_start_epoch)
        .bind(source)
        .bind(upstream_base_url_host)
        .bind(upstream_account_id)
        .bind(totals.upload_bytes.max(0))
        .bind(totals.download_bytes.max(0))
        .execute(&mut *tx)
        .await?;
    }

    Ok(())
}

pub(crate) async fn flush_dashboard_network_socket_minute_rollups(
    pool: &Pool<Sqlite>,
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
    observed_at: DateTime<Utc>,
) -> Result<u64> {
    let rows = dashboard_network_speed_cache.drain_completed_socket_minute_rows(observed_at);
    if rows.is_empty() {
        return Ok(0);
    }

    let mut tx = pool.begin().await?;
    if let Err(err) = upsert_dashboard_network_socket_minute_rows_tx(tx.as_mut(), &rows).await {
        let _ = tx.rollback().await;
        dashboard_network_speed_cache.restore_completed_socket_minute_rows(rows);
        return Err(err).context("failed to flush dashboard socket minute rollups");
    }
    if let Err(err) = tx.commit().await {
        dashboard_network_speed_cache.restore_completed_socket_minute_rows(rows);
        return Err(err).context("failed to commit dashboard socket minute rollups");
    }

    Ok(rows.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_utc(second: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(second, 0)
            .single()
            .expect("valid fixed second")
    }

    #[test]
    fn realtime_rates_use_previous_complete_second_only() {
        let cache = DashboardNetworkSpeedCache::new(fixed_utc(0));
        cache.record_request_bytes(
            "invoke-1",
            "2026-07-15 12:00:00",
            Some(7),
            Some("api.openai.com"),
            30,
            fixed_utc(10),
        );
        cache.record_request_bytes(
            "invoke-2",
            "2026-07-15 12:00:01",
            Some(7),
            Some("api.openai.com"),
            45,
            fixed_utc(24),
        );
        cache.record_response_chunk_bytes(
            "invoke-3",
            "2026-07-15 12:00:02",
            Some(7),
            Some("api.openai.com"),
            60,
            fixed_utc(24),
        );

        let rates = cache.snapshot_account_rates(fixed_utc(25));
        let account = rates.get(&Some(7)).copied().unwrap_or_default();
        assert!((account.upload_bytes_per_second - 45.0).abs() < f64::EPSILON);
        assert!((account.download_bytes_per_second - 60.0).abs() < f64::EPSILON);
    }

    #[test]
    fn request_recording_accumulates_attempt_bytes_and_finish_cleans_state() {
        let cache = DashboardNetworkSpeedCache::new(fixed_utc(0));
        cache.record_request_bytes(
            "invoke-1",
            "2026-07-15 12:00:00",
            None,
            Some("api.openai.com"),
            120,
            fixed_utc(100),
        );
        cache.record_request_bytes(
            "invoke-1",
            "2026-07-15 12:00:00",
            None,
            Some("api.openai.com"),
            120,
            fixed_utc(100),
        );
        cache.record_request_bytes(
            "invoke-1",
            "2026-07-15 12:00:00",
            None,
            Some("api.openai.com"),
            150,
            fixed_utc(100),
        );
        cache.finish_invocation("invoke-1", "2026-07-15 12:00:00");

        let rates = cache.snapshot_account_rates(fixed_utc(101));
        let account = rates.get(&None).copied().unwrap_or_default();
        assert!((account.upload_bytes_per_second - 390.0).abs() < f64::EPSILON);
    }

    #[test]
    fn realtime_rates_ignore_current_second_partial_bytes() {
        let cache = DashboardNetworkSpeedCache::new(fixed_utc(0));
        cache.record_request_bytes(
            "invoke-1",
            "2026-07-15 12:00:00",
            Some(7),
            Some("api.openai.com"),
            90,
            fixed_utc(200),
        );
        cache.record_request_bytes(
            "invoke-1",
            "2026-07-15 12:00:00",
            Some(7),
            Some("api.openai.com"),
            120,
            fixed_utc(201),
        );

        let rates = cache.snapshot_account_rates(fixed_utc(201));
        let account = rates.get(&Some(7)).copied().unwrap_or_default();
        assert!((account.upload_bytes_per_second - 90.0).abs() < f64::EPSILON);
    }

    #[test]
    fn global_realtime_rate_reads_previous_complete_second() {
        let cache = DashboardNetworkSpeedCache::new(fixed_utc(0));
        cache.record_request_bytes(
            "invoke-1",
            "2026-07-15 12:00:00",
            Some(7),
            Some("api.openai.com"),
            8240,
            fixed_utc(320),
        );
        cache.record_response_chunk_bytes(
            "invoke-2",
            "2026-07-15 12:00:01",
            Some(8),
            Some("api.openai.com"),
            4096,
            fixed_utc(320),
        );

        let snapshot =
            cache.snapshot_scope_realtime_bytes(DashboardNetworkScopeKey::Global, fixed_utc(321));
        let rate =
            DashboardNetworkRateSnapshot::from_totals(snapshot.totals, snapshot.sample_seconds);
        assert_eq!(snapshot.sample_start_epoch_second, 320);
        assert_eq!(snapshot.sample_end_epoch_second, 321);
        assert_eq!(snapshot.totals.upload_bytes, 8240);
        assert_eq!(snapshot.totals.download_bytes, 4096);
        assert!((rate.upload_bytes_per_second - 8240.0).abs() < f64::EPSILON);
        assert!((rate.download_bytes_per_second - 4096.0).abs() < f64::EPSILON);
    }

    #[test]
    fn recent_global_window_keeps_three_hundred_seconds_of_complete_samples() {
        let cache = DashboardNetworkSpeedCache::new(fixed_utc(0));
        cache.record_request_bytes(
            "invoke-1",
            "2026-07-15 12:00:00",
            Some(7),
            Some("api.openai.com"),
            600,
            fixed_utc(900),
        );
        cache.record_response_chunk_bytes(
            "invoke-2",
            "2026-07-15 12:00:01",
            Some(7),
            Some("api.openai.com"),
            900,
            fixed_utc(1199),
        );

        let snapshot = cache.snapshot_recent_global_window(fixed_utc(1200));

        assert_eq!(snapshot.range_start_epoch_second, 900);
        assert_eq!(snapshot.range_end_epoch_second, 1200);
        assert_eq!(snapshot.window_seconds, 300);
        assert_eq!(snapshot.sample_seconds, 1);
        assert!(!snapshot.is_warming_up);
        assert_eq!(snapshot.points.len(), 300);
        assert_eq!(
            snapshot
                .points
                .first()
                .map(|point| point.sample_start_epoch_second),
            Some(900)
        );
        assert_eq!(
            snapshot
                .points
                .first()
                .map(|point| point.totals.upload_bytes),
            Some(600)
        );
        assert_eq!(
            snapshot
                .points
                .last()
                .map(|point| point.sample_start_epoch_second),
            Some(1199)
        );
        assert_eq!(
            snapshot
                .points
                .last()
                .map(|point| point.totals.download_bytes),
            Some(900)
        );
    }

    #[test]
    fn recent_global_window_marks_prestart_gap_unavailable_but_keeps_runtime_zeros_available() {
        let cache = DashboardNetworkSpeedCache::new(fixed_utc(1_150));
        cache.record_request_bytes(
            "invoke-1",
            "2026-07-15 12:00:00",
            Some(3),
            Some("api.openai.com"),
            240,
            fixed_utc(1_180),
        );

        let snapshot = cache.snapshot_recent_global_window(fixed_utc(1_200));

        assert!(snapshot.is_warming_up);
        let unavailable_point = snapshot
            .points
            .iter()
            .find(|point| point.sample_start_epoch_second == 900)
            .expect("expected leading unavailable point");
        assert!(!unavailable_point.is_available);
        assert_eq!(unavailable_point.totals.upload_bytes, 0);

        let zero_available_point = snapshot
            .points
            .iter()
            .find(|point| point.sample_start_epoch_second == 1_179)
            .expect("expected runtime zero point");
        assert!(zero_available_point.is_available);
        assert_eq!(zero_available_point.totals.upload_bytes, 0);

        let populated_point = snapshot
            .points
            .iter()
            .find(|point| point.sample_start_epoch_second == 1_180)
            .expect("expected populated runtime point");
        assert!(populated_point.is_available);
        assert_eq!(populated_point.totals.upload_bytes, 240);
    }

    #[test]
    fn open_bucket_seed_merges_with_runtime_delta() {
        let cache = DashboardNetworkSpeedCache::new(fixed_utc(0));
        cache.record_request_bytes(
            "invoke-1",
            "2026-07-15 12:00:00",
            Some(3),
            Some("api.openai.com"),
            120,
            fixed_utc(620),
        );

        let read =
            cache.open_bucket_read_state(DashboardNetworkScopeKey::Account(3), fixed_utc(620));
        assert!(read.needs_seed);

        let snapshot = cache.seed_open_bucket(
            DashboardNetworkScopeKey::Account(3),
            read.bucket_start,
            DashboardNetworkByteTotals {
                upload_bytes: 300,
                download_bytes: 500,
            },
            fixed_utc(620),
        );

        assert_eq!(snapshot.totals.upload_bytes, 420);
        assert_eq!(snapshot.totals.download_bytes, 500);
    }

    #[test]
    fn heartbeat_stays_alive_until_the_live_bucket_rolls_over() {
        let cache = DashboardNetworkSpeedCache::new(fixed_utc(0));
        cache.record_response_chunk_bytes(
            "invoke-1",
            "2026-07-15 12:00:00",
            Some(4),
            Some("api.openai.com"),
            64,
            fixed_utc(200),
        );

        assert!(cache.should_keep_dashboard_activity_live_stream(fixed_utc(214)));
        assert!(cache.should_keep_dashboard_activity_live_stream(fixed_utc(299)));
        assert!(!cache.should_keep_dashboard_activity_live_stream(fixed_utc(300)));
    }

    #[test]
    fn seeded_open_bucket_keeps_live_stream_running_without_recent_chunks() {
        let cache = DashboardNetworkSpeedCache::new(fixed_utc(0));
        let read = cache.open_bucket_read_state(DashboardNetworkScopeKey::Global, fixed_utc(620));
        cache.seed_open_bucket(
            DashboardNetworkScopeKey::Global,
            read.bucket_start,
            DashboardNetworkByteTotals {
                upload_bytes: 120,
                download_bytes: 240,
            },
            fixed_utc(620),
        );

        assert!(cache.should_keep_dashboard_activity_live_stream(fixed_utc(700)));
        assert!(!cache.should_keep_dashboard_activity_live_stream(fixed_utc(900)));
    }
}
