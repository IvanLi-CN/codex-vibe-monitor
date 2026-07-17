use super::*;

pub(crate) const DASHBOARD_NETWORK_REALTIME_WINDOW_SECONDS: i64 = 15;
pub(crate) const DASHBOARD_NETWORK_BUCKET_SECONDS: i64 = 300;
const DASHBOARD_NETWORK_SECOND_BUCKET_RETENTION_SECONDS: i64 = 45;
pub(crate) const DASHBOARD_ACTIVITY_REALTIME_WINDOW_SECONDS: i64 = 60;
const DASHBOARD_ACTIVITY_SECOND_BUCKET_RETENTION_SECONDS: i64 = 180;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct DashboardNetworkRateSnapshot {
    pub(crate) upload_bytes_per_second: f64,
    pub(crate) download_bytes_per_second: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DashboardNetworkByteTotals {
    pub(crate) upload_bytes: i64,
    pub(crate) download_bytes: i64,
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

#[derive(Debug, Default)]
struct DashboardTrackedInvocationTraffic {
    upstream_account_id: Option<i64>,
    upload_bytes_recorded: i64,
    download_bytes_recorded: i64,
    live_first_response_byte_epoch_second: Option<i64>,
    live_first_response_byte_total_ms: Option<f64>,
}

#[derive(Debug, Default)]
struct DashboardNetworkSpeedCacheInner {
    second_buckets: HashMap<DashboardNetworkScopeKey, VecDeque<DashboardNetworkSecondBucket>>,
    activity_second_buckets:
        HashMap<DashboardNetworkScopeKey, VecDeque<DashboardActivitySecondBucket>>,
    open_buckets: HashMap<DashboardNetworkScopeKey, DashboardNetworkOpenBucketState>,
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
        let delta = bytes.saturating_sub(tracked.upload_bytes_recorded).max(0);
        if delta <= 0 {
            return;
        }
        tracked.upload_bytes_recorded = bytes;
        let tracked_upstream_account_id = tracked.upstream_account_id;
        record_scope_delta_locked(
            &mut inner,
            tracked_upstream_account_id,
            observed_at.timestamp(),
            DashboardNetworkByteTotals {
                upload_bytes: delta,
                download_bytes: 0,
            },
        );
    }

    pub(crate) fn record_response_chunk_bytes(
        &self,
        invoke_id: &str,
        occurred_at: &str,
        upstream_account_id: Option<i64>,
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
        tracked.download_bytes_recorded = tracked.download_bytes_recorded.saturating_add(bytes);
        let tracked_upstream_account_id = tracked.upstream_account_id;
        record_scope_delta_locked(
            &mut inner,
            tracked_upstream_account_id,
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
            let mut totals = DashboardNetworkByteTotals::default();
            for bucket in buckets {
                if bucket.epoch_second
                    < now_epoch_second
                        .saturating_sub(DASHBOARD_NETWORK_REALTIME_WINDOW_SECONDS.saturating_sub(1))
                {
                    continue;
                }
                if bucket.epoch_second > now_epoch_second {
                    continue;
                }
                totals.add_assign(bucket.totals);
            }
            rates.insert(
                upstream_account_id,
                DashboardNetworkRateSnapshot {
                    upload_bytes_per_second: totals.upload_bytes.max(0) as f64
                        / DASHBOARD_NETWORK_REALTIME_WINDOW_SECONDS as f64,
                    download_bytes_per_second: totals.download_bytes.max(0) as f64
                        / DASHBOARD_NETWORK_REALTIME_WINDOW_SECONDS as f64,
                },
            );
        }
        rates
    }

    pub(crate) fn should_keep_dashboard_activity_live_stream(&self, now: DateTime<Utc>) -> bool {
        let now_epoch_second = now.timestamp();
        let inner = self
            .inner
            .lock()
            .expect("dashboard network speed cache should not be poisoned");
        if inner.latest_speed_epoch_second.is_some_and(|latest| {
            now_epoch_second.saturating_sub(latest) < DASHBOARD_NETWORK_REALTIME_WINDOW_SECONDS
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

fn epoch_second_to_utc(epoch_second: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(epoch_second, 0)
        .single()
        .expect("valid UTC second bucket")
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
    fn realtime_rates_use_last_fifteen_seconds_only() {
        let cache = DashboardNetworkSpeedCache::new(fixed_utc(0));
        cache.record_request_bytes(
            "invoke-1",
            "2026-07-15 12:00:00",
            Some(7),
            30,
            fixed_utc(10),
        );
        cache.record_request_bytes(
            "invoke-2",
            "2026-07-15 12:00:01",
            Some(7),
            45,
            fixed_utc(24),
        );
        cache.record_response_chunk_bytes(
            "invoke-3",
            "2026-07-15 12:00:02",
            Some(7),
            60,
            fixed_utc(24),
        );

        let rates = cache.snapshot_account_rates(fixed_utc(24));
        let account = rates.get(&Some(7)).copied().unwrap_or_default();
        assert!((account.upload_bytes_per_second - 5.0).abs() < f64::EPSILON);
        assert!((account.download_bytes_per_second - 4.0).abs() < f64::EPSILON);
    }

    #[test]
    fn request_recording_is_idempotent_and_finish_cleans_state() {
        let cache = DashboardNetworkSpeedCache::new(fixed_utc(0));
        cache.record_request_bytes("invoke-1", "2026-07-15 12:00:00", None, 120, fixed_utc(100));
        cache.record_request_bytes("invoke-1", "2026-07-15 12:00:00", None, 120, fixed_utc(100));
        cache.record_request_bytes("invoke-1", "2026-07-15 12:00:00", None, 150, fixed_utc(100));
        cache.finish_invocation("invoke-1", "2026-07-15 12:00:00");

        let rates = cache.snapshot_account_rates(fixed_utc(100));
        let account = rates.get(&None).copied().unwrap_or_default();
        assert!((account.upload_bytes_per_second - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn open_bucket_seed_merges_with_runtime_delta() {
        let cache = DashboardNetworkSpeedCache::new(fixed_utc(0));
        cache.record_request_bytes(
            "invoke-1",
            "2026-07-15 12:00:00",
            Some(3),
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
