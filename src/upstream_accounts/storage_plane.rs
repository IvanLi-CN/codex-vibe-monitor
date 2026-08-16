use super::*;
use crate::db_pressure::global_db_pressure_gate;
use crate::maintenance::{
    StartupBackfillProgressUpdate, StartupBackfillTask, load_startup_backfill_progress,
    save_startup_backfill_progress, wake_startup_backfill_tasks,
};
use std::{
    collections::{BTreeSet, HashMap},
    hash::{Hash, Hasher},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

const ACCOUNT_WINDOW_STORAGE_MAX_ENTRIES: usize = 128;
const ACCOUNT_WINDOW_STORAGE_IDLE_TTL: Duration = Duration::from_secs(10 * 60);
const ACCOUNT_WINDOW_LAST_GOOD_MAX_AGE: Duration = Duration::from_secs(60);
const ACCOUNT_WINDOW_PREPARING_RETRY_AFTER_MS: u64 = 1_000;
const ACCOUNT_WINDOW_LEGACY_BACKFILL_WRITE_BATCH_SIZE: i64 = 32;
const ACCOUNT_WINDOW_LEGACY_BACKFILL_BUDGET: Duration = Duration::from_millis(200);
const ACCOUNT_WINDOW_LEGACY_BACKFILL_ROLLUP_MAX_ROWS: usize = 200;
const ACCOUNT_WINDOW_LEGACY_BACKFILL_FAIRNESS_INTERVAL: Duration = Duration::from_secs(15);
const ACCOUNT_WINDOW_LEGACY_BACKFILL_PROGRESS_KEY: &str =
    "account_window_usage_upstream_account_id";
const ACCOUNT_WINDOW_LEGACY_BACKFILL_STATE_PENDING: &str = "pending";
const ACCOUNT_WINDOW_LIVE_COVERAGE_REPAIR_BATCH_SIZE: usize = 2;
const ACCOUNT_WINDOW_LIVE_COVERAGE_REPAIR_MAX_ROWS: usize = 1_000;
const ACCOUNT_WINDOW_LIVE_COVERAGE_FULL_REPAIR_BATCH_SIZE: usize = 1;
const ACCOUNT_WINDOW_LIVE_COVERAGE_REPAIR_FAIRNESS_INTERVAL: Duration = Duration::from_secs(15);
const ACCOUNT_WINDOW_LIVE_COVERAGE_REPAIR_BUDGET: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AccountWindowStoragePlaneHealthSnapshot {
    pub(crate) state: String,
    pub(crate) active_selection_count: usize,
    pub(crate) in_flight_build_count: usize,
    pub(crate) coalesced_waiter_count: u64,
    pub(crate) rollup_row_count: u64,
    pub(crate) minute_row_count: u64,
    pub(crate) bounded_raw_row_count: u64,
    pub(crate) coverage_hole_bucket_count: u64,
    pub(crate) backfill_cursor: i64,
    pub(crate) last_good_age_ms: Option<u64>,
    pub(crate) direct_pool_violation_count: u64,
    pub(crate) last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum AccountWindowStorageResponse {
    Ready(UpstreamAccountWindowUsageResponse),
    Preparing { retry_after_ms: u64 },
}

#[derive(Debug)]
struct AccountWindowLoadFlight {
    result_tx: watch::Sender<Option<AccountWindowStorageResponse>>,
    result_rx: watch::Receiver<Option<AccountWindowStorageResponse>>,
}

#[derive(Debug, Clone)]
struct AccountWindowPreflight {
    summaries: Vec<UpstreamAccountSummary>,
    durable_cursor: i64,
    selection: String,
    legacy_ranges: Vec<AccountWindowLegacyBackfillRange>,
    legacy_backfill_required: bool,
    legacy_backfill_complete: bool,
    selection_now: DateTime<Utc>,
}

#[derive(Debug, Clone)]
enum AccountWindowPreflightResult {
    Ready(AccountWindowPreflight),
    Deferred,
}

#[derive(Debug)]
struct AccountWindowPreflightFlight {
    result_tx: watch::Sender<Option<AccountWindowPreflightResult>>,
    result_rx: watch::Receiver<Option<AccountWindowPreflightResult>>,
}

impl AccountWindowPreflightFlight {
    fn new() -> Self {
        let (result_tx, result_rx) = watch::channel(None);
        Self {
            result_tx,
            result_rx,
        }
    }

    fn subscribe(&self) -> watch::Receiver<Option<AccountWindowPreflightResult>> {
        self.result_rx.clone()
    }

    fn complete(&self, result: AccountWindowPreflightResult) {
        self.result_tx.send_replace(Some(result));
    }
}

impl AccountWindowLoadFlight {
    fn new() -> Self {
        let (result_tx, result_rx) = watch::channel(None);
        Self {
            result_tx,
            result_rx,
        }
    }

    fn subscribe(&self) -> watch::Receiver<Option<AccountWindowStorageResponse>> {
        self.result_rx.clone()
    }

    fn complete(&self, response: AccountWindowStorageResponse) {
        self.result_tx.send_replace(Some(response));
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum AccountWindowLegacyReadiness {
    #[default]
    Unknown,
    Checking,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccountWindowLegacyBackfillOutcome {
    Ready,
    Pending,
}

#[derive(Debug)]
struct AccountWindowSelectionEntry {
    in_flight: Option<Arc<AccountWindowLoadFlight>>,
    last_used_at: Instant,
    last_good: Option<AccountWindowLastGood>,
    coverage_hole_bucket_count: u64,
    last_error: Option<String>,
    legacy_readiness: AccountWindowLegacyReadiness,
}

#[derive(Debug, Clone)]
struct AccountWindowLastGood {
    response: UpstreamAccountWindowUsageResponse,
    stored_at: Instant,
    durable_cursor: i64,
}

impl Default for AccountWindowSelectionEntry {
    fn default() -> Self {
        Self {
            in_flight: None,
            last_used_at: Instant::now(),
            last_good: None,
            coverage_hole_bucket_count: 0,
            last_error: None,
            legacy_readiness: AccountWindowLegacyReadiness::Unknown,
        }
    }
}

#[derive(Debug, Default)]
struct AccountWindowStorageHealthState {
    rollup_row_count: u64,
    minute_row_count: u64,
    bounded_raw_row_count: u64,
    backfill_cursor: i64,
}

#[derive(Debug, Clone)]
struct AccountWindowLegacyBackfillRange {
    account_id: i64,
    start_at: String,
    end_at: String,
    selection_generation: String,
    window_duration_secs: i64,
}

#[derive(Debug, Clone)]
struct AccountWindowBuildResult {
    response: AccountWindowStorageResponse,
    telemetry: AccountWindowUsageBuildTelemetry,
}

#[derive(Debug, Default)]
struct AccountWindowLiveCoverageRepairQueue {
    pending_invocation_ids: BTreeSet<i64>,
    pending_full_recompute_ids: BTreeSet<i64>,
    refresh_live_cursor: bool,
    in_flight: Option<AccountWindowLiveCoverageRepairBatch>,
    worker_running: bool,
}

#[derive(Debug, Clone)]
struct AccountWindowLiveCoverageRepairBatch {
    invocation_ids: Vec<i64>,
    refresh_live_cursor: bool,
    full_bucket_recompute: bool,
}

struct AccountWindowLiveCoverageRepairWorkerGuard {
    repairs: Arc<Mutex<AccountWindowLiveCoverageRepairQueue>>,
    completed: bool,
}

impl AccountWindowLiveCoverageRepairWorkerGuard {
    fn complete(&mut self) {
        self.completed = true;
    }
}

impl Drop for AccountWindowLiveCoverageRepairWorkerGuard {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let repairs = self.repairs.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut repairs = repairs.lock().await;
                if let Some(batch) = repairs.in_flight.take() {
                    requeue_live_coverage_repair_batch(&mut repairs, batch);
                }
                repairs.worker_running = false;
            });
        }
    }
}

fn requeue_live_coverage_repair_batch(
    repairs: &mut AccountWindowLiveCoverageRepairQueue,
    batch: AccountWindowLiveCoverageRepairBatch,
) {
    if batch.full_bucket_recompute {
        repairs
            .pending_full_recompute_ids
            .extend(batch.invocation_ids);
    } else {
        repairs.pending_invocation_ids.extend(batch.invocation_ids);
    }
    repairs.refresh_live_cursor |= batch.refresh_live_cursor;
}

struct AccountWindowLoadLease {
    entries: Arc<Mutex<HashMap<String, AccountWindowSelectionEntry>>>,
    selection: String,
    flight: Arc<AccountWindowLoadFlight>,
    completed: bool,
}

struct AccountWindowPreflightLease {
    flights: Arc<Mutex<HashMap<String, Arc<AccountWindowPreflightFlight>>>>,
    key: String,
    flight: Arc<AccountWindowPreflightFlight>,
    completed: bool,
}

impl AccountWindowPreflightLease {
    fn complete(&mut self) {
        self.completed = true;
    }
}

impl AccountWindowLoadLease {
    fn complete(&mut self) {
        self.completed = true;
    }
}

impl Drop for AccountWindowLoadLease {
    fn drop(&mut self) {
        if self.completed {
            return;
        }

        let entries = self.entries.clone();
        let selection = self.selection.clone();
        let flight = self.flight.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut entries = entries.lock().await;
                if let Some(entry) = entries.get_mut(&selection)
                    && entry
                        .in_flight
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &flight))
                {
                    entry.in_flight = None;
                    flight.complete(AccountWindowStorageResponse::Preparing {
                        retry_after_ms: ACCOUNT_WINDOW_PREPARING_RETRY_AFTER_MS,
                    });
                }
            });
        }
    }
}

