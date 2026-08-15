use super::*;
use crate::db_pressure::global_db_pressure_gate;
use crate::maintenance::{
    StartupBackfillProgressUpdate, load_startup_backfill_progress, save_startup_backfill_progress,
};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use tokio::sync::Notify;

const ACCOUNT_WINDOW_STORAGE_MAX_ENTRIES: usize = 128;
const ACCOUNT_WINDOW_STORAGE_IDLE_TTL: Duration = Duration::from_secs(10 * 60);
const ACCOUNT_WINDOW_LAST_GOOD_MAX_AGE: Duration = Duration::from_secs(60);
const ACCOUNT_WINDOW_PREPARING_RETRY_AFTER_MS: u64 = 1_000;
const ACCOUNT_WINDOW_LEGACY_BACKFILL_LIMIT: i64 = 200;
const ACCOUNT_WINDOW_LEGACY_BACKFILL_BUDGET: Duration = Duration::from_millis(200);
const ACCOUNT_WINDOW_LEGACY_BACKFILL_PROGRESS_KEY: &str =
    "account_window_usage_upstream_account_id";

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

#[derive(Debug, Clone)]
struct AccountWindowStoredResult {
    response: AccountWindowStorageResponse,
    completed_at: Instant,
}

#[derive(Debug)]
struct AccountWindowSelectionEntry {
    in_flight: bool,
    last_used_at: Instant,
    last_good: Option<(UpstreamAccountWindowUsageResponse, Instant)>,
    completed: Option<AccountWindowStoredResult>,
    notify: Arc<Notify>,
}

impl Default for AccountWindowSelectionEntry {
    fn default() -> Self {
        Self {
            in_flight: false,
            last_used_at: Instant::now(),
            last_good: None,
            completed: None,
            notify: Arc::new(Notify::new()),
        }
    }
}

#[derive(Debug, Default)]
struct AccountWindowStorageHealthState {
    rollup_row_count: u64,
    minute_row_count: u64,
    bounded_raw_row_count: u64,
    coverage_hole_bucket_count: u64,
    backfill_cursor: i64,
    last_good_at: Option<Instant>,
    last_error: Option<String>,
}

#[derive(Debug)]
pub(crate) struct AccountWindowStoragePlane {
    entries: Mutex<HashMap<String, AccountWindowSelectionEntry>>,
    health: Arc<std::sync::Mutex<AccountWindowStorageHealthState>>,
    coalesced_waiter_count: AtomicU64,
    backfill_running: Arc<AtomicBool>,
}

impl Default for AccountWindowStoragePlane {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            health: Arc::new(std::sync::Mutex::new(
                AccountWindowStorageHealthState::default(),
            )),
            coalesced_waiter_count: AtomicU64::new(0),
            backfill_running: Arc::new(AtomicBool::new(false)),
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

        let selection = format!(
            "accounts={}",
            normalized_account_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(","),
        );

        let waiter = {
            let mut entries = self.entries.lock().await;
            Self::prune_entries(&mut entries);
            let entry = entries.entry(selection.clone()).or_default();
            entry.last_used_at = Instant::now();
            if entry.in_flight {
                self.coalesced_waiter_count.fetch_add(1, Ordering::Relaxed);
                let mut waiter = Box::pin(entry.notify.clone().notified_owned());
                waiter.as_mut().enable();
                Some(waiter)
            } else {
                entry.in_flight = true;
                None
            }
        };

        if let Some(waiter) = waiter {
            waiter.await;
            let entries = self.entries.lock().await;
            if let Some(result) = entries
                .get(&selection)
                .and_then(|entry| entry.completed.as_ref())
                .filter(|result| result.completed_at.elapsed() <= Duration::from_secs(5))
            {
                return Ok(result.response.clone());
            }
            return Ok(AccountWindowStorageResponse::Preparing {
                retry_after_ms: ACCOUNT_WINDOW_PREPARING_RETRY_AFTER_MS,
            });
        }

        let response = async {
            let durable_cursor =
                sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
                    .fetch_one(&state.pool)
                    .await?;
            self.build(state, &normalized_account_ids, durable_cursor)
                .await
        }
        .await;
        let (response, refresh_last_good) = match response {
            Ok(AccountWindowStorageResponse::Ready(response)) => {
                (AccountWindowStorageResponse::Ready(response), true)
            }
            Ok(AccountWindowStorageResponse::Preparing { retry_after_ms }) => (
                self.last_good_response(&selection)
                    .await
                    .unwrap_or(AccountWindowStorageResponse::Preparing { retry_after_ms }),
                false,
            ),
            Err(err) => {
                let message = err.to_string();
                self.record_error(message.clone());
                (
                    self.last_good_response(&selection).await.unwrap_or(
                        AccountWindowStorageResponse::Preparing {
                            retry_after_ms: ACCOUNT_WINDOW_PREPARING_RETRY_AFTER_MS,
                        },
                    ),
                    false,
                )
            }
        };
        self.complete_load(&selection, response.clone(), refresh_last_good)
            .await;

