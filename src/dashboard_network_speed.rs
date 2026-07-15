use super::*;

pub(crate) const DASHBOARD_NETWORK_REALTIME_WINDOW_SECONDS: i64 = 15;
pub(crate) const DASHBOARD_NETWORK_BUCKET_SECONDS: i64 = 300;
const DASHBOARD_NETWORK_SECOND_BUCKET_RETENTION_SECONDS: i64 = 45;

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
}

#[derive(Debug, Default)]
struct DashboardNetworkSpeedCacheInner {
    second_buckets: HashMap<DashboardNetworkScopeKey, VecDeque<DashboardNetworkSecondBucket>>,
    open_buckets: HashMap<DashboardNetworkScopeKey, DashboardNetworkOpenBucketState>,
    tracked_invocations: HashMap<(String, String), DashboardTrackedInvocationTraffic>,
    latest_speed_epoch_second: Option<i64>,
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
        inner.latest_speed_epoch_second.is_some_and(|latest| {
            now_epoch_second.saturating_sub(latest) < DASHBOARD_NETWORK_REALTIME_WINDOW_SECONDS
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
    fn heartbeat_expires_after_realtime_window() {
        let cache = DashboardNetworkSpeedCache::new(fixed_utc(0));
        cache.record_response_chunk_bytes(
            "invoke-1",
            "2026-07-15 12:00:00",
            Some(4),
            64,
            fixed_utc(200),
        );

        assert!(cache.should_keep_dashboard_activity_live_stream(fixed_utc(214)));
        assert!(!cache.should_keep_dashboard_activity_live_stream(fixed_utc(216)));
    }
}