impl Drop for AccountWindowPreflightLease {
    fn drop(&mut self) {
        if self.completed {
            return;
        }

        let flights = self.flights.clone();
        let key = self.key.clone();
        let flight = self.flight.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut flights = flights.lock().await;
                if flights
                    .get(&key)
                    .is_some_and(|current| Arc::ptr_eq(current, &flight))
                {
                    flights.remove(&key);
                    flight.complete(AccountWindowPreflightResult::Deferred);
                }
            });
        }
    }
}

#[derive(Debug)]
pub(crate) struct AccountWindowStoragePlane {
    entries: Arc<Mutex<HashMap<String, AccountWindowSelectionEntry>>>,
    preflight_flights: Arc<Mutex<HashMap<String, Arc<AccountWindowPreflightFlight>>>>,
    health: Arc<std::sync::Mutex<AccountWindowStorageHealthState>>,
    coalesced_waiter_count: AtomicU64,
    legacy_backfill_state_loaded: AtomicBool,
    legacy_backfill_required: AtomicBool,
    // Completion is valid only at the durable cursor which proved this selection has no
    // legacy-shaped rows. Keep this bounded so one-off selections cannot become retained state.
    legacy_backfill_completed_cursors: Arc<Mutex<HashMap<String, i64>>>,
    backfill_running: Arc<AtomicBool>,
    coverage_repair_running: Arc<AtomicBool>,
    live_coverage_repairs: Arc<Mutex<AccountWindowLiveCoverageRepairQueue>>,
    direct_pool_violation_count: AtomicU64,
}

impl Default for AccountWindowStoragePlane {
    fn default() -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            preflight_flights: Arc::new(Mutex::new(HashMap::new())),
            health: Arc::new(std::sync::Mutex::new(
                AccountWindowStorageHealthState::default(),
            )),
            coalesced_waiter_count: AtomicU64::new(0),
            legacy_backfill_state_loaded: AtomicBool::new(false),
            legacy_backfill_required: AtomicBool::new(false),
            legacy_backfill_completed_cursors: Arc::new(Mutex::new(HashMap::new())),
            backfill_running: Arc::new(AtomicBool::new(false)),
            coverage_repair_running: Arc::new(AtomicBool::new(false)),
            live_coverage_repairs: Arc::new(Mutex::new(
                AccountWindowLiveCoverageRepairQueue::default(),
            )),
            direct_pool_violation_count: AtomicU64::new(0),
        }
    }
}

impl AccountWindowStoragePlane {
    pub(crate) async fn load(
        &self,
        state: &AppState,
        account_ids: &[i64],
    ) -> Result<AccountWindowStorageResponse> {
        let mut normalized_account_ids = account_ids.to_vec();
        normalized_account_ids.sort_unstable();
        normalized_account_ids.dedup();
        if normalized_account_ids.is_empty() {
            return Ok(AccountWindowStorageResponse::Ready(
                UpstreamAccountWindowUsageResponse { items: Vec::new() },
            ));
        }

        let selection_now = Utc::now();
        // A concurrent request for the same account set must share its metadata reads as well
        // as the builder. Otherwise a burst still multiplies SQLite preflight work before the
        // selection-level flight can admit followers.
        let Some(preflight) = self
            .load_preflight(state, &normalized_account_ids, selection_now)
            .await
        else {
            return Ok(AccountWindowStorageResponse::Preparing {
                retry_after_ms: ACCOUNT_WINDOW_PREPARING_RETRY_AFTER_MS,
            });
        };
        let AccountWindowPreflight {
            summaries,
            durable_cursor,
            selection,
            legacy_ranges,
            legacy_backfill_required,
            legacy_backfill_complete,
            selection_now,
        } = preflight;
        let last_good_compatibility_key = account_window_last_good_compatibility_key(&selection)
            .unwrap_or_else(|| account_window_selection_config_from_key(&selection).to_string());
        let (waiter, lease, schedule_legacy_backfill, immediate_response) = {
            let mut entries = self.entries.lock().await;
            Self::prune_entries(&mut entries);
            Self::make_room_for_selection(&mut entries, &selection);
            if !can_admit_account_window_selection(&entries, &selection) {
                // Never let a burst of distinct selections turn coordination state into an
                // unbounded cache. The caller retries through the normal preparing contract.
                return Ok(AccountWindowStorageResponse::Preparing {
                    retry_after_ms: ACCOUNT_WINDOW_PREPARING_RETRY_AFTER_MS,
                });
            }
            let compatible_last_good = entries
                .iter()
                .filter(|(candidate, _)| {
                    account_window_last_good_compatibility_key(candidate)
                        .as_deref()
                        .unwrap_or_else(|| account_window_selection_config_from_key(candidate))
                        == last_good_compatibility_key
                })
                .filter_map(|(_, entry)| entry.last_good.clone())
                .filter(|last_good| {
                    last_good.stored_at.elapsed() <= ACCOUNT_WINDOW_LAST_GOOD_MAX_AGE
                })
                .max_by_key(|last_good| (last_good.durable_cursor, last_good.stored_at));
            let entry = entries.entry(selection.clone()).or_default();
            if entry.last_good.is_none() {
                entry.last_good = compatible_last_good;
            }
            entry.last_used_at = Instant::now();
            if legacy_backfill_complete {
                entry.legacy_readiness = AccountWindowLegacyReadiness::Ready;
            }
            if legacy_backfill_required
                && !legacy_backfill_complete
                && !legacy_ranges.is_empty()
                && entry.legacy_readiness != AccountWindowLegacyReadiness::Ready
            {
                // A different selection may already own the single bounded pass. Once it
                // releases the shared worker, the next owner retry may enqueue this selection
                // instead of leaving it permanently in `checking`.
                let should_schedule = entry.legacy_readiness
                    == AccountWindowLegacyReadiness::Unknown
                    || !self.backfill_running.load(Ordering::Acquire);
                entry.legacy_readiness = AccountWindowLegacyReadiness::Checking;
                (
                    None,
                    None,
                    should_schedule,
                    // A matching legacy row is not represented by the structured account
                    // selection yet. A compatible last-good response can therefore omit it;
                    // wait for the bounded attribution pass instead of serving stale totals.
                    Some(AccountWindowStorageResponse::Preparing {
                        retry_after_ms: ACCOUNT_WINDOW_PREPARING_RETRY_AFTER_MS,
                    }),
                )
            } else if let Some(flight) = entry.in_flight.as_ref() {
                self.coalesced_waiter_count.fetch_add(1, Ordering::Relaxed);
                (Some(flight.subscribe()), None, false, None)
            } else {
                let flight = Arc::new(AccountWindowLoadFlight::new());
                entry.in_flight = Some(flight.clone());
                (
                    None,
                    Some(AccountWindowLoadLease {
                        entries: self.entries.clone(),
                        selection: selection.clone(),
                        flight,
                        completed: false,
                    }),
                    false,
                    None,
                )
            }
        };

        if let Some(response) = immediate_response {
            if schedule_legacy_backfill {
                self.schedule_legacy_backfill(state.pool.clone(), selection, legacy_ranges);
            }
            return Ok(response);
        }

        if let Some(mut waiter) = waiter {
            if waiter.changed().await.is_ok()
                && let Some(response) = waiter.borrow_and_update().clone()
            {
                return Ok(response);
            }
            return Ok(AccountWindowStorageResponse::Preparing {
                retry_after_ms: ACCOUNT_WINDOW_PREPARING_RETRY_AFTER_MS,
            });
        }

        let mut lease = lease.expect("leader selection must hold an in-flight lease");
        let response = self
            .build(
                state,
                summaries,
                &normalized_account_ids,
                durable_cursor,
                selection_now,
            )
            .await;
        let (response, refresh_last_good, coverage_hole_bucket_count, error) = match response {
            Ok(AccountWindowBuildResult {
                response: AccountWindowStorageResponse::Ready(response),
                telemetry,
            }) => (
                AccountWindowStorageResponse::Ready(response),
                true,
                telemetry.coverage_hole_bucket_count as u64,
                None,
            ),
            Ok(AccountWindowBuildResult {
                response: AccountWindowStorageResponse::Preparing { retry_after_ms },
                telemetry,
            }) => (
                self.last_good_response(&selection)
                    .await
                    .unwrap_or(AccountWindowStorageResponse::Preparing { retry_after_ms }),
                false,
                telemetry.coverage_hole_bucket_count as u64,
                None,
            ),
            Err(err) => {
                let message = err.to_string();
                (
                    self.last_good_response(&selection).await.unwrap_or(
                        AccountWindowStorageResponse::Preparing {
                            retry_after_ms: ACCOUNT_WINDOW_PREPARING_RETRY_AFTER_MS,
                        },
                    ),
                    false,
                    0,
                    Some(message),
                )
            }
        };
        self.complete_load(
            &selection,
            response.clone(),
            refresh_last_good,
            durable_cursor,
            coverage_hole_bucket_count,
            error,
        )
        .await;
        lease.complete();

        Ok(response)
    }