        self.schedule_legacy_backfill(
            state.pool.clone(),
            normalized_account_ids,
            Utc::now() - ChronoDuration::days(7),
        );
        Ok(response)
    }

    async fn last_good_response(&self, selection: &str) -> Option<AccountWindowStorageResponse> {
        self.entries
            .lock()
            .await
            .get(selection)
            .and_then(|entry| entry.last_good.as_ref())
            .filter(|(_, stored_at)| stored_at.elapsed() <= ACCOUNT_WINDOW_LAST_GOOD_MAX_AGE)
            .map(|(response, _)| AccountWindowStorageResponse::Ready(response.clone()))
    }

    async fn complete_load(
        &self,
        selection: &str,
        response: AccountWindowStorageResponse,
        refresh_last_good: bool,
    ) {
        let mut entries = self.entries.lock().await;
        if let Some(entry) = entries.get_mut(selection) {
            if refresh_last_good && let AccountWindowStorageResponse::Ready(payload) = &response {
                entry.last_good = Some((payload.clone(), Instant::now()));
                self.health
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .last_good_at = Some(Instant::now());
            }
            entry.completed = Some(AccountWindowStoredResult {
                response,
                completed_at: Instant::now(),
            });
            entry.in_flight = false;
            entry.notify.notify_waiters();
        }
    }

    async fn build(
        &self,
        state: &AppState,
        account_ids: &[i64],
        durable_cursor: i64,
    ) -> Result<AccountWindowStorageResponse> {
        let started_at = Instant::now();
        let mut summaries =
            load_upstream_account_window_usage_summaries(&state.pool, &state.config, account_ids)
                .await?;
        let (outcome, telemetry) = enrich_window_actual_usage_for_summaries_from_storage(
            &state.pool,
            &state.config,
            &mut summaries,
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
            health.coverage_hole_bucket_count = telemetry.coverage_hole_bucket_count as u64;
            if outcome == AccountWindowUsageBuildOutcome::Ready {
                health.last_error = None;
            }
        }
        tracing::info!(
            route = "upstream_account_window_usage",
            builder = "account_window_rollup",
            response_source = if outcome == AccountWindowUsageBuildOutcome::Ready { "rollup_boundary_tail" } else { "preparing" },
            selection_fingerprint = %durable_cursor,
            rollup_row_count = telemetry.rollup_row_count,
            minute_row_count = telemetry.minute_row_count,
            bounded_raw_row_count = telemetry.bounded_raw_row_count,
            coverage_hole_bucket_count = telemetry.coverage_hole_bucket_count,
            elapsed_ms = started_at.elapsed().as_millis() as u64,
            "upstream account window usage storage-plane build completed"
        );
        if outcome == AccountWindowUsageBuildOutcome::Preparing {
            return Ok(AccountWindowStorageResponse::Preparing {
                retry_after_ms: ACCOUNT_WINDOW_PREPARING_RETRY_AFTER_MS,
            });
        }
        Ok(AccountWindowStorageResponse::Ready(
            UpstreamAccountWindowUsageResponse {
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
            },
        ))
    }

    fn schedule_legacy_backfill(
        &self,
        pool: Pool<Sqlite>,
        account_ids: Vec<i64>,
        active_window_start: DateTime<Utc>,
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
        tokio::spawn(async move {
            struct BackfillGuard(Arc<AtomicBool>);
            impl Drop for BackfillGuard {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::Release);
                }
            }
            let _running = BackfillGuard(running);
            if let Err(err) =
                Self::run_legacy_backfill_pass(pool, account_ids, active_window_start, health).await
            {
                warn!(error = %err, "account window legacy backfill pass failed");
            }
        });
    }

    async fn run_legacy_backfill_pass(
        pool: Pool<Sqlite>,
        account_ids: Vec<i64>,
        active_window_start: DateTime<Utc>,
        health: Arc<std::sync::Mutex<AccountWindowStorageHealthState>>,
    ) -> Result<()> {
        let Ok(_permit) =
            global_db_pressure_gate().try_begin_background("account_window_usage_backfill")
        else {
            return Ok(());
        };
        let progress =
            load_startup_backfill_progress(&pool, ACCOUNT_WINDOW_LEGACY_BACKFILL_PROGRESS_KEY)
                .await?;
        let started_at = Instant::now();
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT id, payload FROM codex_invocations WHERE upstream_account_id IS NULL AND json_valid(payload) AND ",
        );
        query.push("CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) IN (");
        {
            let mut separated = query.separated(", ");
            for account_id in &account_ids {
                separated.push_bind(account_id);
            }
        }
        query
            .push(") ORDER BY CASE WHEN occurred_at >= ")
            .push_bind(active_window_start.to_rfc3339())
            .push(" THEN 0 ELSE 1 END, id ASC LIMIT ")
            .push_bind(ACCOUNT_WINDOW_LEGACY_BACKFILL_LIMIT);
        let rows = query
            .build_query_as::<(i64, Option<String>)>()
            .fetch_all(&pool)
            .await?;
        if rows.is_empty() {
            health
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .backfill_cursor = progress.cursor_id;
            return Ok(());
        }

        let mut tx = pool.begin().await?;
        let mut last_updated_id = None;
        let mut updated = 0_u64;
        for (id, payload) in &rows {
            if started_at.elapsed() >= ACCOUNT_WINDOW_LEGACY_BACKFILL_BUDGET {
                break;
            }
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
            last_updated_id = Some(*id);
        }
        tx.commit().await?;

        let cursor_id = last_updated_id.unwrap_or(progress.cursor_id);
        let next_run_after = Utc::now().to_rfc3339();
        save_startup_backfill_progress(
            &pool,
            ACCOUNT_WINDOW_LEGACY_BACKFILL_PROGRESS_KEY,
            StartupBackfillProgressUpdate {
                cursor_id,
                scanned: rows.len() as u64,
                updated,
                zero_update_streak: if updated == 0 {
                    progress.zero_update_streak.saturating_add(1)
                } else {
                    0
                },
                next_run_after: &next_run_after,
                status: "ok",
                suspension_reason: None,
            },
        )
        .await?;
        health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .backfill_cursor = cursor_id;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) async fn run_legacy_backfill_pass_for_test(
        pool: Pool<Sqlite>,
        account_ids: Vec<i64>,
        active_window_start: DateTime<Utc>,
    ) -> Result<()> {
        Self::run_legacy_backfill_pass(
            pool,
            account_ids,
            active_window_start,
            Arc::new(std::sync::Mutex::new(
                AccountWindowStorageHealthState::default(),
            )),
        )
        .await
    }

    pub(crate) async fn health_snapshot(&self) -> AccountWindowStoragePlaneHealthSnapshot {
        let entries = self.entries.lock().await;
        let health = self
            .health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let in_flight_build_count = entries.values().filter(|entry| entry.in_flight).count();
        let state = if health.last_error.is_some() {
            "degraded"
        } else if in_flight_build_count > 0 || health.coverage_hole_bucket_count > 0 {
            "deferred"
        } else {
            "healthy"
        };
        AccountWindowStoragePlaneHealthSnapshot {
            state: state.to_string(),
            active_selection_count: entries.len(),
            in_flight_build_count,
            coalesced_waiter_count: self.coalesced_waiter_count.load(Ordering::Relaxed),
            rollup_row_count: health.rollup_row_count,
            minute_row_count: health.minute_row_count,
            bounded_raw_row_count: health.bounded_raw_row_count,
            coverage_hole_bucket_count: health.coverage_hole_bucket_count,
            backfill_cursor: health.backfill_cursor,
            last_good_age_ms: health
                .last_good_at
                .map(|at| at.elapsed().as_millis() as u64),
            direct_pool_violation_count: 0,
            last_error: health.last_error.clone(),
        }
    }

    fn record_error(&self, error: String) {
        self.health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .last_error = Some(error);
    }

    fn prune_entries(entries: &mut HashMap<String, AccountWindowSelectionEntry>) {
        let now = Instant::now();
        entries.retain(|_, entry| {
            entry.in_flight
                || now.duration_since(entry.last_used_at) <= ACCOUNT_WINDOW_STORAGE_IDLE_TTL
        });
        if entries.len() <= ACCOUNT_WINDOW_STORAGE_MAX_ENTRIES {
            return;
        }
        let mut keys = entries
            .iter()
            .filter(|(_, entry)| !entry.in_flight)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stale_completion_does_not_extend_last_good_lifetime() {
        let storage = AccountWindowStoragePlane::default();
        let selection = "accounts=42";
        let response = UpstreamAccountWindowUsageResponse { items: Vec::new() };
        {
            let mut entries = storage.entries.lock().await;
            let entry = entries.entry(selection.to_string()).or_default();
            entry.last_good = Some((
                response.clone(),
                Instant::now() - ACCOUNT_WINDOW_LAST_GOOD_MAX_AGE - Duration::from_millis(1),
            ));
        }

        storage
            .complete_load(
                selection,
                AccountWindowStorageResponse::Ready(response),
                false,
            )
            .await;

        assert!(storage.last_good_response(selection).await.is_none());
    }
}