    async fn load_preflight(
        &self,
        state: &AppState,
        account_ids: &[i64],
        selection_now: DateTime<Utc>,
    ) -> Option<AccountWindowPreflight> {
        let key = account_window_preflight_key(account_ids);
        let (waiter, lease) = {
            let mut flights = self.preflight_flights.lock().await;
            if let Some(flight) = flights.get(&key) {
                self.coalesced_waiter_count.fetch_add(1, Ordering::Relaxed);
                (Some(flight.subscribe()), None)
            } else {
                let flight = Arc::new(AccountWindowPreflightFlight::new());
                flights.insert(key.clone(), flight.clone());
                (
                    None,
                    Some(AccountWindowPreflightLease {
                        flights: self.preflight_flights.clone(),
                        key: key.clone(),
                        flight,
                        completed: false,
                    }),
                )
            }
        };

        if let Some(mut waiter) = waiter {
            return match waiter.changed().await {
                Ok(()) => match waiter.borrow_and_update().clone() {
                    Some(AccountWindowPreflightResult::Ready(preflight)) => Some(preflight),
                    Some(AccountWindowPreflightResult::Deferred) | None => None,
                },
                Err(_) => None,
            };
        }

        let mut lease = lease.expect("preflight leader must hold a lease");
        let result = match self
            .resolve_preflight(state, account_ids, selection_now)
            .await
        {
            Ok(preflight) => AccountWindowPreflightResult::Ready(preflight),
            Err(error) => {
                tracing::warn!(
                    route = "upstream_account_window_usage",
                    builder = "account_window_preflight",
                    error = %error,
                    "upstream account window usage preflight deferred"
                );
                AccountWindowPreflightResult::Deferred
            }
        };
        {
            let mut flights = self.preflight_flights.lock().await;
            if flights
                .get(&key)
                .is_some_and(|current| Arc::ptr_eq(current, &lease.flight))
            {
                flights.remove(&key);
            }
        }
        lease.flight.complete(result.clone());
        lease.complete();
        match result {
            AccountWindowPreflightResult::Ready(preflight) => Some(preflight),
            AccountWindowPreflightResult::Deferred => None,
        }
    }

    async fn resolve_preflight(
        &self,
        state: &AppState,
        account_ids: &[i64],
        selection_now: DateTime<Utc>,
    ) -> Result<AccountWindowPreflight> {
        let summaries =
            load_upstream_account_window_usage_summaries(&state.pool, &state.config, account_ids)
                .await?;
        let plans = collect_account_window_usage_plans(&summaries, selection_now)
            .map(|(plans, _, _)| plans)
            .unwrap_or_default();
        let durable_cursor = durable_cursor_for_account_selection(&state.pool, account_ids).await?;
        let selection_config = account_window_selection_config_key(account_ids, &plans);
        let selection = account_window_selection_key(&selection_config, durable_cursor);
        let legacy_ranges = legacy_backfill_ranges_from_plans(&plans);
        let legacy_backfill_required = self.legacy_backfill_required(&state.pool).await?;
        let legacy_backfill_complete = if legacy_backfill_required && !legacy_ranges.is_empty() {
            let completion_key = legacy_backfill_completion_key(&legacy_ranges);
            if self
                .legacy_backfill_completed_cursors
                .lock()
                .await
                .get(&completion_key)
                .is_some_and(|cursor| *cursor == durable_cursor)
            {
                true
            } else {
                Self::legacy_backfill_complete(&state.pool, &legacy_ranges).await?
            }
        } else {
            true
        };
        if legacy_backfill_required && legacy_backfill_complete && !legacy_ranges.is_empty() {
            self.record_legacy_backfill_completion(
                legacy_backfill_completion_key(&legacy_ranges),
                durable_cursor,
            )
            .await;
        }
        Ok(AccountWindowPreflight {
            summaries,
            durable_cursor,
            selection,
            legacy_ranges,
            legacy_backfill_required,
            legacy_backfill_complete,
            selection_now,
        })
    }

    async fn legacy_backfill_required(&self, pool: &Pool<Sqlite>) -> Result<bool> {
        if !self.legacy_backfill_state_loaded.load(Ordering::Acquire) {
            let pending = sqlx::query_scalar::<_, String>(
                "SELECT state FROM upstream_account_attribution_backfill_state WHERE id = 1",
            )
            .fetch_optional(pool)
            .await?
            .is_some_and(|state| state == ACCOUNT_WINDOW_LEGACY_BACKFILL_STATE_PENDING);
            self.legacy_backfill_required
                .store(pending, Ordering::Release);
            self.legacy_backfill_state_loaded
                .store(true, Ordering::Release);
        }
        Ok(self.legacy_backfill_required.load(Ordering::Acquire))
    }

    async fn legacy_backfill_complete(
        pool: &Pool<Sqlite>,
        ranges: &[AccountWindowLegacyBackfillRange],
    ) -> Result<bool> {
        if ranges.is_empty() {
            return Ok(true);
        }
        // The global marker only says some legacy attribution remains somewhere in the
        // database. The owner-facing selection is blocked only by a matching active-window
        // row; rolling generations must not become preparing merely because an older row exists.
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT 1 FROM codex_invocations WHERE upstream_account_id IS NULL AND json_valid(payload)",
        );
        push_legacy_backfill_range_predicate(&mut query, ranges);
        query.push(" LIMIT 1");
        Ok(query
            .build_query_scalar::<i64>()
            .fetch_optional(pool)
            .await?
            .is_none())
    }

    async fn record_legacy_backfill_completion(&self, progress_key: String, durable_cursor: i64) {
        let mut completions = self.legacy_backfill_completed_cursors.lock().await;
        if !completions.contains_key(&progress_key)
            && completions.len() >= ACCOUNT_WINDOW_STORAGE_MAX_ENTRIES
            && let Some(oldest_key) = completions.keys().next().cloned()
        {
            completions.remove(&oldest_key);
        }
        completions.insert(progress_key, durable_cursor);
    }

    async fn last_good_response(&self, selection: &str) -> Option<AccountWindowStorageResponse> {
        let compatibility_key = account_window_last_good_compatibility_key(selection)
            .unwrap_or_else(|| account_window_selection_config_from_key(selection).to_string());
        self.entries
            .lock()
            .await
            .iter()
            .filter(|(candidate, _)| {
                account_window_last_good_compatibility_key(candidate)
                    .as_deref()
                    .unwrap_or_else(|| account_window_selection_config_from_key(candidate))
                    == compatibility_key
            })
            .filter_map(|(_, entry)| entry.last_good.as_ref())
            .filter(|last_good| last_good.stored_at.elapsed() <= ACCOUNT_WINDOW_LAST_GOOD_MAX_AGE)
            .max_by_key(|last_good| (last_good.durable_cursor, last_good.stored_at))
            .map(|last_good| AccountWindowStorageResponse::Ready(last_good.response.clone()))
    }

    async fn complete_load(
        &self,
        selection: &str,
        response: AccountWindowStorageResponse,
        refresh_last_good: bool,
        durable_cursor: i64,
        coverage_hole_bucket_count: u64,
        error: Option<String>,
    ) {
        let mut entries = self.entries.lock().await;
        if let Some(entry) = entries.get_mut(selection) {
            if refresh_last_good && let AccountWindowStorageResponse::Ready(payload) = &response {
                entry.last_good = Some(AccountWindowLastGood {
                    response: payload.clone(),
                    stored_at: Instant::now(),
                    durable_cursor,
                });
            }
            entry.coverage_hole_bucket_count = coverage_hole_bucket_count;
            entry.last_error = error;
            let flight = entry.in_flight.take();
            if let Some(flight) = flight {
                flight.complete(response);
            }
        }
    }

    async fn build(
        &self,
        state: &AppState,
        summaries: Vec<UpstreamAccountSummary>,
        account_ids: &[i64],
        durable_cursor: i64,
        now: DateTime<Utc>,
    ) -> Result<AccountWindowBuildResult> {
        let started_at = Instant::now();
        let mut summaries = summaries;
        let (outcome, telemetry) = enrich_window_actual_usage_for_summaries_from_storage_at(
            &state.pool,
            &state.config,
            &mut summaries,
            now,
        )
        .await?;
        {
            let mut health = self
                .health
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            health.rollup_row_count = health
                .rollup_row_count
                .saturating_add(telemetry.rollup_row_count as u64);
            health.minute_row_count = health
                .minute_row_count
                .saturating_add(telemetry.minute_row_count as u64);
            health.bounded_raw_row_count = health
                .bounded_raw_row_count
                .saturating_add(telemetry.bounded_raw_row_count as u64);
        }
        tracing::info!(
            route = "upstream_account_window_usage",
            builder = "account_window_rollup",
            response_source = if outcome == AccountWindowUsageBuildOutcome::Ready { "rollup_boundary_tail" } else { "preparing" },
            selection_fingerprint = %account_window_selection_fingerprint(account_ids),
            durable_cursor,
            rollup_row_count = telemetry.rollup_row_count,
            minute_row_count = telemetry.minute_row_count,
            bounded_raw_row_count = telemetry.bounded_raw_row_count,
            coverage_hole_bucket_count = telemetry.coverage_hole_bucket_count,
            elapsed_ms = started_at.elapsed().as_millis() as u64,
            "upstream account window usage storage-plane build completed"
        );
        if telemetry.live_coverage_repair_required || telemetry.live_cursor_repair_required {
            self.schedule_live_rollup_repair(
                state.pool.clone(),
                state.hourly_rollup_sync_lock.clone(),
                state.shutdown.clone(),
                telemetry.live_coverage_repair_invocation_ids.clone(),
                telemetry.live_cursor_repair_required,
            )
            .await;
        }
        if outcome == AccountWindowUsageBuildOutcome::Preparing {
            if telemetry.archive_coverage_repair_required {
                self.schedule_historical_rollup_repair(state.pool.clone());
            }
            return Ok(AccountWindowBuildResult {
                response: AccountWindowStorageResponse::Preparing {
                    retry_after_ms: ACCOUNT_WINDOW_PREPARING_RETRY_AFTER_MS,
                },
                telemetry,
            });
        }
        if telemetry.archive_coverage_repair_required {
            self.schedule_historical_rollup_repair(state.pool.clone());
        }
        Ok(AccountWindowBuildResult {
            response: AccountWindowStorageResponse::Ready(UpstreamAccountWindowUsageResponse {
                items: summaries
                    .into_iter()
                    .map(|summary| UpstreamAccountWindowUsageItem {
                        account_id: summary.id,
                        primary_actual_usage: summary
                            .primary_window
                            .and_then(|window| window.actual_usage),
                        secondary_actual_usage: summary
                            .secondary_window
                            .and_then(|window| window.actual_usage),
                    })
                    .collect(),
            }),
            telemetry,
        })
    }

    fn schedule_historical_rollup_repair(&self, pool: Pool<Sqlite>) {
        if self
            .coverage_repair_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let running = self.coverage_repair_running.clone();
        tokio::spawn(async move {
            struct CoverageRepairGuard(Arc<AtomicBool>);
            impl Drop for CoverageRepairGuard {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::Release);
                }
            }
            let _running = CoverageRepairGuard(running);
            if let Err(err) = wake_startup_backfill_tasks(
                &pool,
                &[StartupBackfillTask::HistoricalRollups],
                "account_window_usage_coverage_hole",
            )
            .await
            {
                warn!(error = %err, "failed to wake account window coverage repair");
            }
        });
    }

    async fn schedule_live_rollup_repair(
        &self,
        pool: Pool<Sqlite>,
        hourly_rollup_sync_lock: Arc<Mutex<()>>,
        shutdown: CancellationToken,
        invocation_ids: Vec<i64>,
        refresh_live_cursor: bool,
    ) {
        if invocation_ids.is_empty() && !refresh_live_cursor {
            return;
        }
        let should_spawn = {
            let mut repairs = self.live_coverage_repairs.lock().await;
            repairs
                .pending_invocation_ids
                .extend(invocation_ids.into_iter().filter(|id| *id > 0));
            repairs.refresh_live_cursor |= refresh_live_cursor;
            if repairs.worker_running {
                false
            } else {
                repairs.worker_running = true;
                true
            }
        };
        if !should_spawn {
            return;
        }
        let repairs = self.live_coverage_repairs.clone();
        tokio::spawn(async move {
            let mut worker = AccountWindowLiveCoverageRepairWorkerGuard {
                repairs: repairs.clone(),
                completed: false,
            };
            let gate = crate::db_pressure::global_db_pressure_gate();
            loop {
                if shutdown.is_cancelled() {
                    return;
                }
                let observed_eligibility = gate.eligibility_generation();
                match gate.background_deny_reason() {
                    Some(crate::db_pressure::DbPressureDenyReason::PressureCooldown {
                        remaining_ms,
                    }) => {
                        tokio::select! {
                            _ = shutdown.cancelled() => return,
                            _ = tokio::time::sleep(Duration::from_millis(remaining_ms.max(1))) => {}
                            _ = gate.wait_for_eligibility_change(observed_eligibility) => {}
                        }
                        continue;
                    }
                    Some(crate::db_pressure::DbPressureDenyReason::BackgroundBusy) => {
                        tokio::select! {
                            _ = shutdown.cancelled() => return,
                            _ = gate.wait_for_eligibility_change(observed_eligibility) => {}
                        }
                        continue;
                    }
                    None => {}
                }
                let next = {
                    let mut repairs = repairs.lock().await;
                    if repairs.pending_invocation_ids.is_empty()
                        && repairs.pending_full_recompute_ids.is_empty()
                        && !repairs.refresh_live_cursor
                    {
                        repairs.worker_running = false;
                        None
                    } else {
                        let full_bucket_recompute = !repairs.pending_full_recompute_ids.is_empty();
                        let batch_size = if full_bucket_recompute {
                            ACCOUNT_WINDOW_LIVE_COVERAGE_FULL_REPAIR_BATCH_SIZE
                        } else {
                            ACCOUNT_WINDOW_LIVE_COVERAGE_REPAIR_BATCH_SIZE
                        };
                        let queue = if full_bucket_recompute {
                            &mut repairs.pending_full_recompute_ids
                        } else {
                            &mut repairs.pending_invocation_ids
                        };
                        let invocation_ids =
                            queue.iter().take(batch_size).copied().collect::<Vec<_>>();
                        for invocation_id in &invocation_ids {
                            queue.remove(invocation_id);
                        }
                        let refresh_live_cursor = std::mem::take(&mut repairs.refresh_live_cursor);
                        let batch = AccountWindowLiveCoverageRepairBatch {
                            invocation_ids,
                            refresh_live_cursor,
                            full_bucket_recompute,
                        };
                        repairs.in_flight = Some(batch.clone());
                        Some(batch)
                    }
                };
                let Some(batch) = next else {
                    // A fresh worker may start immediately after this flag is cleared. Mark
                    // this guard complete so its Drop implementation cannot reset that newer
                    // worker's ownership bit.
                    worker.complete();
                    return;
                };
                // Never hold the global background slot while waiting on the single SQLite
                // writer. P1 may otherwise be denied solely by an idle repair waiter.
                let Some(mut write_permit) =
                    crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
                        .acquire_maintenance_cancellable(
                            ACCOUNT_WINDOW_LIVE_COVERAGE_REPAIR_FAIRNESS_INTERVAL,
                            &shutdown,
                        )
                        .await
                else {
                    return;
                };
                let permit = match gate.try_begin_background("account_window_usage_live_repair") {
                    Ok(permit) => permit,
                    Err(deny_reason) => {
                        {
                            let mut queue = repairs.lock().await;
                            if let Some(batch) = queue.in_flight.take() {
                                requeue_live_coverage_repair_batch(&mut queue, batch);
                            }
                        }
                        write_permit.revoke_fairness_admission();
                        drop(write_permit);
                        match deny_reason {
                            crate::db_pressure::DbPressureDenyReason::PressureCooldown {
                                remaining_ms,
                            } => {
                                tokio::select! {
                                    _ = shutdown.cancelled() => return,
                                    _ = tokio::time::sleep(Duration::from_millis(remaining_ms.max(1))) => {}
                                    _ = gate.wait_for_eligibility_change(observed_eligibility) => {}
                                }
                            }
                            crate::db_pressure::DbPressureDenyReason::BackgroundBusy => {
                                tokio::select! {
                                    _ = shutdown.cancelled() => return,
                                    _ = gate.wait_for_eligibility_change(observed_eligibility) => {}
                                }
                            }
                        }
                        continue;
                    }
                };
                let _guard = tokio::select! {
                    _ = shutdown.cancelled() => return,
                    guard = hourly_rollup_sync_lock.lock() => guard,
                };
                let started_at = Instant::now();
                let result = match tokio::time::timeout(
                    ACCOUNT_WINDOW_LIVE_COVERAGE_REPAIR_BUDGET,
                    async {
                        if batch.refresh_live_cursor {
                            // One replay batch advances the durable cursor without turning an
                            // owner retry into the historical catch-up loop.
                            crate::maintenance::replay_live_invocation_hourly_rollups(&pool)
                                .await?;
                        }
                        if batch.invocation_ids.is_empty() {
                            return Ok(
                                crate::maintenance::InvocationHourlyRollupRecomputeOutcome::Recomputed,
                            );
                        }
                        let mut tx = pool.begin().await?;
                        let outcome = crate::maintenance::recompute_invocation_hourly_rollups_for_ids_with_row_limit_tx(
                            tx.as_mut(),
                            &batch.invocation_ids,
                            (!batch.full_bucket_recompute)
                                .then_some(ACCOUNT_WINDOW_LIVE_COVERAGE_REPAIR_MAX_ROWS),
                        )
                        .await?;
                        tx.commit().await?;
                        Ok(outcome)
                    },
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(anyhow::anyhow!(
                        "account window live coverage repair exceeded {}ms budget",
                        ACCOUNT_WINDOW_LIVE_COVERAGE_REPAIR_BUDGET.as_millis()
                    )),
                };
                drop(_guard);
                drop(write_permit);
                drop(permit);
                match result {
                    Ok(crate::maintenance::InvocationHourlyRollupRecomputeOutcome::Recomputed) => {
                        repairs.lock().await.in_flight = None;
                        tracing::debug!(
                            repaired_bucket_limit = batch.invocation_ids.len(),
                            repair_mode = if batch.full_bucket_recompute { "full_bucket" } else { "bounded" },
                            cursor_replay = batch.refresh_live_cursor,
                            elapsed_ms = started_at.elapsed().as_millis() as u64,
                            "account window live coverage repair completed"
                        );
                    }
                    Ok(crate::maintenance::InvocationHourlyRollupRecomputeOutcome::RowLimitExceeded) => {
                        warn!(
                            repaired_bucket_limit = batch.invocation_ids.len(),
                            repair_row_limit = ACCOUNT_WINDOW_LIVE_COVERAGE_REPAIR_MAX_ROWS,
                            repair_mode = "bounded",
                            cursor_replay = batch.refresh_live_cursor,
                            elapsed_ms = started_at.elapsed().as_millis() as u64,
                            "account window live coverage repair exceeded its bounded row budget; retaining a full-bucket repair"
                        );
                        let mut queue = repairs.lock().await;
                        if let Some(batch) = queue.in_flight.take() {
                            requeue_live_coverage_repair_batch(&mut queue, AccountWindowLiveCoverageRepairBatch {
                                full_bucket_recompute: true,
                                ..batch
                            });
                        }
                    }
                    Err(err) => {
                        let under_pressure = gate.record_error("account_window_usage_live_repair", &err);
                        warn!(
                            error = %err,
                            repaired_bucket_limit = batch.invocation_ids.len(),
                            repair_mode = if batch.full_bucket_recompute { "full_bucket" } else { "bounded" },
                            cursor_replay = batch.refresh_live_cursor,
                            elapsed_ms = started_at.elapsed().as_millis() as u64,
                            "account window live coverage repair failed; retaining queued work"
                        );
                        {
                            let mut queue = repairs.lock().await;
                            if let Some(batch) = queue.in_flight.take() {
                                requeue_live_coverage_repair_batch(&mut queue, batch);
                            }
                        }
                        if !under_pressure {
                            if let Err(wake_err) = wake_startup_backfill_tasks(
                                &pool,
                                &[StartupBackfillTask::HistoricalRollups],
                                "account_window_usage_live_repair_error",
                            )
                            .await
                            {
                                warn!(error = %wake_err, "failed to wake retained account window coverage repair");
                            }
                            // Keep ownership of the merged queue so a concurrent request cannot
                            // observe a running worker and then lose the wake-up. Deterministic
                            // failures retry slowly while the durable historical repair is woken.
                            tokio::time::sleep(Duration::from_secs(5)).await;
                        }
                    }
                }
            }
        });
    }

    fn schedule_legacy_backfill(
        &self,
        pool: Pool<Sqlite>,
        selection: String,
        ranges: Vec<AccountWindowLegacyBackfillRange>,
    ) {
        if self
            .backfill_running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let running = self.backfill_running.clone();
        let health = self.health.clone();
        let entries = self.entries.clone();
        tokio::spawn(async move {
            struct BackfillGuard(Arc<AtomicBool>);
            impl Drop for BackfillGuard {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::Release);
                }
            }
            let _running = BackfillGuard(running);
            let outcome = Self::run_legacy_backfill_pass(pool, ranges, health).await;
            let mut entries = entries.lock().await;
            let Some(entry) = entries.get_mut(&selection) else {
                return;
            };
            match outcome {
                Ok(AccountWindowLegacyBackfillOutcome::Ready) => {
                    // The next owner preflight verifies the durable cursor before accepting
                    // completion. A background pass cannot safely publish a completion cache
                    // because a new terminal may arrive after its final range scan.
                    entry.legacy_readiness = AccountWindowLegacyReadiness::Ready;
                    entry.last_error = None;
                }
                Ok(AccountWindowLegacyBackfillOutcome::Pending) => {
                    // The next owner retry reschedules the bounded pass. Do not spin in a
                    // background task while the pressure gate is closed or work remains.
                    entry.legacy_readiness = AccountWindowLegacyReadiness::Unknown;
                }
                Err(err) => {
                    entry.legacy_readiness = AccountWindowLegacyReadiness::Unknown;
                    entry.last_error = Some(err.to_string());
                    warn!(error = %err, "account window legacy backfill pass failed");
                }
            }
        });
    }

    async fn run_legacy_backfill_pass(
        pool: Pool<Sqlite>,
        ranges: Vec<AccountWindowLegacyBackfillRange>,
        health: Arc<std::sync::Mutex<AccountWindowStorageHealthState>>,
    ) -> Result<AccountWindowLegacyBackfillOutcome> {
        if ranges.is_empty() {
            return Ok(AccountWindowLegacyBackfillOutcome::Ready);
        }
        if global_db_pressure_gate().background_deny_reason().is_some() {
            return Ok(AccountWindowLegacyBackfillOutcome::Pending);
        }
        let progress_key = legacy_backfill_progress_key(&ranges);
        let progress = load_startup_backfill_progress(&pool, &progress_key).await?;
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT id, payload FROM codex_invocations WHERE upstream_account_id IS NULL AND json_valid(payload) AND id > ",
        );
        query.push_bind(progress.cursor_id);
        push_legacy_backfill_range_predicate(&mut query, &ranges);
        query
            .push(" ORDER BY id ASC LIMIT ")
            .push_bind(ACCOUNT_WINDOW_LEGACY_BACKFILL_WRITE_BATCH_SIZE);
        let rows = query
            .build_query_as::<(i64, Option<String>)>()
            .fetch_all(&pool)
            .await?;
        if rows.is_empty() {
            let write_permit =
                crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
                    .acquire_maintenance(ACCOUNT_WINDOW_LEGACY_BACKFILL_FAIRNESS_INTERVAL)
                    .await;
            let Ok(_pressure_permit) =
                global_db_pressure_gate().try_begin_background("account_window_usage_backfill")
            else {
                drop(write_permit);
                return Ok(AccountWindowLegacyBackfillOutcome::Pending);
            };
            save_startup_backfill_progress(
                &pool,
                &progress_key,
                StartupBackfillProgressUpdate {
                    cursor_id: progress.cursor_id,
                    scanned: 0,
                    updated: 0,
                    zero_update_streak: progress.zero_update_streak.saturating_add(1),
                    next_run_after: &Utc::now().to_rfc3339(),
                    status: "complete",
                    suspension_reason: None,
                },
            )
            .await?;
            health
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .backfill_cursor = progress.cursor_id;
            return Ok(AccountWindowLegacyBackfillOutcome::Ready);
        }

        let write_permit = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .acquire_maintenance(ACCOUNT_WINDOW_LEGACY_BACKFILL_FAIRNESS_INTERVAL)
            .await;
        let Ok(_pressure_permit) =
            global_db_pressure_gate().try_begin_background("account_window_usage_backfill")
        else {
            drop(write_permit);
            return Ok(AccountWindowLegacyBackfillOutcome::Pending);
        };
        let started_at = Instant::now();
        let mut tx = pool.begin().await?;
        let mut last_scanned_id = progress.cursor_id;
        let mut updated = 0_u64;
        let mut scanned = 0_u64;
        let mut updated_ids = Vec::new();
        for (id, payload) in &rows {
            if started_at.elapsed() >= ACCOUNT_WINDOW_LEGACY_BACKFILL_BUDGET {
                break;
            }
            last_scanned_id = *id;
            scanned = scanned.saturating_add(1);
            let Some(account_id) =
                crate::proxy::upstream_account_id_from_payload(payload.as_deref())
            else {
                continue;
            };
            let result = sqlx::query(
                "UPDATE codex_invocations SET upstream_account_id = ?2 WHERE id = ?1 AND upstream_account_id IS NULL",
            )
            .bind(id)
            .bind(account_id)
            .execute(&mut *tx)
            .await?;
            updated += result.rows_affected();
            if result.rows_affected() > 0 {
                updated_ids.push(*id);
            }
        }

        // Keep the structured assignment and every rollup that can be rebuilt within the same
        // small transaction atomic. Dense hours are left as an explicit coverage hole: their
        // assignment still commits, while the owner path stays preparing and queues the bounded
        // live repair instead of widening this maintenance transaction.
        let rollup_outcome =
            crate::maintenance::recompute_invocation_hourly_rollups_for_ids_with_row_limit_tx(
                tx.as_mut(),
                &updated_ids,
                Some(ACCOUNT_WINDOW_LEGACY_BACKFILL_ROLLUP_MAX_ROWS),
            )
            .await?;
        if rollup_outcome
            == crate::maintenance::InvocationHourlyRollupRecomputeOutcome::RowLimitExceeded
        {
            tracing::warn!(
                updated_invocation_count = updated_ids.len(),
                rollup_row_limit = ACCOUNT_WINDOW_LEGACY_BACKFILL_ROLLUP_MAX_ROWS,
                write_budget_ms = ACCOUNT_WINDOW_LEGACY_BACKFILL_BUDGET.as_millis() as u64,
                "account window attribution backfill committed a bounded coverage hole for live repair"
            );
        }
        tx.commit().await?;

        if started_at.elapsed() > ACCOUNT_WINDOW_LEGACY_BACKFILL_BUDGET {
            tracing::warn!(
                write_budget_ms = ACCOUNT_WINDOW_LEGACY_BACKFILL_BUDGET.as_millis() as u64,
                candidate_count = rows.len(),
                scanned_count = scanned,
                updated_count = updated,
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                "account window attribution backfill exceeded its write-budget target"
            );
        }

        let cursor_id = last_scanned_id;

        let outcome = if scanned == rows.len() as u64
            && rows.len() < ACCOUNT_WINDOW_LEGACY_BACKFILL_WRITE_BATCH_SIZE as usize
        {
            AccountWindowLegacyBackfillOutcome::Ready
        } else {
            AccountWindowLegacyBackfillOutcome::Pending
        };
        let next_run_after = Utc::now().to_rfc3339();
        save_startup_backfill_progress(
            &pool,
            &progress_key,
            StartupBackfillProgressUpdate {
                cursor_id,
                scanned,
                updated,
                zero_update_streak: if updated == 0 {
                    progress.zero_update_streak.saturating_add(1)
                } else {
                    0
                },
                next_run_after: &next_run_after,
                status: match outcome {
                    AccountWindowLegacyBackfillOutcome::Ready => "complete",
                    AccountWindowLegacyBackfillOutcome::Pending => "pending",
                },
                suspension_reason: None,
            },
        )
        .await?;
        health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .backfill_cursor = cursor_id;
        Ok(outcome)
    }

    #[cfg(test)]
    pub(crate) async fn run_legacy_backfill_pass_for_test(
        pool: Pool<Sqlite>,
        account_ids: Vec<i64>,
        active_window_start: DateTime<Utc>,
    ) -> Result<()> {
        let ranges = account_ids
            .into_iter()
            .map(|account_id| AccountWindowLegacyBackfillRange {
                account_id,
                start_at: format_naive(active_window_start.with_timezone(&Shanghai).naive_local()),
                end_at: format_naive(
                    (Utc::now() + ChronoDuration::minutes(1))
                        .with_timezone(&Shanghai)
                        .naive_local(),
                ),
                selection_generation: format!("test:{account_id}"),
                window_duration_secs: 60,
            })
            .collect();
        Self::run_legacy_backfill_pass(
            pool,
            ranges,
            Arc::new(std::sync::Mutex::new(
                AccountWindowStorageHealthState::default(),
            )),
        )
        .await
        .map(|_| ())
    }

    pub(crate) async fn health_snapshot(&self) -> AccountWindowStoragePlaneHealthSnapshot {
        let entries = self.entries.lock().await;
        let health = self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let active_entries = entries
            .values()
            .filter(|entry| {
                entry.in_flight.is_some()
                    || entry.last_used_at.elapsed() <= ACCOUNT_WINDOW_STORAGE_IDLE_TTL
            })
            .collect::<Vec<_>>();
        let in_flight_build_count = active_entries
            .iter()
            .filter(|entry| entry.in_flight.is_some())
            .count();
        let has_deferred_selection = active_entries.iter().any(|entry| {
            entry.in_flight.is_some()
                || entry.legacy_readiness != AccountWindowLegacyReadiness::Ready
                || entry.last_good.as_ref().is_some_and(|last_good| {
                    last_good.stored_at.elapsed() > ACCOUNT_WINDOW_LAST_GOOD_MAX_AGE
                })
        });
        let coverage_hole_bucket_count = active_entries
            .iter()
            .map(|entry| entry.coverage_hole_bucket_count)
            .sum();
        let last_good_age_ms = active_entries
            .iter()
            .filter_map(|entry| entry.last_good.as_ref())
            .map(|last_good| last_good.stored_at.elapsed().as_millis() as u64)
            .max();
        let last_error = active_entries
            .iter()
            .filter_map(|entry| entry.last_error.clone())
            .next();
        let direct_pool_violation_count = self.direct_pool_violation_count.load(Ordering::Relaxed);
        let state = if last_error.is_some() || direct_pool_violation_count > 0 {
            "degraded"
        } else if has_deferred_selection
            || in_flight_build_count > 0
            || coverage_hole_bucket_count > 0
            || last_good_age_ms
                .is_some_and(|age| age > ACCOUNT_WINDOW_LAST_GOOD_MAX_AGE.as_millis() as u64)
        {
            "deferred"
        } else {
            "healthy"
        };
        AccountWindowStoragePlaneHealthSnapshot {
            state: state.to_string(),
            active_selection_count: active_entries.len(),
            in_flight_build_count,
            coalesced_waiter_count: self.coalesced_waiter_count.load(Ordering::Relaxed),
            rollup_row_count: health.rollup_row_count,
            minute_row_count: health.minute_row_count,
            bounded_raw_row_count: health.bounded_raw_row_count,
            coverage_hole_bucket_count,
            backfill_cursor: health.backfill_cursor,
            last_good_age_ms,
            direct_pool_violation_count,
            last_error,
        }
    }

    #[cfg(test)]
    pub(crate) fn record_direct_pool_violation_for_test(&self) {
        self.direct_pool_violation_count
            .fetch_add(1, Ordering::Relaxed);
    }

    fn prune_entries(entries: &mut HashMap<String, AccountWindowSelectionEntry>) {
        let now = Instant::now();
        entries.retain(|_, entry| {
            entry.in_flight.is_some()
                || now.duration_since(entry.last_used_at) <= ACCOUNT_WINDOW_STORAGE_IDLE_TTL
        });
        if entries.len() <= ACCOUNT_WINDOW_STORAGE_MAX_ENTRIES {
            return;
        }
        let mut keys = entries
            .iter()
            .filter(|(_, entry)| entry.in_flight.is_none())
            .map(|(key, entry)| (key.clone(), entry.last_used_at))
            .collect::<Vec<_>>();
        keys.sort_by_key(|(_, last_used_at)| *last_used_at);
        for (key, _) in keys.into_iter().take(
            entries
                .len()
                .saturating_sub(ACCOUNT_WINDOW_STORAGE_MAX_ENTRIES),
        ) {
            entries.remove(&key);
        }
    }

    fn make_room_for_selection(
        entries: &mut HashMap<String, AccountWindowSelectionEntry>,
        selection: &str,
    ) {
        if entries.contains_key(selection) || entries.len() < ACCOUNT_WINDOW_STORAGE_MAX_ENTRIES {
            return;
        }
        if let Some(key) = entries
            .iter()
            .filter(|(_, entry)| entry.in_flight.is_none())
            .min_by_key(|(_, entry)| entry.last_used_at)
            .map(|(key, _)| key.clone())
        {
            entries.remove(&key);
        }
    }
}

fn legacy_backfill_progress_key(ranges: &[AccountWindowLegacyBackfillRange]) -> String {
    let mut normalized_ranges = ranges
        .iter()
        .map(|range| {
            let reset_generation =
                if let Some(anchor) = range.selection_generation.strip_prefix("reset:") {
                    format!("reset:{anchor}")
                } else if let Some(anchor) = range
                    .selection_generation
                    .strip_prefix("reset-pending:")
                    .and_then(|value| value.split(":minute:").next())
                {
                    format!("reset-pending:{anchor}")
                } else {
                    "rolling".to_string()
                };
            (
                range.account_id,
                range.window_duration_secs,
                reset_generation,
            )
        })
        .collect::<Vec<_>>();
    normalized_ranges.sort_unstable();
    normalized_ranges.dedup();
    format!(
        "{ACCOUNT_WINDOW_LEGACY_BACKFILL_PROGRESS_KEY}:{}",
        normalized_ranges
            .iter()
            .map(|(account_id, duration, reset_generation)| {
                format!("{account_id}:{duration}:{reset_generation}")
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

// Cursor progress for rolling windows can be shared across minute boundaries, but a readiness
// proof must remain pinned to the exact selection. An older legacy row can enter the active
// window without changing the durable cursor.
fn legacy_backfill_completion_key(ranges: &[AccountWindowLegacyBackfillRange]) -> String {
    let mut normalized_ranges = ranges
        .iter()
        .map(|range| {
            (
                range.account_id,
                range.start_at.as_str(),
                range.end_at.as_str(),
                range.selection_generation.as_str(),
                range.window_duration_secs,
            )
        })
        .collect::<Vec<_>>();
    normalized_ranges.sort_unstable();
    normalized_ranges.dedup();
    format!(
        "{ACCOUNT_WINDOW_LEGACY_BACKFILL_PROGRESS_KEY}:completion:{}",
        normalized_ranges
            .iter()
            .map(|(account_id, start_at, end_at, generation, duration)| {
                format!("{account_id}:{start_at}:{end_at}:{generation}:{duration}")
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn legacy_backfill_ranges_from_plans(
    plans: &HashMap<i64, AccountWindowUsagePlan>,
) -> Vec<AccountWindowLegacyBackfillRange> {
    let mut ranges = plans
        .iter()
        .flat_map(|(account_id, plan)| {
            [plan.primary.as_ref(), plan.secondary.as_ref()]
                .into_iter()
                .flatten()
                .map(move |range| AccountWindowLegacyBackfillRange {
                    account_id: *account_id,
                    start_at: range.start_at.clone(),
                    end_at: range.end_at.clone(),
                    selection_generation: range.selection_generation.clone(),
                    window_duration_secs: range.window_duration_secs,
                })
        })
        .collect::<Vec<_>>();
    ranges.sort_by(|left, right| {
        (left.account_id, &left.start_at, &left.end_at).cmp(&(
            right.account_id,
            &right.start_at,
            &right.end_at,
        ))
    });
    ranges.dedup_by(|left, right| {
        left.account_id == right.account_id
            && left.selection_generation == right.selection_generation
            && left.window_duration_secs == right.window_duration_secs
    });
    ranges
}

fn push_legacy_backfill_range_predicate<'a>(
    query: &mut QueryBuilder<'a, Sqlite>,
    ranges: &'a [AccountWindowLegacyBackfillRange],
) {
    query.push(" AND json_type(payload, '$.upstreamAccountId') IN ('integer', 'text') AND (");
    for (index, range) in ranges.iter().enumerate() {
        if index > 0 {
            query.push(" OR ");
        }
        query
            .push("(CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) = ")
            .push_bind(range.account_id)
            .push(" AND occurred_at >= ")
            .push_bind(&range.start_at)
            .push(" AND occurred_at < ")
            .push_bind(&range.end_at)
            .push(")");
    }
    query.push(")");
}

fn account_window_selection_config_key(
    account_ids: &[i64],
    plans: &HashMap<i64, AccountWindowUsagePlan>,
) -> String {
    let ranges = legacy_backfill_ranges_from_plans(plans);
    format!(
        "accounts={};windows={}",
        account_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(","),
        ranges
            .iter()
            .map(|range| {
                format!(
                    "{}:{}:{}",
                    range.account_id, range.selection_generation, range.window_duration_secs
                )
            })
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn account_window_preflight_key(account_ids: &[i64]) -> String {
    format!(
        "accounts={}",
        account_ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",")
    )
}

async fn durable_cursor_for_account_selection(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<i64> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT COALESCE(MAX(id), 0) FROM codex_invocations WHERE upstream_account_id IN (",
    );
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    query.push(")");
    Ok(query.build_query_scalar::<i64>().fetch_one(pool).await?)
}

fn account_window_selection_key(selection_config: &str, durable_cursor: i64) -> String {
    format!("{selection_config};cursor={durable_cursor}")
}

fn account_window_selection_config_from_key(selection: &str) -> &str {
    selection
        .rsplit_once(";cursor=")
        .map_or(selection, |(config, _)| config)
}

// A moving window advances its exact build key every minute. Last-good is explicitly allowed
// to be up to 60 seconds old, so retain it across that minute generation while still requiring
// the same account set, window duration, and reset anchor.
fn account_window_last_good_compatibility_key(selection: &str) -> Option<String> {
    let selection_config = account_window_selection_config_from_key(selection);
    let (accounts, windows) = selection_config.split_once(";windows=")?;
    let normalized_windows = windows
        .split(',')
        .map(|window| {
            let (prefix, duration) = window.rsplit_once(':')?;
            let (account_id, generation) = prefix.split_once(':')?;
            let normalized_generation = if generation.starts_with("minute:") {
                "minute".to_string()
            } else if let Some((reset_anchor, _)) = generation.rsplit_once(":minute:") {
                if reset_anchor.starts_with("reset-pending:") {
                    reset_anchor.to_string()
                } else {
                    generation.to_string()
                }
            } else {
                generation.to_string()
            };
            Some(format!("{account_id}:{normalized_generation}:{duration}"))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(format!(
        "{accounts};windows={}",
        normalized_windows.join(",")
    ))
}

fn account_window_selection_fingerprint(account_ids: &[i64]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut normalized_account_ids = account_ids.to_vec();
    normalized_account_ids.sort_unstable();
    normalized_account_ids.dedup();
    normalized_account_ids.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn can_admit_account_window_selection(
    entries: &HashMap<String, AccountWindowSelectionEntry>,
    selection: &str,
) -> bool {
    entries.contains_key(selection)
        || entries.len() < ACCOUNT_WINDOW_STORAGE_MAX_ENTRIES
        || entries.values().any(|entry| entry.in_flight.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn completed_singleflight_does_not_extend_last_good_lifetime() {
        let storage = AccountWindowStoragePlane::default();
        let selection = "accounts=42";
        let response = UpstreamAccountWindowUsageResponse { items: Vec::new() };
        {
            let mut entries = storage.entries.lock().await;
            let entry = entries.entry(selection.to_string()).or_default();
            entry.in_flight = Some(Arc::new(AccountWindowLoadFlight::new()));
            entry.last_good = Some(AccountWindowLastGood {
                response: response.clone(),
                stored_at: Instant::now()
                    - ACCOUNT_WINDOW_LAST_GOOD_MAX_AGE
                    - Duration::from_millis(1),
                durable_cursor: 1,
            });
        }

        storage
            .complete_load(
                selection,
                AccountWindowStorageResponse::Ready(response),
                false,
                1,
                0,
                None,
            )
            .await;

        assert!(storage.last_good_response(selection).await.is_none());
    }

    #[tokio::test]
    async fn last_good_prefers_the_newest_durable_cursor_over_completion_order() {
        let storage = AccountWindowStoragePlane::default();
        let older_selection = "accounts=42;windows=42:minute:100:300;cursor=10";
        let newer_selection = "accounts=42;windows=42:minute:101:300;cursor=11";
        let older = UpstreamAccountWindowUsageResponse { items: Vec::new() };
        let newer = UpstreamAccountWindowUsageResponse {
            items: vec![UpstreamAccountWindowUsageItem {
                account_id: 42,
                primary_actual_usage: None,
                secondary_actual_usage: None,
            }],
        };
        let mut entries = storage.entries.lock().await;
        entries
            .entry(older_selection.to_string())
            .or_default()
            .last_good = Some(AccountWindowLastGood {
            response: older,
            stored_at: Instant::now(),
            durable_cursor: 10,
        });
        entries
            .entry(newer_selection.to_string())
            .or_default()
            .last_good = Some(AccountWindowLastGood {
            response: newer.clone(),
            stored_at: Instant::now() - Duration::from_secs(1),
            durable_cursor: 11,
        });
        drop(entries);

        let Some(AccountWindowStorageResponse::Ready(response)) =
            storage.last_good_response(older_selection).await
        else {
            panic!("expected the durable-cursor newest last-good response");
        };
        assert_eq!(response.items.len(), newer.items.len());
        assert_eq!(response.items[0].account_id, 42);
    }

    #[test]
    fn last_good_compatibility_keeps_rolling_generation_but_not_reset_anchor() {
        let first = "accounts=42;windows=42:minute:100:300;cursor=1";
        let next = "accounts=42;windows=42:minute:101:300;cursor=2";
        let reset_changed = "accounts=42;windows=42:reset:101:300;cursor=2";

        assert_eq!(
            account_window_last_good_compatibility_key(first),
            account_window_last_good_compatibility_key(next)
        );
        assert_ne!(
            account_window_last_good_compatibility_key(first),
            account_window_last_good_compatibility_key(reset_changed)
        );
    }

    #[tokio::test]
    async fn abandoned_leader_releases_the_selection_for_waiters() {
        let storage = AccountWindowStoragePlane::default();
        let selection = "accounts=42".to_string();
        let (lease, mut waiter) = {
            let mut entries = storage.entries.lock().await;
            let entry = entries.entry(selection.clone()).or_default();
            let flight = Arc::new(AccountWindowLoadFlight::new());
            entry.in_flight = Some(flight.clone());
            let waiter = flight.subscribe();
            (
                AccountWindowLoadLease {
                    entries: storage.entries.clone(),
                    selection: selection.clone(),
                    flight,
                    completed: false,
                },
                waiter,
            )
        };

        drop(lease);
        let _ = tokio::time::timeout(Duration::from_millis(100), waiter.changed())
            .await
            .expect("abandoned leader should notify waiting requests");
        assert!(matches!(
            waiter.borrow_and_update().clone(),
            Some(AccountWindowStorageResponse::Preparing { .. })
        ));

        let entries = storage.entries.lock().await;
        let entry = entries
            .get(&selection)
            .expect("selection entry remains available");
        assert!(entry.in_flight.is_none());
    }

    #[tokio::test]
    async fn cancelled_live_coverage_worker_requeues_its_in_flight_batch() {
        let repairs = Arc::new(Mutex::new(AccountWindowLiveCoverageRepairQueue {
            in_flight: Some(AccountWindowLiveCoverageRepairBatch {
                invocation_ids: vec![41, 42],
                refresh_live_cursor: true,
                full_bucket_recompute: false,
            }),
            worker_running: true,
            ..Default::default()
        }));
        {
            let _worker = AccountWindowLiveCoverageRepairWorkerGuard {
                repairs: repairs.clone(),
                completed: false,
            };
        }
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let queue = repairs.lock().await;
                if !queue.worker_running {
                    assert!(queue.in_flight.is_none());
                    assert_eq!(
                        queue.pending_invocation_ids,
                        BTreeSet::from([41_i64, 42_i64])
                    );
                    assert!(queue.refresh_live_cursor);
                    break;
                }
                drop(queue);
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("worker drop should restore its in-flight repair batch");
    }

    #[tokio::test]
    async fn completed_live_coverage_worker_does_not_clear_a_new_worker_flag() {
        let repairs = Arc::new(Mutex::new(AccountWindowLiveCoverageRepairQueue {
            worker_running: true,
            ..Default::default()
        }));
        {
            let mut worker = AccountWindowLiveCoverageRepairWorkerGuard {
                repairs: repairs.clone(),
                completed: false,
            };
            worker.complete();
        }
        tokio::task::yield_now().await;
        assert!(repairs.lock().await.worker_running);
    }

    #[test]
    fn account_window_preflight_key_only_depends_on_normalized_account_selection() {
        assert_eq!(
            account_window_preflight_key(&[3, 7, 11]),
            account_window_preflight_key(&[3, 7, 11])
        );
        assert_ne!(
            account_window_preflight_key(&[3, 7, 11]),
            account_window_preflight_key(&[3, 7])
        );
    }

    #[tokio::test]
    async fn account_window_durable_cursor_ignores_unrelated_accounts() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("create cursor fixture pool");
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, upstream_account_id INTEGER)",
        )
        .execute(&pool)
        .await
        .expect("create invocation fixture");
        sqlx::query(
            "INSERT INTO codex_invocations (id, upstream_account_id) VALUES (7, 42), (99, 77)",
        )
        .execute(&pool)
        .await
        .expect("seed invocation fixture");

        assert_eq!(
            durable_cursor_for_account_selection(&pool, &[42])
                .await
                .expect("load selected account cursor"),
            7
        );
        assert_eq!(
            durable_cursor_for_account_selection(&pool, &[77])
                .await
                .expect("load unrelated account cursor"),
            99
        );
    }

    #[tokio::test]
    async fn direct_pool_violation_degrades_storage_plane_health() {
        let storage = AccountWindowStoragePlane::default();
        assert_eq!(storage.health_snapshot().await.state, "healthy");

        storage.record_direct_pool_violation_for_test();

        let health = storage.health_snapshot().await;
        assert_eq!(health.state, "degraded");
        assert_eq!(health.direct_pool_violation_count, 1);
    }

    #[test]
    fn full_in_flight_selection_set_rejects_new_entries() {
        let mut entries = HashMap::new();
        for index in 0..ACCOUNT_WINDOW_STORAGE_MAX_ENTRIES {
            entries.insert(
                format!("accounts={index}"),
                AccountWindowSelectionEntry {
                    in_flight: Some(Arc::new(AccountWindowLoadFlight::new())),
                    ..Default::default()
                },
            );
        }

        assert!(!can_admit_account_window_selection(
            &entries,
            "accounts=next"
        ));
        assert!(can_admit_account_window_selection(&entries, "accounts=0"));
    }

    #[test]
    fn legacy_backfill_progress_identity_changes_with_duration_or_reset_generation() {
        let short_window = vec![AccountWindowLegacyBackfillRange {
            account_id: 42,
            start_at: "2026-08-16 10:00:00".to_string(),
            end_at: "2026-08-16 15:00:00".to_string(),
            selection_generation: "reset:123".to_string(),
            window_duration_secs: 5 * 60 * 60,
        }];
        let expanded_window = vec![AccountWindowLegacyBackfillRange {
            account_id: 42,
            start_at: "2026-08-16 08:00:00".to_string(),
            end_at: "2026-08-16 15:00:00".to_string(),
            selection_generation: "reset:124".to_string(),
            window_duration_secs: 7 * 60 * 60,
        }];

        assert_ne!(
            legacy_backfill_progress_key(&short_window),
            legacy_backfill_progress_key(&expanded_window)
        );
    }

    #[test]
    fn legacy_backfill_progress_identity_keeps_a_shared_moving_cursor() {
        let closed_reset = AccountWindowLegacyBackfillRange {
            account_id: 42,
            start_at: "2026-08-16 10:00:00".to_string(),
            end_at: "2026-08-16 15:00:00".to_string(),
            selection_generation: "reset:123".to_string(),
            window_duration_secs: 5 * 60 * 60,
        };
        let rolling = AccountWindowLegacyBackfillRange {
            selection_generation: "minute:456".to_string(),
            ..closed_reset.clone()
        };
        let pending_reset = AccountWindowLegacyBackfillRange {
            selection_generation: "reset-pending:123:minute:456".to_string(),
            ..closed_reset.clone()
        };

        assert_ne!(
            legacy_backfill_progress_key(std::slice::from_ref(&rolling)),
            legacy_backfill_progress_key(std::slice::from_ref(&pending_reset)),
            "a future reset must not advance the rolling cursor past older rows"
        );
    }

    #[test]
    fn legacy_completion_identity_is_pinned_to_the_exact_rolling_window() {
        let rolling = AccountWindowLegacyBackfillRange {
            account_id: 42,
            start_at: "2026-08-16 10:00:00".to_string(),
            end_at: "2026-08-16 15:00:00".to_string(),
            selection_generation: "minute:456".to_string(),
            window_duration_secs: 5 * 60 * 60,
        };
        let next_minute = AccountWindowLegacyBackfillRange {
            start_at: "2026-08-16 10:01:00".to_string(),
            end_at: "2026-08-16 15:01:00".to_string(),
            selection_generation: "minute:457".to_string(),
            ..rolling.clone()
        };

        assert_eq!(
            legacy_backfill_progress_key(std::slice::from_ref(&rolling)),
            legacy_backfill_progress_key(std::slice::from_ref(&next_minute)),
            "the durable cursor remains reusable for the same rolling duration"
        );
        assert_ne!(
            legacy_backfill_completion_key(std::slice::from_ref(&rolling)),
            legacy_backfill_completion_key(std::slice::from_ref(&next_minute)),
            "an exact completion proof must not hide legacy rows entering the next window"
        );
    }

    #[tokio::test]
    async fn legacy_completion_cache_is_bounded_and_cursor_aware() {
        let storage = AccountWindowStoragePlane::default();
        storage
            .record_legacy_backfill_completion("active-selection".to_string(), 41)
            .await;
        assert_eq!(
            storage
                .legacy_backfill_completed_cursors
                .lock()
                .await
                .get("active-selection"),
            Some(&41)
        );

        storage
            .record_legacy_backfill_completion("active-selection".to_string(), 42)
            .await;
        assert_eq!(
            storage
                .legacy_backfill_completed_cursors
                .lock()
                .await
                .get("active-selection"),
            Some(&42),
            "a later durable cursor must invalidate an older completion proof"
        );

        for index in 0..=ACCOUNT_WINDOW_STORAGE_MAX_ENTRIES {
            storage
                .record_legacy_backfill_completion(format!("selection-{index}"), index as i64)
                .await;
        }
        assert!(
            storage.legacy_backfill_completed_cursors.lock().await.len()
                <= ACCOUNT_WINDOW_STORAGE_MAX_ENTRIES
        );
    }

    #[tokio::test]
    async fn selection_health_does_not_hide_a_coverage_hole_in_another_active_entry() {
        let storage = AccountWindowStoragePlane::default();
        let mut entries = storage.entries.lock().await;
        entries.insert(
            "accounts=1;windows=short;cursor=1".to_string(),
            AccountWindowSelectionEntry {
                coverage_hole_bucket_count: 1,
                ..Default::default()
            },
        );
        entries.insert(
            "accounts=2;windows=short;cursor=1".to_string(),
            AccountWindowSelectionEntry::default(),
        );
        drop(entries);

        let health = storage.health_snapshot().await;
        assert_eq!(health.state, "deferred");
        assert_eq!(health.coverage_hole_bucket_count, 1);
    }

    #[test]
    fn lru_admission_never_exceeds_the_selection_entry_limit() {
        let mut entries = HashMap::new();
        for index in 0..ACCOUNT_WINDOW_STORAGE_MAX_ENTRIES {
            entries.insert(
                format!("accounts={index}"),
                AccountWindowSelectionEntry::default(),
            );
        }

        AccountWindowStoragePlane::make_room_for_selection(&mut entries, "accounts=next");
        assert_eq!(entries.len(), ACCOUNT_WINDOW_STORAGE_MAX_ENTRIES - 1);
        entries.insert(
            "accounts=next".to_string(),
            AccountWindowSelectionEntry::default(),
        );
        assert_eq!(entries.len(), ACCOUNT_WINDOW_STORAGE_MAX_ENTRIES);
    }

    #[test]
    fn window_usage_handler_delegates_database_access_to_storage_plane() {
        let source = include_str!("crud_group_notes.rs");
        let handler_start = source
            .find("pub(crate) async fn get_upstream_account_window_usage")
            .expect("window usage handler exists");
        let handler_end = source[handler_start..]
            .find("pub(crate) async fn list_forward_proxy_binding_nodes")
            .map(|offset| handler_start + offset)
            .expect("next route handler exists");
        let handler = &source[handler_start..handler_end];

        assert!(handler.contains("window_usage_storage"));
        assert!(!handler.contains("state.pool"));
    }
}
