use super::*;
use crate::observability::contention::{
    PerformanceCounter, PerformanceGauge, RecoveryDiagnostic, record_recovery,
};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::{from_str, to_string};
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(test)]
tokio::task_local! {
    static BOOTSTRAP_TIMEOUT_ELAPSED: Cell<bool>;
}

#[cfg(test)]
tokio::task_local! {
    static SUMMARY_PROJECTION_TEST_SETUP_DEADLINE: (
        SummaryProjectionTestInterleave,
        Duration,
        std::sync::Arc<tokio::sync::Notify>,
    );
}

#[cfg(test)]
pub(super) async fn observe_bootstrap_timeout<T>(
    future: impl std::future::Future<Output = T>,
) -> (T, bool) {
    BOOTSTRAP_TIMEOUT_ELAPSED
        .scope(Cell::new(false), async move {
            let result = future.await;
            (result, BOOTSTRAP_TIMEOUT_ELAPSED.with(Cell::get))
        })
        .await
}

fn summary_snapshot_bootstrap_keys(default_limit: i64) -> impl Iterator<Item = SummarySnapshotKey> {
    [
        SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(50),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        },
        SummaryQuery {
            window: Some("today".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        },
        SummaryQuery {
            window: Some("yesterday".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        },
        SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        },
        SummaryQuery {
            window: Some("30m".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        },
        SummaryQuery {
            window: Some("1h".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        },
        SummaryQuery {
            window: Some("7d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        },
        SummaryQuery {
            window: Some("thisWeek".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        },
        SummaryQuery {
            window: Some("1mo".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        },
        SummaryQuery {
            window: Some("thisMonth".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        },
        SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        },
        SummaryQuery {
            window: Some("previous7d".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        },
    ]
    .into_iter()
    .map(move |query| {
        SummarySnapshotKey::try_from_query(&query, default_limit)
            .expect("summary snapshot bootstrap query must be valid")
    })
}
pub(crate) async fn hydrate_summary_snapshots(state: &AppState) -> Result<()> {
    hydrate_summary_snapshots_with_deadline(state, SUMMARY_PROJECTION_STARTUP_BUILD_DEADLINE).await
}

pub(crate) async fn hydrate_summary_snapshots_with_deadline(
    state: &AppState,
    deadline: Duration,
) -> Result<()> {
    if state.pg_analytics.backend() == AnalyticsBackend::Postgres {
        return Ok(());
    }
    refresh_summary_snapshots_with_deadline(
        state,
        SummaryProjectionBuildMode::Bootstrap,
        Some(deadline),
        false,
    )
    .await
}

pub(crate) async fn refresh_summary_snapshots(state: &AppState) -> Result<()> {
    if state.pg_analytics.backend() == AnalyticsBackend::Postgres {
        return Ok(());
    }
    if state.subscription_hub.summary_projection().await.is_none() {
        // A cold retry has no exact historical coverage proof to reuse. Keep the first
        // publication on the Bootstrap deadline instead of downgrading it to the shorter
        // Rolling deadline used only after an immutable projection exists.
        hydrate_summary_snapshots(state).await?;
    } else if let Some(fallback_fence) = state
        .subscription_hub
        .summary_bootstrap_fallback_fence()
        .await
    {
        let current_fence = load_summary_projection_generation_fence(state).await?;
        if fallback_fence == current_fence {
            // The fallback already published every source which can be proved within the
            // bounded cold-start budget. Keep the local unavailable range in place until a
            // durable source fence changes; do not re-enter boundary/archive hydration.
            let _ = renew_summary_projection_freshness_if_generation_matches(state).await?;
        } else if fallback_fence.coverage_sources_match(current_fence) {
            // New terminals do not repair the historical authority which made Bootstrap
            // fall back. Reconcile only their bounded tail, preserving the published gap;
            // otherwise every active request stream can reopen the 30-second cold builder.
            refresh_summary_snapshots_with_mode(state, SummaryProjectionBuildMode::RollingDelta)
                .await?;
        } else {
            state
                .subscription_hub
                .clear_summary_bootstrap_fallback_fence()
                .await;
            // A changed manifest or coverage proof is a new historical admission identity.
            // Give Bootstrap one chance to replace the fallback before the supervisor catches up.
            hydrate_summary_snapshots(state).await?;
        }
    } else if !renew_summary_projection_freshness_if_generation_matches(state).await? {
        // Coverage revisions are reconciled by the historical supervisor and published through
        // its immutable overlay. If the live terminal tail did not move, keep recent selections
        // fresh without entering the archive-sized RollingDelta builder.
        let live_tail_renewed =
            renew_summary_projection_freshness_if_live_tail_matches(state).await?;
        if !live_tail_renewed {
            refresh_summary_snapshots_with_mode(state, SummaryProjectionBuildMode::RollingDelta)
                .await?;
        }
    }
    // Historical coverage is owned exclusively by the independent supervisor. Snapshot cadence
    // keeps this path limited to the availability-critical rolling Projection and must never
    // trigger durable V2 proof, archive, checkpoint, or finalization I/O.
    Ok(())
}

/// Owns historical Summary recovery independently from the availability-critical rolling
/// projection. Every pass advances at most one bounded proof/checkpoint page, then gives the
/// Snapshot V2 backfill a chance to recover the next missing authority. The generic projection
/// builder is intentionally never used here: archive raw hydration belongs to this supervisor.
pub(crate) struct SummaryCoverageRecoverySupervisor;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum SummaryCoverageRecoveryNextTurn {
    Immediate,
    Idle,
    Deferred,
}

#[derive(Debug)]
struct SummaryCoverageRecoveryTurn {
    next_turn: SummaryCoverageRecoveryNextTurn,
    reservation: Option<crate::db_pressure::DbBackgroundPriorityReservation>,
}

const SUMMARY_COVERAGE_RECOVERY_PROGRESS_YIELD: Duration = Duration::from_secs(1);
const SUMMARY_COVERAGE_RECOVERY_RETRY_INITIAL: Duration = Duration::from_secs(10);
const SUMMARY_COVERAGE_RECOVERY_RETRY_MAX: Duration = Duration::from_secs(60);

fn summary_coverage_recovery_retry_delay(current: Duration) -> Duration {
    current
        .saturating_mul(2)
        .min(SUMMARY_COVERAGE_RECOVERY_RETRY_MAX)
}

pub(crate) fn summary_coverage_recovery_scheduler_delay(
    next_turn: SummaryCoverageRecoveryNextTurn,
    deferred_retry_delay: Duration,
) -> Duration {
    match next_turn {
        SummaryCoverageRecoveryNextTurn::Deferred => deferred_retry_delay,
        SummaryCoverageRecoveryNextTurn::Immediate => SUMMARY_COVERAGE_RECOVERY_PROGRESS_YIELD,
        SummaryCoverageRecoveryNextTurn::Idle => SUMMARY_SNAPSHOT_MIN_REFRESH_INTERVAL,
    }
}

pub(crate) fn summary_coverage_recovery_requires_second_v2_turn(
    result: &SummaryArchiveSnapshotBackfillWindowResult,
) -> bool {
    result.candidate_count > 0
        || result.pending_obligation_count > 0
        || result.hit_budget
        || result.materialized_archive_batches > 0
        || result.unavailable_archive_batches > 0
}

#[cfg(test)]
fn note_summary_coverage_v2_window() {
    let _ = SUMMARY_COVERAGE_TEST_V2_WINDOW_CALLS.try_with(|calls| {
        calls.set(calls.get().saturating_add(1));
    });
}

#[cfg(test)]
fn note_summary_coverage_checkpoint_advance() {
    let _ = SUMMARY_COVERAGE_TEST_CHECKPOINT_ADVANCE_CALLS.try_with(|calls| {
        calls.set(calls.get().saturating_add(1));
    });
}

#[cfg(test)]
pub(crate) fn note_summary_projection_historical_identity_hydration(rows: usize) {
    let _ = SUMMARY_PROJECTION_TEST_HISTORICAL_IDENTITY_HYDRATION_ROWS.try_with(|count| {
        count.set(count.get().saturating_add(rows));
    });
}

#[cfg(test)]
pub(crate) fn note_summary_projection_boundary_archive_hydration() {
    let _ = SUMMARY_PROJECTION_TEST_BOUNDARY_ARCHIVE_HYDRATION_CALLS.try_with(|calls| {
        calls.set(calls.get().saturating_add(1));
    });
}

pub(crate) async fn summary_all_time_checkpoint_publication_required(
    state: &AppState,
    checkpoint: &SummaryAllTimeProjectionCheckpointRow,
) -> Result<bool> {
    let Some(projection) = state.subscription_hub.summary_projection().await else {
        return Ok(true);
    };
    let coverage_fence = checkpoint.generation_fence().coverage_fence();
    // Historical readiness is independent from the live tail. A completed checkpoint can
    // remain coverage-compatible while new committed rows arrive after its rollup cursor. In
    // that case the finalizer must run again so the bounded live-tail query folds those rows into
    // the all-time aggregate; otherwise the rolling projection advances its fence while the
    // retained all-time response stays one or more rows behind indefinitely.
    let durable_live_tail = load_summary_projection_generation_fence(state)
        .await?
        .live_tail_cursor();
    let checkpoint_live_tail = checkpoint.generation_fence().live_tail_cursor();
    if checkpoint.global_ready()
        && (durable_live_tail.live_high_watermark_id > checkpoint_live_tail.live_high_watermark_id
            || durable_live_tail.rollup_live_cursor > checkpoint_live_tail.rollup_live_cursor
            || durable_live_tail.durable_terminal_sequence_watermark
                > checkpoint_live_tail.durable_terminal_sequence_watermark)
    {
        return Ok(true);
    }
    if checkpoint.account_ready()
        && (durable_live_tail.live_high_watermark_id > checkpoint_live_tail.live_high_watermark_id
            || durable_live_tail.account_rollup_live_cursor
                > checkpoint_live_tail.account_rollup_live_cursor
            || durable_live_tail.durable_terminal_sequence_watermark
                > checkpoint_live_tail.durable_terminal_sequence_watermark)
    {
        return Ok(true);
    }
    if checkpoint.global_ready()
        && (!projection.freshness.global_all_time_eligible
            || projection
                .global_all_time_coverage_fence
                .is_none_or(|published| !published.global_sources_match(coverage_fence)))
    {
        return Ok(true);
    }
    // A ready checkpoint can outlive the immutable all-time snapshot it produced.  This is
    // especially visible when a large staged recovery keeps the supervisor busy for longer than
    // the serving freshness budget: the coverage fence is still valid, but retaining the old
    // response would make the next request fail closed without scheduling a bounded finalization.
    // Re-enter the finalizer only for the affected scope; fresh ready projections remain no-ops.
    if checkpoint.global_ready()
        && projection.freshness.global_all_time_eligible
        && projection
            .all_time_refreshed_at
            .is_none_or(|refreshed_at| refreshed_at.elapsed() > SUMMARY_SNAPSHOT_MAX_STALE)
    {
        return Ok(true);
    }
    if checkpoint.account_ready() {
        if projection
            .account_all_time_coverage_fence
            .is_none_or(|published| !published.account_sources_match(coverage_fence))
        {
            return Ok(true);
        }
        let checkpoint_account_ids = sqlx::query_scalar::<_, i64>(
            "SELECT upstream_account_id \
             FROM summary_all_time_projection_account_checkpoint WHERE scope = ?1",
        )
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .fetch_all(&state.pool)
        .await
        .context("summary all-time publication account readiness lookup failed")?;
        if checkpoint_account_ids.iter().any(|account_id| {
            !projection
                .freshness
                .account_all_time_eligible
                .contains(account_id)
                || projection
                    .all_time_account_refreshed_at
                    .get(account_id)
                    .is_none_or(|refreshed_at| refreshed_at.elapsed() > SUMMARY_SNAPSHOT_MAX_STALE)
        }) {
            return Ok(true);
        }
    }
    Ok(false)
}

impl SummaryCoverageRecoverySupervisor {
    pub(crate) async fn run(state: &AppState) -> Result<()> {
        Self::run_with_priority_reservation(state, None)
            .await
            .map(|_| ())
    }

    async fn run_with_startup_priority_reservation(
        state: &AppState,
        reservation: crate::db_pressure::DbBackgroundPriorityReservation,
        runtime: &mut CompactedSourceRecoveryRuntime,
    ) -> Result<SummaryCoverageRecoveryNextTurn> {
        Self::run_with_priority_reservation_runtime(state, Some(reservation), runtime).await
    }

    pub(crate) async fn run_with_priority_reservation(
        state: &AppState,
        reservation: Option<crate::db_pressure::DbBackgroundPriorityReservation>,
    ) -> Result<SummaryCoverageRecoveryNextTurn> {
        let mut runtime = CompactedSourceRecoveryRuntime::default();
        Self::run_with_priority_reservation_runtime(state, reservation, &mut runtime).await
    }

    async fn run_with_priority_reservation_runtime(
        state: &AppState,
        reservation: Option<crate::db_pressure::DbBackgroundPriorityReservation>,
        runtime: &mut CompactedSourceRecoveryRuntime,
    ) -> Result<SummaryCoverageRecoveryNextTurn> {
        if state.pg_analytics.backend() == AnalyticsBackend::Postgres {
            return Ok(SummaryCoverageRecoveryNextTurn::Idle);
        }
        let Ok(_singleflight) = state.subscription_hub.try_lock_summary_coverage_recovery() else {
            return Ok(SummaryCoverageRecoveryNextTurn::Idle);
        };
        Self::run_with_priority_reservation_locked(state, reservation, runtime).await
    }

    async fn run_with_priority_reservation_locked(
        state: &AppState,
        reservation: Option<crate::db_pressure::DbBackgroundPriorityReservation>,
        runtime: &mut CompactedSourceRecoveryRuntime,
    ) -> Result<SummaryCoverageRecoveryNextTurn> {
        // A recovery invocation owns one bounded admission turn. In particular, a turn must not
        // hand a fresh reservation back to this function and immediately chain another turn while
        // retaining the execution lease. The caller drops the returned turn at this boundary, so
        // any reservation created by legacy branches is released before the next scheduler tick.
        let turn = match Self::run_once(state, reservation, true, runtime).await {
            Ok(turn) => turn,
            Err(error)
                if error
                    .downcast_ref::<SummaryProjectionAllTimeGenerationChanged>()
                    .is_some() =>
            {
                // A coverage fence changed during the bounded turn. Release the current
                // admission and let the scheduler retry against the new identity; do not chain
                // a second turn while retaining the execution lease.
                runtime.mark_progress();
                return Ok(SummaryCoverageRecoveryNextTurn::Immediate);
            }
            Err(error) => return Err(error),
        };
        let next_turn = turn.next_turn;
        // Never carry a reservation across the scheduler boundary. The old implementation
        // returned it to this function and immediately ran another turn while holding the task
        // lease, which starved request-facing and unrelated background work.
        drop(turn.reservation);
        Ok(next_turn)
    }

    async fn run_once(
        state: &AppState,
        reservation: Option<crate::db_pressure::DbBackgroundPriorityReservation>,
        publish_overlay: bool,
        runtime: &mut CompactedSourceRecoveryRuntime,
    ) -> Result<SummaryCoverageRecoveryTurn> {
        // Admission deliberately precedes every checkpoint, manifest, and archive operation.
        // Pressure is scheduler-only: a denied pass neither reads nor writes recovery progress.
        let pressure_gate = crate::db_pressure::global_db_pressure_gate();
        let queue_wait = if reservation.is_some() {
            SUMMARY_HISTORICAL_COVERAGE_RECOVERY_CONTINUATION_QUEUE_WAIT
        } else {
            SUMMARY_HISTORICAL_COVERAGE_RECOVERY_QUEUE_WAIT
        };
        let admission = match reservation {
            Some(reservation) => {
                pressure_gate
                    .begin_reserved_priority_background(
                        reservation,
                        queue_wait,
                        Some("summary_historical_coverage_recovery"),
                    )
                    .await
            }
            None => {
                pressure_gate
                    .begin_priority_background_with_queue_wait(
                        "summary_historical_coverage_recovery",
                        queue_wait,
                    )
                    .await
            }
        };
        let _pressure_permit = match admission {
            Ok(permit) => permit,
            Err(reason) => {
                debug!(
                    stage = "historical_coverage_pressure_defer",
                    reason = %reason,
                    "summary historical coverage recovery deferred before durable progress access"
                );
                state.observability.count_performance(
                    PerformanceCounter::Recovery(
                        RecoveryDiagnostic::SummaryRecoveryPressureDeferred,
                    ),
                    1,
                );
                crate::record_managed_task_discovered_work(&["summary_coverage_recovery"], 0);
                crate::record_managed_task_processed_work(&["summary_coverage_recovery"], 0);
                return Ok(SummaryCoverageRecoveryTurn {
                    next_turn: SummaryCoverageRecoveryNextTurn::Deferred,
                    reservation: None,
                });
            }
        };
        record_recovery(RecoveryDiagnostic::StatisticsHistoricalCoverage);
        if let Some(turn) = run_compacted_source_recovery_turn_with_runtime(state, runtime).await? {
            return Ok(turn);
        }
        let started_at = Instant::now();
        // Required 30-day authority takes priority over all-time aggregation. Each V2 turn is
        // page- and time-bounded, and committed cursor/hash progress is immediately eligible
        // for the next turn. Do not let a broad all-time checkpoint consume the only recovery
        // permit while a recent selection still lacks its authority.
        #[cfg(test)]
        note_summary_coverage_v2_window();
        let priority_backfill = backfill_summary_archive_snapshots_v2_window(
            &state.pool,
            SUMMARY_HISTORICAL_COVERAGE_BACKFILL_BUDGET,
        )
        .await?;
        crate::record_managed_task_discovered_work(
            &["summary_coverage_recovery"],
            i64::try_from(priority_backfill.candidate_count).unwrap_or(i64::MAX),
        );
        info!(
            stage = "historical_coverage_snapshot_backfill",
            elapsed_ms = started_at.elapsed().as_millis() as u64,
            candidate_count = priority_backfill.candidate_count,
            recent_candidate_count = priority_backfill.recent_candidate_count,
            scanned_archive_batches = priority_backfill.scanned_archive_batches,
            materialized_archive_batches = priority_backfill.materialized_archive_batches,
            unavailable_archive_batches = priority_backfill.unavailable_archive_batches,
            pending_obligation_count = priority_backfill.pending_obligation_count,
            terminal_gap_count = priority_backfill.terminal_gap_count,
            verified_proof_count = priority_backfill.verified_proof_count,
            hit_budget = priority_backfill.hit_budget,
            "summary historical coverage recovery page completed"
        );
        // `candidate_count` includes the all-time backlog. Recent candidates control the first
        // overlay publication, but the durable obligation count controls all-time finalization:
        // no historical checkpoint may run until every archive contribution is proven or has a
        // persisted terminal gap.
        let priority_backfill_pending = priority_backfill.recent_candidate_count > 0;
        let recent_overlay_needed = !priority_backfill_pending
            && summary_projection_recent_coverage_needs_overlay(state).await;
        // A final proof is durable before it is published. During startup, batch immutable
        // overlay swaps so a long recovery does not repeatedly clone the resident Projection.
        // The final no-pending turn below always publishes, preserving exact-or-unavailable.
        if publish_overlay
            && (recent_overlay_needed
                || (priority_backfill.pending_obligation_count == 0 && priority_backfill_pending))
        {
            let publication_started_at = Instant::now();
            // Full reduction is reserved for the final no-pending pass. While the broad
            // historical queue is still draining, newly verified pages must extend the existing
            // overlay incrementally; rescanning every V2 proof here can consume the entire
            // all-time readiness budget.
            match publish_summary_coverage_overlay(
                state,
                priority_backfill.pending_obligation_count == 0,
            )
            .await
            {
                Ok(true) => info!(
                    stage = "historical_coverage_overlay_publication",
                    elapsed_ms = publication_started_at.elapsed().as_millis() as u64,
                    "summary historical coverage proof published through immutable overlay"
                ),
                Ok(false) => debug!(
                    stage = "historical_coverage_overlay_deferred",
                    elapsed_ms = publication_started_at.elapsed().as_millis() as u64,
                    "summary historical coverage overlay had no publishable contribution"
                ),
                Err(error)
                    if error
                        .to_string()
                        .contains("summary V2 boundary record budget exceeded") =>
                {
                    debug!(
                        stage = "historical_coverage_overlay_deferred",
                        reason = "boundary_record_budget",
                        elapsed_ms = publication_started_at.elapsed().as_millis() as u64,
                        "summary historical coverage overlay deferred until final proof reduction"
                    );
                }
                Err(error) => return Err(error),
            }
        }
        if priority_backfill.pending_obligation_count > 0 {
            // Historical obligations always outrank generic all-time aggregation. This is
            // intentionally independent of the startup flag: an `all` HTTP request may arrive
            // after bootstrap, but it must not claim the recovery slot and starve the V2 page
            // queue. Keep the supervisor's bounded loop alive while obligations remain so the
            // 30-second cadence does not turn a finite recovery set into an hour-long drain.
            // The database-pressure permit is still released at this boundary, so every source
            // page stays bounded and pressure refusal can stop the loop safely.
            return Ok(SummaryCoverageRecoveryTurn {
                next_turn: SummaryCoverageRecoveryNextTurn::Immediate,
                reservation: Some(pressure_gate.reserve_priority_background()),
            });
        }
        // A completed global checkpoint is independently exact. Do not make its first
        // publication wait for the account-scope manifest/rollup pass below: large account
        // rosters can legitimately take more than the bounded finalization turn while the
        // global `all` response is already proven. The publication function performs its own
        // live/coverage fence check before the immutable swap.
        let checkpoint = load_summary_all_time_projection_checkpoint(&state.pool).await?;
        if let Some(global_checkpoint) = checkpoint.as_ref()
            && global_checkpoint.global_ready()
            && summary_all_time_checkpoint_publication_required(state, global_checkpoint).await?
        {
            let publication_started_at = Instant::now();
            match tokio::time::timeout(
                SUMMARY_PROJECTION_ALL_TIME_FINALIZATION_DEADLINE,
                publish_summary_all_time_projection_checkpoint(state, global_checkpoint.clone()),
            )
            .await
            {
                Ok(result) => result?,
                Err(_) => {
                    warn!(
                        stage = "historical_coverage_global_projection_publish_deferred",
                        elapsed_ms = publication_started_at.elapsed().as_millis() as u64,
                        "summary global all-time projection publication exceeded its bounded turn"
                    );
                }
            }
        }
        if !summary_coverage_recovery_requires_second_v2_turn(&priority_backfill)
            && let Some(checkpoint) = checkpoint.as_ref()
            && checkpoint.global_settled()
            && checkpoint.account_settled()
            && !summary_all_time_checkpoint_publication_required(state, checkpoint).await?
        {
            debug!(
                stage = "historical_coverage_projection_finalize",
                "summary idle recovery skipped unchanged ready checkpoint"
            );
            return Ok(SummaryCoverageRecoveryTurn {
                next_turn: SummaryCoverageRecoveryNextTurn::Idle,
                reservation: None,
            });
        }
        #[cfg(test)]
        note_summary_coverage_checkpoint_advance();
        let checkpoint = tokio::time::timeout(
            SUMMARY_PROJECTION_ALL_TIME_FINALIZATION_DEADLINE,
            advance_summary_all_time_projection_checkpoint(state),
        )
        .await
        .map_err(|_| {
            anyhow!(
                "summary historical coverage checkpoint exceeded {SUMMARY_PROJECTION_ALL_TIME_FINALIZATION_DEADLINE:?}"
            )
        })??;

        if (checkpoint.global_ready() || checkpoint.account_ready())
            && summary_all_time_checkpoint_publication_required(state, &checkpoint).await?
        {
            let global_ready = checkpoint.global_ready();
            let account_ready = checkpoint.account_ready();
            #[cfg(test)]
            pause_summary_projection_test_interleave(
                &state.pool,
                SummaryProjectionBuildMode::AllTime,
                SummaryProjectionTestInterleaveStage::BeforeProjectionPublication,
            )
            .await?;
            tokio::time::timeout(
                SUMMARY_PROJECTION_ALL_TIME_FINALIZATION_DEADLINE,
                publish_summary_all_time_projection_checkpoint(state, checkpoint),
            )
            .await
            .map_err(|_| {
                anyhow!(
                    "summary historical coverage finalization exceeded {SUMMARY_PROJECTION_ALL_TIME_FINALIZATION_DEADLINE:?}"
                )
            })??;
            info!(
                stage = "historical_coverage_projection_publish",
                elapsed_ms = started_at.elapsed().as_millis() as u64,
                global_ready,
                account_ready,
                "summary historical coverage projection published"
            );
        }

        let needs_second_v2_turn =
            summary_coverage_recovery_requires_second_v2_turn(&priority_backfill);
        let backfill = if needs_second_v2_turn {
            #[cfg(test)]
            note_summary_coverage_v2_window();
            backfill_summary_archive_snapshots_v2_window(
                &state.pool,
                SUMMARY_HISTORICAL_COVERAGE_BACKFILL_BUDGET,
            )
            .await?
        } else {
            debug!(
                stage = "historical_coverage_snapshot_backfill",
                "summary idle recovery skipped redundant second V2 window"
            );
            priority_backfill
        };
        if needs_second_v2_turn {
            crate::record_managed_task_discovered_work(
                &["summary_coverage_recovery"],
                i64::try_from(backfill.candidate_count).unwrap_or(i64::MAX),
            );
        }
        info!(
            stage = "historical_coverage_snapshot_backfill",
            elapsed_ms = started_at.elapsed().as_millis() as u64,
            candidate_count = backfill.candidate_count,
            recent_candidate_count = backfill.recent_candidate_count,
            scanned_archive_batches = backfill.scanned_archive_batches,
            materialized_archive_batches = backfill.materialized_archive_batches,
            unavailable_archive_batches = backfill.unavailable_archive_batches,
            pending_obligation_count = backfill.pending_obligation_count,
            terminal_gap_count = backfill.terminal_gap_count,
            verified_proof_count = backfill.verified_proof_count,
            hit_budget = backfill.hit_budget,
            "summary historical coverage recovery page completed"
        );

        // A successful V2 page may complete the checkpoint. Give finalization one bounded turn,
        // then yield. Repeating a generic recovery loop here previously let one supervisor pass
        // monopolize maintenance and made the V2 worker depend on all-time readiness.
        if started_at.elapsed() < SUMMARY_PROJECTION_ALL_TIME_FINALIZATION_DEADLINE {
            // A manifest page advances its seek cursor before the following empty page marks
            // that scope complete. Give the checkpoint a second bounded turn after the V2 page
            // commit so a small candidate becomes publishable in this recovery pass; larger
            // histories still yield after these bounded turns and resume from their cursor.
            for _ in 0..2 {
                if started_at.elapsed() >= SUMMARY_PROJECTION_ALL_TIME_FINALIZATION_DEADLINE {
                    break;
                }
                let checkpoint = advance_summary_all_time_projection_checkpoint(state).await?;
                if (checkpoint.global_ready() || checkpoint.account_ready())
                    && summary_all_time_checkpoint_publication_required(state, &checkpoint).await?
                {
                    #[cfg(test)]
                    pause_summary_projection_test_interleave(
                        &state.pool,
                        SummaryProjectionBuildMode::AllTime,
                        SummaryProjectionTestInterleaveStage::BeforeProjectionPublication,
                    )
                    .await?;
                    publish_summary_all_time_projection_checkpoint(state, checkpoint).await?;
                    break;
                }
            }
        }

        // V2 proof changes are published as a compact immutable overlay. This avoids re-running
        // live admission after every historical page and keeps the 4-second RollingDelta budget
        // reserved for actual terminal tail changes.
        if backfill.pending_obligation_count == 0 {
            let publication_started_at = Instant::now();
            match publish_summary_coverage_overlay(state, false).await {
                Ok(true) => info!(
                    stage = "historical_coverage_overlay_publication",
                    elapsed_ms = publication_started_at.elapsed().as_millis() as u64,
                    "summary historical coverage proof published through immutable overlay"
                ),
                Ok(false) => debug!(
                    stage = "historical_coverage_overlay_deferred",
                    elapsed_ms = publication_started_at.elapsed().as_millis() as u64,
                    "summary historical coverage overlay had no publishable contribution"
                ),
                Err(error)
                    if error
                        .to_string()
                        .contains("summary V2 boundary record budget exceeded") =>
                {
                    debug!(
                        stage = "historical_coverage_overlay_deferred",
                        reason = "boundary_record_budget",
                        elapsed_ms = publication_started_at.elapsed().as_millis() as u64,
                        "summary historical coverage overlay deferred until final proof reduction"
                    );
                }
                Err(error) => return Err(error),
            }
        }
        Ok(SummaryCoverageRecoveryTurn {
            next_turn: if summary_coverage_recovery_requires_second_v2_turn(&backfill) {
                SummaryCoverageRecoveryNextTurn::Immediate
            } else {
                SummaryCoverageRecoveryNextTurn::Idle
            },
            reservation: None,
        })
    }
}

async fn summary_projection_recent_coverage_needs_overlay(state: &AppState) -> bool {
    let Some(projection) = state.subscription_hub.summary_projection().await else {
        return true;
    };
    let Some(overlay) = projection.coverage_overlay.as_ref() else {
        return true;
    };
    let end = Utc::now();
    let start = end - ChronoDuration::days(30);
    let recent_proof_identities =
        load_summary_v2_archive_proof_identities_in_range(&state.pool, start, end)
            .await
            .unwrap_or_default();
    overlay.recent_proof_identities != recent_proof_identities
}

/// Runs the durable historical supervisor independently from rolling Projection maintenance.
/// It retains priority only while the startup's finite durable obligation set is being recovered;
/// afterwards periodic turns are best-effort and cannot delay current/rolling refresh.
pub(crate) fn spawn_summary_coverage_recovery_maintenance(
    state: Arc<AppState>,
    startup_priority: crate::db_pressure::DbBackgroundPriorityReservation,
) {
    crate::observability::contention::spawn_performance(state.observability.clone(), async move {
        if state.pg_analytics.backend() == AnalyticsBackend::Postgres {
            return;
        }
        if crate::maintenance_store::legacy_worker_should_skip("summary_coverage_recovery").await {
            return;
        }
        let mut recovery_runtime = CompactedSourceRecoveryRuntime::default();
        let mut next_turn = {
            let Some(_execution_lease) =
                crate::maintenance_store::try_acquire_task_execution("summary_coverage_recovery")
            else {
                return;
            };
            let observation = crate::TaskExecutionObservation::begin(
                "summary_coverage_recovery",
                &crate::maintenance_store::task_title_for_observation("summary_coverage_recovery"),
                "startup",
                crate::maintenance_store::task_execution_class("summary_coverage_recovery"),
                "processing",
            );
            let initial_result = crate::with_managed_task_observation(
                observation.clone(),
                async {
                    tokio::select! {
                        biased;
                        _ = state.shutdown.cancelled() => None,
                        result = SummaryCoverageRecoverySupervisor::run_with_startup_priority_reservation(
                            state.as_ref(),
                            startup_priority,
                            &mut recovery_runtime,
                        ) => Some(result),
                    }
                },
            )
            .await;
            let Some(initial_result) = initial_result else {
                return;
            };
            observation.finish_with_status(if initial_result.is_ok() {
                "success"
            } else {
                "skipped"
            });
            match initial_result {
                Ok(next_turn) => next_turn,
                Err(error) => {
                    warn!(error = ?error, "initial summary historical coverage recovery deferred");
                    SummaryCoverageRecoveryNextTurn::Idle
                }
            }
        };
        // The initial pass establishes the runtime identity. The first scheduled retry always
        // starts at the initial delay; only later turns participate in exponential backoff.
        let _ = recovery_runtime.take_retry_reset();
        let mut deferred_retry_delay = SUMMARY_COVERAGE_RECOVERY_RETRY_INITIAL;
        loop {
            let delay = summary_coverage_recovery_scheduler_delay(next_turn, deferred_retry_delay);
            state.observability.set_performance(
                PerformanceGauge::SummaryRecoveryRetryDelay,
                delay.as_secs_f64(),
            );
            debug!(
                next_turn = ?next_turn,
                retry_delay_seconds = delay.as_secs_f64(),
                "scheduled summary coverage recovery turn"
            );
            tokio::select! {
                _ = state.shutdown.cancelled() => return,
                _ = tokio::time::sleep(delay) => {}
            }
            if crate::maintenance_store::legacy_worker_should_skip("summary_coverage_recovery")
                .await
            {
                continue;
            }
            let Some(_execution_lease) =
                crate::maintenance_store::try_acquire_task_execution("summary_coverage_recovery")
            else {
                continue;
            };
            let observation = crate::TaskExecutionObservation::begin(
                "summary_coverage_recovery",
                &crate::maintenance_store::task_title_for_observation("summary_coverage_recovery"),
                "adaptive",
                crate::maintenance_store::task_execution_class("summary_coverage_recovery"),
                "processing",
            );
            let result = crate::with_managed_task_observation(observation.clone(), async {
                tokio::select! {
                    biased;
                    _ = state.shutdown.cancelled() => None,
                    result = SummaryCoverageRecoverySupervisor::run_with_priority_reservation_runtime(
                        state.as_ref(),
                        None,
                        &mut recovery_runtime,
                    ) => Some(result),
                }
            })
            .await;
            let Some(result) = result else {
                return;
            };
            observation.finish_with_status(if result.is_ok() { "success" } else { "skipped" });
            let scheduled_next_turn = match result {
                Ok(next_turn) => next_turn,
                Err(error) => {
                    warn!(error = ?error, "summary historical coverage recovery deferred");
                    SummaryCoverageRecoveryNextTurn::Idle
                }
            };
            let progress_or_identity_changed = recovery_runtime.take_retry_reset();
            if progress_or_identity_changed
                || scheduled_next_turn != SummaryCoverageRecoveryNextTurn::Deferred
            {
                deferred_retry_delay = SUMMARY_COVERAGE_RECOVERY_RETRY_INITIAL;
            } else {
                deferred_retry_delay = summary_coverage_recovery_retry_delay(deferred_retry_delay);
                state.observability.count_performance(
                    PerformanceCounter::Recovery(RecoveryDiagnostic::SummaryRecoveryRetry),
                    1,
                );
            }
            next_turn = scheduled_next_turn;
        }
    });
}

pub(crate) async fn renew_summary_projection_freshness_if_generation_matches(
    state: &AppState,
) -> Result<bool> {
    let generation_fence = load_summary_projection_generation_fence(state).await?;
    let renewed = state
        .subscription_hub
        .renew_summary_projection_freshness_if_generation_matches(generation_fence)
        .await;
    if renewed {
        debug!("summary projection freshness renewed for unchanged durable generation");
    }
    Ok(renewed)
}

pub(crate) async fn renew_summary_projection_freshness_if_live_tail_matches(
    state: &AppState,
) -> Result<bool> {
    let generation_fence = load_summary_projection_generation_fence(state).await?;
    let renewed = state
        .subscription_hub
        .renew_summary_projection_freshness_if_live_tail_matches(generation_fence)
        .await;
    if renewed {
        debug!(
            "summary projection freshness renewed for unchanged live tail during coverage recovery"
        );
    }
    Ok(renewed)
}

async fn renew_summary_projection_freshness_if_coverage_matches(state: &AppState) -> Result<bool> {
    let generation_fence = load_summary_projection_generation_fence(state).await?;
    let renewed = state
        .subscription_hub
        .renew_summary_projection_freshness_if_coverage_matches(generation_fence)
        .await;
    if renewed {
        debug!("summary all-time coverage freshness renewed for unchanged coverage fence");
    }
    Ok(renewed)
}

/// Reconstruct the bounded durable source tail after a restart or when the in-process delta
/// queue has been dropped.  Only descriptor identities are read from the journal; the canonical
/// source row is hydrated in 400-id chunks and exposed through the existing in-memory replay
/// overlay.  This function deliberately never invokes the full live admission builder.
const SUMMARY_LIVE_TAIL_RECONCILIATION_SCOPE: &str = "summary-global";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SummaryLiveTailReconciliationCheckpoint {
    pub(crate) format_version: i64,
    pub(crate) recovery_epoch: i64,
    pub(crate) base_projection_revision: i64,
    pub(crate) target_terminal_watermark: u64,
    pub(crate) target_source_cursor: u64,
    pub(crate) next_source_cursor: u64,
    pub(crate) state: String,
}

pub(crate) async fn load_summary_live_tail_reconciliation_checkpoint(
    pool: &sqlx::Pool<sqlx::Sqlite>,
) -> Result<Option<SummaryLiveTailReconciliationCheckpoint>> {
    let row = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, String)>(
        "SELECT format_version, recovery_epoch, base_projection_revision, \
                target_terminal_watermark, target_source_cursor, next_source_cursor, state \
         FROM summary_live_tail_reconciliation_checkpoint WHERE scope = ?1",
    )
    .bind(SUMMARY_LIVE_TAIL_RECONCILIATION_SCOPE)
    .fetch_optional(pool)
    .await
    .context("load Summary live-tail reconciliation checkpoint")?;
    row.map(
        |(
            format_version,
            recovery_epoch,
            base_projection_revision,
            target_terminal_watermark,
            target_source_cursor,
            next_source_cursor,
            state,
        )| {
            Ok(SummaryLiveTailReconciliationCheckpoint {
                format_version,
                recovery_epoch,
                base_projection_revision,
                target_terminal_watermark: u64::try_from(target_terminal_watermark)
                    .context("live-tail checkpoint target watermark is negative")?,
                target_source_cursor: u64::try_from(target_source_cursor)
                    .context("live-tail checkpoint target cursor is negative")?,
                next_source_cursor: u64::try_from(next_source_cursor)
                    .context("live-tail checkpoint next cursor is negative")?,
                state,
            })
        },
    )
    .transpose()
}

pub(crate) async fn store_summary_live_tail_reconciliation_checkpoint(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    checkpoint: SummaryLiveTailReconciliationCheckpoint,
) -> Result<()> {
    let mut transaction = pool
        .begin()
        .await
        .context("begin Summary live-tail reconciliation checkpoint")?;
    store_summary_live_tail_reconciliation_checkpoint_tx(&mut transaction, checkpoint).await?;
    transaction
        .commit()
        .await
        .context("commit Summary live-tail reconciliation checkpoint")?;
    Ok(())
}

async fn store_summary_live_tail_reconciliation_checkpoint_tx(
    connection: &mut sqlx::SqliteConnection,
    checkpoint: SummaryLiveTailReconciliationCheckpoint,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO summary_live_tail_reconciliation_checkpoint \
         (scope, format_version, recovery_epoch, base_projection_revision, \
          target_terminal_watermark, target_source_cursor, next_source_cursor, state, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now')) \
         ON CONFLICT(scope) DO UPDATE SET \
          format_version = excluded.format_version, \
          recovery_epoch = excluded.recovery_epoch, \
          base_projection_revision = excluded.base_projection_revision, \
          target_terminal_watermark = excluded.target_terminal_watermark, \
          target_source_cursor = excluded.target_source_cursor, \
          next_source_cursor = excluded.next_source_cursor, \
          state = excluded.state, updated_at = datetime('now')",
    )
    .bind(SUMMARY_LIVE_TAIL_RECONCILIATION_SCOPE)
    .bind(checkpoint.format_version)
    .bind(checkpoint.recovery_epoch)
    .bind(checkpoint.base_projection_revision)
    .bind(
        i64::try_from(checkpoint.target_terminal_watermark)
            .context("live-tail checkpoint target watermark overflow")?,
    )
    .bind(
        i64::try_from(checkpoint.target_source_cursor)
            .context("live-tail checkpoint target cursor overflow")?,
    )
    .bind(
        i64::try_from(checkpoint.next_source_cursor)
            .context("live-tail checkpoint next cursor overflow")?,
    )
    .bind(checkpoint.state)
    .execute(&mut *connection)
    .await
    .context("store Summary live-tail reconciliation checkpoint")?;
    Ok(())
}

async fn record_summary_live_tail_readiness(
    state: &AppState,
    reason: &str,
    stage: &str,
    gap_count: usize,
    watermark: u64,
    epoch: u64,
    started: Instant,
) {
    state
        .subscription_hub
        .set_summary_live_tail_readiness(SummaryLiveTailReadiness {
            reason: reason.to_string(),
            stage: stage.to_string(),
            gap_count: gap_count as u64,
            watermark,
            epoch,
            elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        })
        .await;
}

async fn restore_summary_source_change_tail(
    state: &AppState,
) -> Result<(bool, Option<SummaryProjectionGenerationFence>)> {
    let mut after_cursor = state.subscription_hub.summary_source_change_cursor().await;
    if after_cursor == 0 {
        after_cursor = load_summary_source_change_checkpoint(
            &state.pool,
            SUMMARY_SOURCE_CHANGE_CHECKPOINT_SCOPE,
        )
        .await?;
        if after_cursor > 0 {
            state
                .subscription_hub
                .advance_summary_source_change_cursor(after_cursor)
                .await;
        }
    }
    // The reconciliation checkpoint describes an in-flight read, not a durable publication.
    // Never use its next cursor to skip source descriptors: a crash can happen after this
    // checkpoint write and before the immutable Projection swap. Replaying the bounded tail is
    // idempotent; skipping it would lose a committed terminal from the served overlay.
    let compaction_boundary = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM summary_source_change_compaction_proof \
         WHERE first_cursor <= ?1 \
           AND (retained_after_cursor > (?2 + 1) OR retained_after_cursor <= 0 \
                OR NOT json_valid(proof_json) \
                OR json_type(json_extract(proof_json, '$.retainedAfterCursor')) NOT IN ('integer', 'real') \
                OR CAST(json_extract(proof_json, '$.retainedAfterCursor') AS INTEGER) <> retained_after_cursor))",
    )
    .bind(
        i64::try_from(after_cursor.saturating_add(1))
            .context("source compaction boundary overflow")?,
    )
    .bind(i64::try_from(after_cursor).context("source compaction cursor overflow")?)
    .fetch_one(&state.pool)
    .await
    .context("load summary source compaction proof")?;
    if compaction_boundary != 0 {
        state
            .subscription_hub
            .record_summary_source_change_gap(after_cursor.saturating_add(1))
            .await;
        // The descriptor prefix may have been compacted, but the canonical invocation table is
        // still an authoritative recovery source. Reconstruct it only when the complete tail
        // fits the existing bounded replay budget; otherwise retain the broad fail-closed proof.
        return restore_compacted_summary_source_tail(state)
            .await
            .map(|restored| (restored, None));
    }
    let Some(base_projection) = state.subscription_hub.summary_projection().await else {
        return Ok((false, None));
    };
    let base_projection_revision = base_projection.revision();
    let base_generation_fence = base_projection.generation_fence();
    // The published base and durable target are different identities when new terminals
    // arrive. Reconcile against the base; publish only if this captured target is still current.
    let target_generation_fence = load_summary_projection_generation_fence(state).await?;
    let tail = load_summary_source_change_tail(
        &state.pool,
        after_cursor,
        SUMMARY_SOURCE_CHANGE_JOURNAL_MAX_ENTRIES,
    )
    .await?;
    let Some(last_cursor) = tail.last().map(|record| record.cursor) else {
        return Ok((false, None));
    };
    let mut expected_cursor = after_cursor.saturating_add(1);
    for record in &tail {
        if record.cursor != expected_cursor {
            state
                .subscription_hub
                .record_summary_source_change_gap(expected_cursor)
                .await;
            return Ok((false, None));
        }
        expected_cursor = record.cursor.saturating_add(1);
    }
    let ids = tail
        .iter()
        .flat_map(|record| record.descriptor.entries.iter().map(|entry| entry.row_id))
        .collect::<Vec<_>>();
    let mut rows_by_id = HashMap::<i64, ApiInvocation>::new();
    for chunk in ids.chunks(DASHBOARD_ACTIVITY_PREVIEW_ID_HYDRATION_CHUNK_SIZE) {
        if chunk.is_empty() {
            continue;
        }
        let mut query = build_invocation_select_query();
        query.push(" AND id IN (");
        {
            let mut separated = query.separated(", ");
            for id in chunk {
                separated.push_bind(*id);
            }
        }
        query.push(") ORDER BY id ASC");
        for row in query
            .build_query_as::<ApiInvocation>()
            .fetch_all(&state.pool)
            .await
            .context("summary source descriptor row reconstruction failed")?
        {
            rows_by_id.insert(row.id, row);
        }
    }

    let mut restored = false;
    let mut complete = true;
    let mut replayed_deltas = Vec::new();
    let target_terminal_watermark = tail
        .iter()
        .filter(|record| record.descriptor.source_kind == "terminal_batch")
        .map(|record| record.descriptor.source_revision)
        .max()
        .unwrap_or_default();
    let previous_epoch = load_summary_live_tail_reconciliation_checkpoint(&state.pool)
        .await?
        .map(|checkpoint| checkpoint.recovery_epoch)
        .unwrap_or_default();
    store_summary_live_tail_reconciliation_checkpoint(
        &state.pool,
        SummaryLiveTailReconciliationCheckpoint {
            format_version: 1,
            recovery_epoch: previous_epoch.saturating_add(1),
            base_projection_revision: base_projection_revision as i64,
            target_terminal_watermark,
            target_source_cursor: last_cursor,
            next_source_cursor: after_cursor,
            state: "reconciling".to_string(),
        },
    )
    .await?;
    for record in tail {
        if record.descriptor.source_kind != "terminal_batch" {
            // Archive and rollup descriptors are durable fence evidence for the independent
            // historical reconciler. They must not be interpreted as live invocation rows.
            state
                .subscription_hub
                .mark_summary_projection_historical_recovery_required()
                .await;
            continue;
        }
        for entry in record.descriptor.entries {
            let Some(row) = rows_by_id.get(&entry.row_id) else {
                complete = false;
                state
                    .subscription_hub
                    .record_summary_source_change_scoped_gap(
                        record.cursor,
                        entry.upstream_account_id,
                        entry.occurred_at.clone(),
                        Some(entry.row_id),
                        Some(entry.invoke_id.clone()),
                    )
                    .await;
                continue;
            };
            if row.invoke_id != entry.invoke_id
                || row.occurred_at != entry.occurred_at
                || row.upstream_account_id != entry.upstream_account_id
            {
                complete = false;
                state
                    .subscription_hub
                    .record_summary_source_change_scoped_gap(
                        record.cursor,
                        entry.upstream_account_id,
                        entry.occurred_at.clone(),
                        Some(entry.row_id),
                        Some(entry.invoke_id.clone()),
                    )
                    .await;
                continue;
            }
            let mut delta = persisted_dashboard_activity_terminal_delta(row);
            delta.persisted_row_id = Some(row.id);
            replayed_deltas.push(delta);
            restored = true;
        }
    }
    let reconciled = state
        .subscription_hub
        .reconcile_summary_delta_tail(
            &replayed_deltas,
            target_terminal_watermark,
            complete,
            base_projection_revision,
            base_generation_fence,
        )
        .await;
    if complete && reconciled {
        // The durable source cursor is intentionally committed by the caller only after the
        // immutable Projection freshness renewal succeeds.  Persisting it here would make a
        // crash between checkpoint and publication lose a committed tail on restart.
    } else {
        store_summary_live_tail_reconciliation_checkpoint(
            &state.pool,
            SummaryLiveTailReconciliationCheckpoint {
                format_version: 1,
                recovery_epoch: previous_epoch.saturating_add(1),
                base_projection_revision: base_projection_revision as i64,
                target_terminal_watermark,
                target_source_cursor: last_cursor,
                next_source_cursor: after_cursor,
                state: "deferred".to_string(),
            },
        )
        .await?;
    }
    Ok((restored || reconciled, Some(target_generation_fence)))
}

async fn restore_compacted_summary_source_tail(state: &AppState) -> Result<bool> {
    ensure_compacted_source_recovery(state).await?;
    // The source journal gap remains broad until a bounded day-window snapshot is published.
    // This deliberately leaves 30d/all-time on their existing exact-or-unavailable contract.
    Ok(false)
}

#[cfg(test)]
mod compacted_source_recovery_budget_tests {
    use sqlx::Execute;

    use super::*;

    fn candidate(id: i64, materialized_text_bytes: usize) -> CompactedSourceRecoveryCandidate {
        CompactedSourceRecoveryCandidate {
            id,
            invoke_id: format!("test-{id}"),
            occurred_at: "2026-10-22 12:00:00".to_string(),
            upstream_account_id: None,
            model: None,
            response_model: None,
            reasoning_effort: None,
            input_tokens: None,
            output_tokens: None,
            cache_input_tokens: None,
            total_tokens: None,
            cost: None,
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            status: Some("success".to_string()),
            error_message: None,
            failure_kind: None,
            failure_class: None,
            is_actionable: Some(0),
            estimated_bytes: i64::try_from(192 + materialized_text_bytes)
                .expect("test byte estimate"),
            error_message_bytes: 0,
        }
    }

    #[test]
    fn tuple_cursor_filter_separates_occurred_at_and_id_bindings() {
        let checkpoint = CompactedSourceRecoveryCheckpoint {
            scope: SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE.to_string(),
            format_version: 1,
            time_zone: SUMMARY_COMPACTED_SOURCE_RECOVERY_TIME_ZONE.to_string(),
            range_start: "2026-10-22 00:00:00".to_string(),
            range_end: "2026-10-29 00:00:00".to_string(),
            window_ranges_json: "[]".to_string(),
            cursor_occurred_at: Some("2026-10-22 12:00:00".to_string()),
            cursor_id: 100,
            target_live_high_watermark_id: 1_000,
            target_rollup_live_cursor: 0,
            target_account_rollup_live_cursor: None,
            target_manifest_high_watermark_id: None,
            target_coverage_revision: 0,
            target_account_coverage_revision: 0,
            target_terminal_sequence: 0,
            list_limit_max: 50,
            processed_rows: 100,
            state: "running".to_string(),
            last_error_code: None,
            last_error_message: None,
        };
        let start = Utc::now();
        let end = start + chrono::Duration::hours(1);
        let mut query =
            sqlx::QueryBuilder::<sqlx::Sqlite>::new("SELECT id FROM codex_invocations WHERE ");
        push_compacted_source_recovery_filter(
            &mut query,
            &checkpoint,
            &[("previous7d".to_string(), start, end)],
        )
        .expect("build tuple cursor recovery filter");
        let query = query.build();
        assert!(
            query.sql().contains("AND (occurred_at, id) > (?, ?) "),
            "tuple cursor must bind occurred_at and id as separate SQL values: {}",
            query.sql()
        );
    }

    #[test]
    fn candidate_budget_query_does_not_materialize_full_fields() {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new("");
        push_compacted_source_recovery_candidate_budget_columns(&mut query);
        let sql = query.build().sql().to_string();
        assert!(sql.starts_with("SELECT id, occurred_at"));
        assert!(sql.contains("AS estimated_bytes"));
        assert!(!sql.contains("AS response_model"));
        assert!(!sql.contains("AS reasoning_effort"));
    }

    #[test]
    fn candidate_materialization_query_reuses_snapshot_budget_metadata() {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new("");
        push_compacted_source_recovery_candidate_columns(&mut query);
        let sql = query.build().sql().to_string();
        assert!(sql.contains("NULL AS failure_kind"));
        assert!(sql.contains("0 AS estimated_bytes"));
        assert!(!sql.contains("= 'service_failure'"));
        assert!(!sql.contains("(192 + length"));
    }

    #[test]
    fn recovery_runtime_keeps_adaptive_page_size_across_scheduler_turns() {
        let mut runtime = CompactedSourceRecoveryRuntime::default();
        runtime.halve_page_size();
        runtime.mark_progress();

        assert_eq!(
            runtime.page_size,
            SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE / 2
        );
        assert!(runtime.take_retry_reset());

        runtime.mark_progress();
        assert!(runtime.take_retry_reset());
        assert_eq!(
            runtime.page_size,
            SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE / 2
        );
    }

    #[test]
    fn page_selection_is_capped_at_512_rows() {
        let page = (1..=513).map(|id| candidate(id, 0)).collect();
        let (selected, completed) = select_compacted_source_recovery_candidates(page)
            .expect("select bounded recovery candidates");
        assert_eq!(selected.len(), SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE);
        assert!(!completed);
    }

    #[test]
    fn page_selection_honors_adaptive_row_limit() {
        let page = CompactedSourceRecoveryCandidatePage {
            keys: vec![
                CompactedSourceRecoveryKey {
                    id: 1,
                    occurred_at: "2026-10-22 12:00:00".to_string(),
                },
                CompactedSourceRecoveryKey {
                    id: 2,
                    occurred_at: "2026-10-22 12:01:00".to_string(),
                },
            ],
            candidates: vec![candidate(1, 0), candidate(2, 0)],
        };
        let (selected, cursor, completed) =
            select_compacted_source_recovery_candidate_page(page, 1)
                .expect("select adaptive recovery candidates");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, 1);
        assert_eq!(cursor, Some(("2026-10-22 12:00:00".to_string(), 1)));
        assert!(!completed);
    }

    #[test]
    fn page_selection_keeps_live_boundary_at_cursor() {
        let page = CompactedSourceRecoveryCandidatePage {
            keys: vec![
                CompactedSourceRecoveryKey {
                    id: 1,
                    occurred_at: "2026-10-22 12:00:00".to_string(),
                },
                CompactedSourceRecoveryKey {
                    id: 2,
                    occurred_at: "2026-10-22 12:01:00".to_string(),
                },
            ],
            candidates: vec![candidate(1, 0)],
        };
        let (selected, cursor, completed) =
            select_compacted_source_recovery_candidate_page(page, 512)
                .expect("select recovery page around live boundary");
        assert_eq!(selected.len(), 1);
        assert_eq!(cursor, Some(("2026-10-22 12:00:00".to_string(), 1)));
        assert!(!completed);
    }

    #[test]
    fn page_selection_yields_before_the_8_mib_byte_budget() {
        let row_bytes = SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_BYTES / 2;
        let page = vec![candidate(1, row_bytes), candidate(2, row_bytes)];
        let (selected, completed) = select_compacted_source_recovery_candidates(page)
            .expect("select byte-bounded recovery candidates");
        assert_eq!(selected.len(), 1);
        assert!(!completed);
    }

    #[test]
    fn oversized_error_message_is_admitted_as_a_singleton() {
        let page = vec![
            CompactedSourceRecoveryCandidate {
                error_message_bytes: i64::try_from(
                    SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_BYTES + 1,
                )
                .expect("test error message byte estimate"),
                ..candidate(1, SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_BYTES + 1)
            },
            candidate(2, 1),
        ];
        let (selected, completed) = select_compacted_source_recovery_candidates(page)
            .expect("select oversized source singleton");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, 1);
        assert!(!completed);
    }

    #[test]
    fn oversized_materialized_source_row_fails_closed() {
        let page = vec![candidate(
            1,
            SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_BYTES + 1,
        )];
        let error = select_compacted_source_recovery_candidates(page)
            .expect_err("oversized materialized source row should fail closed");
        assert!(
            error
                .to_string()
                .contains("bounded recovery materialization budget")
        );
    }

    #[test]
    fn deferred_recovery_uses_bounded_exponential_backoff() {
        assert_eq!(
            summary_coverage_recovery_retry_delay(Duration::from_secs(10)),
            Duration::from_secs(20)
        );
        assert_eq!(
            summary_coverage_recovery_retry_delay(Duration::from_secs(20)),
            Duration::from_secs(40)
        );
        assert_eq!(
            summary_coverage_recovery_retry_delay(Duration::from_secs(40)),
            Duration::from_secs(60)
        );
        assert_eq!(
            summary_coverage_recovery_retry_delay(Duration::from_secs(60)),
            Duration::from_secs(60)
        );
        assert_eq!(
            summary_coverage_recovery_scheduler_delay(
                SummaryCoverageRecoveryNextTurn::Deferred,
                Duration::from_secs(20),
            ),
            Duration::from_secs(20)
        );
        assert_eq!(
            summary_coverage_recovery_scheduler_delay(
                SummaryCoverageRecoveryNextTurn::Immediate,
                Duration::from_secs(20),
            ),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn live_tail_advancement_does_not_invalidate_recovery_coverage() {
        let coverage = SummaryProjectionGenerationFence {
            live_high_watermark_id: 10,
            rollup_live_cursor: 20,
            account_rollup_live_cursor: Some(30),
            completed_manifest_high_watermark_id: Some(40),
            coverage_revision: 50,
            account_coverage_revision: 60,
            durable_terminal_sequence_watermark: 70,
        };
        let live_tail_advanced = SummaryProjectionGenerationFence {
            live_high_watermark_id: 11,
            rollup_live_cursor: 21,
            account_rollup_live_cursor: Some(31),
            completed_manifest_high_watermark_id: Some(40),
            coverage_revision: 50,
            account_coverage_revision: 60,
            durable_terminal_sequence_watermark: 71,
        };
        assert!(coverage.coverage_sources_match(live_tail_advanced));
        assert_ne!(coverage, live_tail_advanced);
    }

    #[test]
    fn recovery_snapshot_window_ranges_expire_at_the_next_reporting_day() {
        let before_midnight = DateTime::parse_from_rfc3339("2026-10-24T15:59:30Z")
            .expect("parse pre-midnight test time")
            .with_timezone(&Utc);
        let stored = to_string(&compacted_source_recovery_windows_at(before_midnight).unwrap())
            .expect("serialize recovery window ranges");

        assert!(
            compacted_source_recovery_window_ranges_match_at(&stored, before_midnight)
                .expect("match same-day recovery window ranges")
        );
        assert!(
            compacted_source_recovery_window_ranges_match_at(
                &stored,
                before_midnight + ChronoDuration::seconds(20),
            )
            .expect("match same-day recovery window ranges after time advances")
        );
        assert!(
            !compacted_source_recovery_window_ranges_match_at(
                &stored,
                before_midnight + ChronoDuration::minutes(2),
            )
            .expect("match next-day recovery window ranges")
        );
    }

    #[test]
    fn recovery_states_have_stable_unavailable_codes() {
        assert_eq!(
            SummaryCompactedSourceRecoveryState::Pending.unavailable_code(),
            super::super::summary_recovery::SUMMARY_RECOVERY_PENDING_CODE
        );
        assert_eq!(
            SummaryCompactedSourceRecoveryState::Deferred.unavailable_code(),
            super::super::summary_recovery::SUMMARY_RECOVERY_PENDING_CODE
        );
        assert_eq!(
            SummaryCompactedSourceRecoveryState::Failed.unavailable_code(),
            SUMMARY_RECOVERY_FAILED_CODE
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CompactedSourceRecoveryWindow {
    name: String,
    start: String,
    end: String,
}

#[derive(Debug, Clone, FromRow)]
struct CompactedSourceRecoveryCheckpoint {
    scope: String,
    format_version: i64,
    time_zone: String,
    range_start: String,
    range_end: String,
    window_ranges_json: String,
    cursor_occurred_at: Option<String>,
    cursor_id: i64,
    target_live_high_watermark_id: i64,
    target_rollup_live_cursor: i64,
    target_account_rollup_live_cursor: Option<i64>,
    target_manifest_high_watermark_id: Option<i64>,
    target_coverage_revision: i64,
    target_account_coverage_revision: i64,
    target_terminal_sequence: i64,
    list_limit_max: i64,
    processed_rows: i64,
    state: String,
    last_error_code: Option<String>,
    last_error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompactedSourceRecoveryIdentity {
    scope: String,
    format_version: i64,
    time_zone: String,
    range_start: String,
    range_end: String,
    window_ranges_json: String,
    target_live_high_watermark_id: i64,
    target_rollup_live_cursor: i64,
    target_account_rollup_live_cursor: Option<i64>,
    target_manifest_high_watermark_id: Option<i64>,
    target_coverage_revision: i64,
    target_account_coverage_revision: i64,
    target_terminal_sequence: i64,
    list_limit_max: i64,
}

impl From<&CompactedSourceRecoveryCheckpoint> for CompactedSourceRecoveryIdentity {
    fn from(checkpoint: &CompactedSourceRecoveryCheckpoint) -> Self {
        Self {
            scope: checkpoint.scope.clone(),
            format_version: checkpoint.format_version,
            time_zone: checkpoint.time_zone.clone(),
            range_start: checkpoint.range_start.clone(),
            range_end: checkpoint.range_end.clone(),
            window_ranges_json: checkpoint.window_ranges_json.clone(),
            target_live_high_watermark_id: checkpoint.target_live_high_watermark_id,
            target_rollup_live_cursor: checkpoint.target_rollup_live_cursor,
            target_account_rollup_live_cursor: checkpoint.target_account_rollup_live_cursor,
            target_manifest_high_watermark_id: checkpoint.target_manifest_high_watermark_id,
            target_coverage_revision: checkpoint.target_coverage_revision,
            target_account_coverage_revision: checkpoint.target_account_coverage_revision,
            target_terminal_sequence: checkpoint.target_terminal_sequence,
            list_limit_max: checkpoint.list_limit_max,
        }
    }
}

#[derive(Debug)]
struct CompactedSourceRecoveryRuntime {
    page_size: usize,
    identity: Option<CompactedSourceRecoveryIdentity>,
    retry_reset: bool,
}

impl Default for CompactedSourceRecoveryRuntime {
    fn default() -> Self {
        Self {
            page_size: SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE,
            identity: None,
            retry_reset: false,
        }
    }
}

impl CompactedSourceRecoveryRuntime {
    fn observe_checkpoint(&mut self, checkpoint: &CompactedSourceRecoveryCheckpoint) {
        let identity = CompactedSourceRecoveryIdentity::from(checkpoint);
        if self.identity.as_ref() != Some(&identity) {
            self.identity = Some(identity);
            self.page_size = SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE;
            self.retry_reset = true;
        }
    }

    fn halve_page_size(&mut self) {
        self.page_size = (self.page_size / 2).max(1);
    }

    fn mark_progress(&mut self) {
        self.retry_reset = true;
    }

    fn take_retry_reset(&mut self) -> bool {
        std::mem::take(&mut self.retry_reset)
    }
}

#[derive(Debug, Clone, FromRow)]
struct CompactedSourceRecoverySourceRow {
    id: i64,
    invoke_id: String,
    occurred_at: String,
    upstream_account_id: Option<i64>,
    model: Option<String>,
    response_model: Option<String>,
    reasoning_effort: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
    cost_input: Option<f64>,
    cost_cache_write: Option<f64>,
    cost_cache_read: Option<f64>,
    cost_output: Option<f64>,
    cost_reasoning: Option<f64>,
    status: Option<String>,
    error_message: Option<String>,
    failure_kind: Option<String>,
    failure_class: Option<String>,
    is_actionable: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
struct CompactedSourceRecoveryCandidate {
    id: i64,
    invoke_id: String,
    occurred_at: String,
    upstream_account_id: Option<i64>,
    model: Option<String>,
    response_model: Option<String>,
    reasoning_effort: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
    cost_input: Option<f64>,
    cost_cache_write: Option<f64>,
    cost_cache_read: Option<f64>,
    cost_output: Option<f64>,
    cost_reasoning: Option<f64>,
    status: Option<String>,
    error_message: Option<String>,
    failure_kind: Option<String>,
    failure_class: Option<String>,
    is_actionable: Option<i64>,
    estimated_bytes: i64,
    error_message_bytes: i64,
}

#[derive(Debug, Clone, FromRow)]
struct CompactedSourceRecoveryCandidateBudget {
    id: i64,
    occurred_at: String,
    status: Option<String>,
    failure_kind: Option<String>,
    failure_class: Option<String>,
    is_actionable: Option<i64>,
    estimated_bytes: i64,
    error_message_bytes: i64,
}

impl CompactedSourceRecoveryCandidateBudget {
    fn into_selection_candidate(self) -> Option<CompactedSourceRecoveryCandidate> {
        if matches!(
            self.status.as_deref(),
            None | Some("running") | Some("pending")
        ) {
            return None;
        }
        Some(CompactedSourceRecoveryCandidate {
            id: self.id,
            invoke_id: String::new(),
            occurred_at: self.occurred_at,
            upstream_account_id: None,
            model: None,
            response_model: None,
            reasoning_effort: None,
            input_tokens: None,
            output_tokens: None,
            cache_input_tokens: None,
            total_tokens: None,
            cost: None,
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            status: self.status,
            error_message: None,
            failure_kind: self.failure_kind,
            failure_class: self.failure_class,
            is_actionable: self.is_actionable,
            estimated_bytes: self.estimated_bytes,
            error_message_bytes: self.error_message_bytes,
        })
    }
}

impl CompactedSourceRecoveryCandidate {
    fn into_source_row(self) -> CompactedSourceRecoverySourceRow {
        CompactedSourceRecoverySourceRow {
            id: self.id,
            invoke_id: self.invoke_id,
            occurred_at: self.occurred_at,
            upstream_account_id: self.upstream_account_id,
            model: self.model,
            response_model: self.response_model,
            reasoning_effort: self.reasoning_effort,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cache_input_tokens: self.cache_input_tokens,
            total_tokens: self.total_tokens,
            cost: self.cost,
            cost_input: self.cost_input,
            cost_cache_write: self.cost_cache_write,
            cost_cache_read: self.cost_cache_read,
            cost_output: self.cost_output,
            cost_reasoning: self.cost_reasoning,
            status: self.status,
            error_message: self.error_message,
            failure_kind: self.failure_kind,
            failure_class: self.failure_class,
            is_actionable: self.is_actionable,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
struct CompactedSourceRecoveryKey {
    id: i64,
    occurred_at: String,
}

#[derive(Debug, Default)]
struct CompactedSourceRecoveryCandidatePage {
    keys: Vec<CompactedSourceRecoveryKey>,
    candidates: Vec<CompactedSourceRecoveryCandidate>,
}

#[derive(Debug, Default, Clone, Copy)]
struct CompactedSourceRecoveryAggregateDelta {
    total_count: i64,
    success_count: i64,
    failure_count: i64,
    total_tokens: i64,
    total_cost: f64,
    non_success_cost: f64,
    non_success_tokens: i64,
}

#[derive(Debug, Default, Clone, Copy)]
struct CompactedSourceRecoveryUsageDelta {
    cache_write_tokens: i64,
    cache_read_tokens: i64,
    output_tokens: i64,
    cost_input: f64,
    cost_cache_write: f64,
    cost_cache_read: f64,
    cost_output: f64,
    cost_reasoning: f64,
    cost_unknown: f64,
    has_cost: i64,
}

#[derive(Debug, Default)]
struct CompactedSourceRecoveryContributions {
    aggregates: HashMap<(String, i64), CompactedSourceRecoveryAggregateDelta>,
    usage: HashMap<(String, i64, String, String), CompactedSourceRecoveryUsageDelta>,
    records: Vec<(String, i64, i64, String, String)>,
    record_scopes: BTreeSet<(String, i64)>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum CompactedSourceRecoveryPageOutcome {
    Progress { completed: bool, deferred: bool },
    Reset,
    Stale,
}

const COMPACTED_SOURCE_RECOVERY_CHECKPOINT_SELECT: &str = "SELECT scope, format_version, time_zone, range_start, range_end, window_ranges_json, \
            cursor_occurred_at, cursor_id, target_live_high_watermark_id, \
            target_rollup_live_cursor, target_account_rollup_live_cursor, \
            target_manifest_high_watermark_id, target_coverage_revision, \
            target_account_coverage_revision, target_terminal_sequence, list_limit_max, \
            processed_rows, state, last_error_code, last_error_message \
       FROM summary_compacted_source_recovery_checkpoint \
      WHERE scope = ?1";

async fn load_compacted_source_recovery_checkpoint(
    pool: &sqlx::SqlitePool,
) -> Result<Option<CompactedSourceRecoveryCheckpoint>> {
    sqlx::query_as::<_, CompactedSourceRecoveryCheckpoint>(
        COMPACTED_SOURCE_RECOVERY_CHECKPOINT_SELECT,
    )
    .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
    .fetch_optional(pool)
    .await
    .context("load compacted source recovery checkpoint")
}

async fn load_compacted_source_recovery_checkpoint_tx(
    transaction: &mut sqlx::SqliteConnection,
) -> Result<Option<CompactedSourceRecoveryCheckpoint>> {
    sqlx::query_as::<_, CompactedSourceRecoveryCheckpoint>(
        COMPACTED_SOURCE_RECOVERY_CHECKPOINT_SELECT,
    )
    .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
    .fetch_optional(transaction)
    .await
    .context("load compacted source recovery checkpoint in transaction")
}

fn compacted_source_recovery_checkpoint_progress_matches(
    expected: &CompactedSourceRecoveryCheckpoint,
    current: &CompactedSourceRecoveryCheckpoint,
) -> bool {
    expected.scope == current.scope
        && expected.format_version == current.format_version
        && expected.time_zone == current.time_zone
        && expected.range_start == current.range_start
        && expected.range_end == current.range_end
        && expected.window_ranges_json == current.window_ranges_json
        && expected.cursor_occurred_at == current.cursor_occurred_at
        && expected.cursor_id == current.cursor_id
        && expected.target_live_high_watermark_id == current.target_live_high_watermark_id
        && expected.target_rollup_live_cursor == current.target_rollup_live_cursor
        && expected.target_account_rollup_live_cursor == current.target_account_rollup_live_cursor
        && expected.target_manifest_high_watermark_id == current.target_manifest_high_watermark_id
        && expected.target_coverage_revision == current.target_coverage_revision
        && expected.target_account_coverage_revision == current.target_account_coverage_revision
        && expected.target_terminal_sequence == current.target_terminal_sequence
        && expected.list_limit_max == current.list_limit_max
        && expected.processed_rows == current.processed_rows
}

fn compacted_source_recovery_fence(
    checkpoint: &CompactedSourceRecoveryCheckpoint,
) -> SummaryProjectionGenerationFence {
    SummaryProjectionGenerationFence {
        live_high_watermark_id: checkpoint.target_live_high_watermark_id,
        rollup_live_cursor: checkpoint.target_rollup_live_cursor,
        account_rollup_live_cursor: checkpoint.target_account_rollup_live_cursor,
        completed_manifest_high_watermark_id: checkpoint.target_manifest_high_watermark_id,
        coverage_revision: checkpoint.target_coverage_revision,
        account_coverage_revision: checkpoint.target_account_coverage_revision,
        durable_terminal_sequence_watermark: u64::try_from(
            checkpoint.target_terminal_sequence.max(0),
        )
        .unwrap_or_default(),
    }
}

fn compacted_source_recovery_windows_at(
    now: DateTime<Utc>,
) -> Result<Vec<CompactedSourceRecoveryWindow>> {
    let reporting_tz = SUMMARY_COMPACTED_SOURCE_RECOVERY_TIME_ZONE
        .parse::<Tz>()
        .context("parse compacted source recovery timezone")?;
    let today = crate::stats::resolve_range_window_at("today", reporting_tz, now)?;
    let yesterday = crate::stats::resolve_range_window_at("yesterday", reporting_tz, now)?;
    let (previous7d_start, previous7d_end) =
        crate::stats::previous_full_days_range_bounds(7, now, reporting_tz)
            .context("resolve previous7d compacted source recovery range")?;
    Ok(vec![
        CompactedSourceRecoveryWindow {
            name: "today".to_string(),
            start: today.start.to_rfc3339(),
            end: today.end.to_rfc3339(),
        },
        CompactedSourceRecoveryWindow {
            name: "yesterday".to_string(),
            start: yesterday.start.to_rfc3339(),
            end: yesterday.end.to_rfc3339(),
        },
        CompactedSourceRecoveryWindow {
            name: "previous7d".to_string(),
            start: previous7d_start.to_rfc3339(),
            end: previous7d_end.to_rfc3339(),
        },
    ])
}

fn parse_compacted_source_recovery_windows(
    json: &str,
) -> Result<Vec<(String, DateTime<Utc>, DateTime<Utc>)>> {
    from_str::<Vec<CompactedSourceRecoveryWindow>>(json)
        .context("parse compacted source recovery window boundaries")?
        .into_iter()
        .map(|window| {
            let start = DateTime::parse_from_rfc3339(&window.start)
                .with_context(|| format!("parse {} recovery start", window.name))?
                .with_timezone(&Utc);
            let end = DateTime::parse_from_rfc3339(&window.end)
                .with_context(|| format!("parse {} recovery end", window.name))?
                .with_timezone(&Utc);
            if start >= end {
                return Err(anyhow!(
                    "invalid {} compacted source recovery range",
                    window.name
                ));
            }
            Ok((window.name, start, end))
        })
        .collect()
}

fn compacted_source_recovery_windows_match(
    current: &[CompactedSourceRecoveryWindow],
    stored: &[CompactedSourceRecoveryWindow],
) -> bool {
    current.len() == stored.len()
        && current.iter().zip(stored).all(|(current, stored)| {
            current.name == stored.name
                && current.start == stored.start
                && (current.name == "today" || current.end == stored.end)
        })
}

pub(crate) fn compacted_source_recovery_window_ranges_match_at(
    stored_json: &str,
    now: DateTime<Utc>,
) -> Result<bool> {
    let stored = from_str::<Vec<CompactedSourceRecoveryWindow>>(stored_json)
        .context("parse stored compacted source recovery window boundaries")?;
    Ok(compacted_source_recovery_windows_match(
        &compacted_source_recovery_windows_at(now)?,
        &stored,
    ))
}

fn compacted_source_recovery_checkpoint_matches(
    checkpoint: &CompactedSourceRecoveryCheckpoint,
    target_fence: SummaryProjectionGenerationFence,
    current_windows: &[CompactedSourceRecoveryWindow],
    list_limit_max: usize,
) -> Result<bool> {
    let stored_windows = parse_compacted_source_recovery_windows(&checkpoint.window_ranges_json)?
        .into_iter()
        .map(|(name, start, end)| CompactedSourceRecoveryWindow {
            name,
            start: start.to_rfc3339(),
            end: end.to_rfc3339(),
        })
        .collect::<Vec<_>>();
    Ok(checkpoint.format_version == 1
        && checkpoint.scope == SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE
        && checkpoint.time_zone == SUMMARY_COMPACTED_SOURCE_RECOVERY_TIME_ZONE
        && checkpoint.range_start
            == current_windows
                .iter()
                .filter_map(|window| {
                    DateTime::parse_from_rfc3339(&window.start)
                        .ok()
                        .map(|value| {
                            crate::stats::db_occurred_at_lower_bound(value.with_timezone(&Utc))
                        })
                })
                .min()
                .unwrap_or_default()
        && checkpoint.list_limit_max == i64::try_from(list_limit_max).unwrap_or(i64::MAX)
        && compacted_source_recovery_windows_match(current_windows, &stored_windows)
        && compacted_source_recovery_fence(checkpoint).coverage_sources_match(target_fence))
}

async fn ensure_compacted_source_recovery(state: &AppState) -> Result<()> {
    let _ = ensure_compacted_source_recovery_with_force_bounded(
        state,
        false,
        SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET,
    )
    .await?;
    Ok(())
}

async fn ensure_compacted_source_recovery_with_force_bounded(
    state: &AppState,
    force_reset: bool,
    budget: Duration,
) -> Result<bool> {
    let now = Utc::now();
    let current_windows = compacted_source_recovery_windows_at(now)?;
    let list_limit_max = state.config.list_limit_max.max(1);
    let range_start = current_windows
        .iter()
        .filter_map(|window| {
            DateTime::parse_from_rfc3339(&window.start)
                .ok()
                .map(|value| crate::stats::db_occurred_at_lower_bound(value.with_timezone(&Utc)))
        })
        .min()
        .unwrap_or_default();
    let range_end = current_windows
        .iter()
        .filter_map(|window| {
            DateTime::parse_from_rfc3339(&window.end)
                .ok()
                .map(|value| crate::db_occurred_at_upper_bound(value.with_timezone(&Utc)))
        })
        .max()
        .unwrap_or_default();
    let window_ranges_json = to_string(&current_windows)
        .context("serialize compacted source recovery window boundaries")?;
    let Some((mut connection, interrupted, deadline)) = open_compacted_source_recovery_connection(
        state,
        budget,
        "open compacted source recovery reset connection",
    )
    .await?
    else {
        state
            .subscription_hub
            .set_summary_compacted_source_recovery_state(
                SummaryCompactedSourceRecoveryState::Deferred,
            )
            .await;
        return Ok(false);
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    let current_fence = match tokio::time::timeout(
        remaining,
        load_summary_projection_generation_fence_tx(state, &mut connection),
    )
    .await
    {
        Ok(Ok(fence)) => fence,
        Ok(Err(error)) => {
            let contention = is_sqlite_contention(&error);
            let _ = close_compacted_source_recovery_connection_bounded(connection, deadline).await;
            if contention || interrupted.load(Ordering::Acquire) || Instant::now() >= deadline {
                state
                    .subscription_hub
                    .set_summary_compacted_source_recovery_state(
                        SummaryCompactedSourceRecoveryState::Deferred,
                    )
                    .await;
                return Ok(false);
            }
            return Err(error.context("load compacted source recovery reset generation fence"));
        }
        Err(_) => {
            let _ = close_compacted_source_recovery_connection_bounded(connection, deadline).await;
            state
                .subscription_hub
                .set_summary_compacted_source_recovery_state(
                    SummaryCompactedSourceRecoveryState::Deferred,
                )
                .await;
            return Ok(false);
        }
    };
    let target_terminal_sequence = i64::try_from(current_fence.durable_terminal_sequence_watermark)
        .context("compacted source recovery terminal sequence overflow")?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    let existing = match tokio::time::timeout(
        remaining,
        load_compacted_source_recovery_checkpoint_tx(&mut connection),
    )
    .await
    {
        Ok(Ok(checkpoint)) => checkpoint,
        Ok(Err(error)) => {
            let contention = is_sqlite_contention(&error);
            let _ = close_compacted_source_recovery_connection_bounded(connection, deadline).await;
            if contention || interrupted.load(Ordering::Acquire) || Instant::now() >= deadline {
                state
                    .subscription_hub
                    .set_summary_compacted_source_recovery_state(
                        SummaryCompactedSourceRecoveryState::Deferred,
                    )
                    .await;
                return Ok(false);
            }
            return Err(error.context("load compacted source recovery reset checkpoint"));
        }
        Err(_) => {
            let _ = close_compacted_source_recovery_connection_bounded(connection, deadline).await;
            state
                .subscription_hub
                .set_summary_compacted_source_recovery_state(
                    SummaryCompactedSourceRecoveryState::Deferred,
                )
                .await;
            return Ok(false);
        }
    };
    if !force_reset
        && let Some(checkpoint) = existing.as_ref()
        && compacted_source_recovery_checkpoint_matches(
            checkpoint,
            current_fence,
            &current_windows,
            list_limit_max,
        )?
    {
        let closed =
            close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
        if !closed {
            state
                .subscription_hub
                .set_summary_compacted_source_recovery_state(
                    SummaryCompactedSourceRecoveryState::Deferred,
                )
                .await;
            return Ok(false);
        }
        state
            .subscription_hub
            .set_summary_compacted_source_recovery_state(
                SummaryCompactedSourceRecoveryState::from_storage(&checkpoint.state),
            )
            .await;
        return Ok(true);
    }

    let read_connection_closed =
        close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
    if !read_connection_closed {
        state
            .subscription_hub
            .set_summary_compacted_source_recovery_state(
                SummaryCompactedSourceRecoveryState::Deferred,
            )
            .await;
        return Ok(false);
    }
    let reset_budget = SUMMARY_COMPACTED_SOURCE_RECOVERY_TRANSACTION_BUDGET
        .min(deadline.saturating_duration_since(Instant::now()));
    let Some((mut connection, interrupted, deadline)) = open_compacted_source_recovery_connection(
        state,
        reset_budget,
        "open compacted source recovery reset transaction connection",
    )
    .await?
    else {
        state
            .subscription_hub
            .set_summary_compacted_source_recovery_state(
                SummaryCompactedSourceRecoveryState::Deferred,
            )
            .await;
        return Ok(false);
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        let _ = close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
        state
            .subscription_hub
            .set_summary_compacted_source_recovery_state(
                SummaryCompactedSourceRecoveryState::Deferred,
            )
            .await;
        return Ok(false);
    }
    let begin_result = tokio::time::timeout(
        remaining,
        sqlx::query("BEGIN IMMEDIATE").execute(&mut connection),
    )
    .await;
    match begin_result {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            let error = anyhow!(error).context("begin compacted source recovery reset");
            let contention = is_sqlite_contention(&error);
            let _ = close_compacted_source_recovery_connection_bounded(connection, deadline).await;
            if contention || interrupted.load(Ordering::Acquire) || Instant::now() >= deadline {
                state
                    .subscription_hub
                    .set_summary_compacted_source_recovery_state(
                        SummaryCompactedSourceRecoveryState::Deferred,
                    )
                    .await;
                return Ok(false);
            }
            return Err(error);
        }
        Err(_) => {
            let _ = close_compacted_source_recovery_connection_bounded(connection, deadline).await;
            state
                .subscription_hub
                .set_summary_compacted_source_recovery_state(
                    SummaryCompactedSourceRecoveryState::Deferred,
                )
                .await;
            return Ok(false);
        }
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    let reset_result = tokio::time::timeout(remaining, async {
        for table in [
            "summary_compacted_source_recovery_aggregate",
            "summary_compacted_source_recovery_usage",
            "summary_compacted_source_recovery_record",
        ] {
            sqlx::query(&format!("DELETE FROM {table} WHERE scope = ?1"))
                .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
                .execute(&mut connection)
                .await
                .with_context(|| format!("clear compacted source recovery {table}"))?;
        }
        sqlx::query(
            "INSERT INTO summary_compacted_source_recovery_checkpoint (\
                scope, format_version, time_zone, range_start, range_end, window_ranges_json,\
                cursor_occurred_at, cursor_id, target_live_high_watermark_id, target_rollup_live_cursor,\
                target_account_rollup_live_cursor, target_manifest_high_watermark_id,\
                target_coverage_revision, target_account_coverage_revision, target_terminal_sequence,\
                list_limit_max, processed_rows, state, last_error_code, last_error_message, updated_at\
            ) VALUES (?1, 1, ?2, ?3, ?4, ?5, NULL, 0, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0, 'pending', NULL, NULL, datetime('now')) \
            ON CONFLICT(scope) DO UPDATE SET \
                format_version = excluded.format_version, time_zone = excluded.time_zone,\
                range_start = excluded.range_start, range_end = excluded.range_end,\
                window_ranges_json = excluded.window_ranges_json, cursor_occurred_at = NULL,\
                cursor_id = 0, target_live_high_watermark_id = excluded.target_live_high_watermark_id,\
                target_rollup_live_cursor = excluded.target_rollup_live_cursor,\
                target_account_rollup_live_cursor = excluded.target_account_rollup_live_cursor,\
                target_manifest_high_watermark_id = excluded.target_manifest_high_watermark_id,\
                target_coverage_revision = excluded.target_coverage_revision,\
                target_account_coverage_revision = excluded.target_account_coverage_revision,\
                target_terminal_sequence = excluded.target_terminal_sequence,\
                list_limit_max = excluded.list_limit_max, processed_rows = 0, state = 'pending',\
                last_error_code = NULL, last_error_message = NULL, updated_at = datetime('now')",
        )
        .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
        .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_TIME_ZONE)
        .bind(range_start)
        .bind(range_end)
        .bind(window_ranges_json)
        .bind(current_fence.live_high_watermark_id)
        .bind(current_fence.rollup_live_cursor)
        .bind(current_fence.account_rollup_live_cursor)
        .bind(current_fence.completed_manifest_high_watermark_id)
        .bind(current_fence.coverage_revision)
        .bind(current_fence.account_coverage_revision)
        .bind(target_terminal_sequence)
        .bind(i64::try_from(list_limit_max).context("compacted source recovery list limit overflow")?)
        .execute(&mut connection)
        .await
        .context("reset compacted source recovery checkpoint")?;
        Ok::<(), anyhow::Error>(())
    })
    .await;
    match reset_result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let budget_expired = interrupted.load(Ordering::Acquire) || Instant::now() >= deadline;
            let contention = is_sqlite_contention(&error);
            let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                connection, deadline,
            )
            .await?;
            if budget_expired || contention {
                state
                    .subscription_hub
                    .set_summary_compacted_source_recovery_state(
                        SummaryCompactedSourceRecoveryState::Deferred,
                    )
                    .await;
                return Ok(false);
            }
            return Err(error);
        }
        Err(_) => {
            let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                connection, deadline,
            )
            .await?;
            state
                .subscription_hub
                .set_summary_compacted_source_recovery_state(
                    SummaryCompactedSourceRecoveryState::Deferred,
                )
                .await;
            return Ok(false);
        }
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    let commit_result =
        tokio::time::timeout(remaining, sqlx::query("COMMIT").execute(&mut connection)).await;
    match commit_result {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            let error = anyhow!(error).context("commit compacted source recovery reset");
            let budget_expired = interrupted.load(Ordering::Acquire) || Instant::now() >= deadline;
            let contention = is_sqlite_contention(&error);
            let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                connection, deadline,
            )
            .await?;
            if budget_expired || contention {
                state
                    .subscription_hub
                    .set_summary_compacted_source_recovery_state(
                        SummaryCompactedSourceRecoveryState::Deferred,
                    )
                    .await;
                return Ok(false);
            }
            return Err(error);
        }
        Err(_) => {
            let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                connection, deadline,
            )
            .await?;
            state
                .subscription_hub
                .set_summary_compacted_source_recovery_state(
                    SummaryCompactedSourceRecoveryState::Deferred,
                )
                .await;
            return Ok(false);
        }
    }
    let closed = close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
    state
        .subscription_hub
        .clear_summary_dashboard_recovery()
        .await;
    state
        .subscription_hub
        .set_summary_compacted_source_recovery_state(SummaryCompactedSourceRecoveryState::Pending)
        .await;
    Ok(closed)
}

fn push_compacted_source_recovery_filter(
    query: &mut sqlx::QueryBuilder<'_, sqlx::Sqlite>,
    checkpoint: &CompactedSourceRecoveryCheckpoint,
    windows: &[(String, DateTime<Utc>, DateTime<Utc>)],
) -> Result<()> {
    let start = windows
        .iter()
        .map(|(_, start, _)| *start)
        .min()
        .ok_or_else(|| anyhow!("compacted source recovery has no scan windows"))?;
    let end = windows
        .iter()
        .map(|(_, _, end)| *end)
        .max()
        .ok_or_else(|| anyhow!("compacted source recovery has no scan windows"))?;
    query
        .push("occurred_at >= ")
        .push_bind(crate::stats::db_occurred_at_lower_bound(start))
        .push(" AND occurred_at < ")
        .push_bind(crate::db_occurred_at_upper_bound(end))
        .push(" AND id <= ")
        .push_bind(checkpoint.target_live_high_watermark_id)
        .push(" ");
    if let Some(cursor_occurred_at) = checkpoint.cursor_occurred_at.as_deref() {
        query
            .push("AND (occurred_at, id) > (")
            .push_bind(cursor_occurred_at.to_string())
            .push(", ")
            .push_bind(checkpoint.cursor_id)
            .push(") ");
    } else if checkpoint.cursor_id > 0 {
        query
            .push("AND id > ")
            .push_bind(checkpoint.cursor_id)
            .push(" ");
    }
    Ok(())
}

fn push_compacted_source_recovery_text_length(
    query: &mut sqlx::QueryBuilder<'_, sqlx::Sqlite>,
    expression: &str,
) {
    query
        .push(" + length(CAST(COALESCE(")
        .push(expression)
        .push(", '') AS BLOB))");
}

fn push_compacted_source_recovery_candidate_columns(
    query: &mut sqlx::QueryBuilder<'_, sqlx::Sqlite>,
) {
    query
        .push("SELECT id, invoke_id, occurred_at, ")
        .push(INVOCATION_UPSTREAM_ACCOUNT_ID_SQL)
        .push(" AS upstream_account_id, model, ")
        .push(INVOCATION_RESPONSE_MODEL_SQL)
        .push(" AS response_model, ")
        .push(INVOCATION_REASONING_EFFORT_SQL)
        .push(
            " AS reasoning_effort, input_tokens, output_tokens, cache_input_tokens, \
                total_tokens, cost, cost_input, cost_cache_write, cost_cache_read, cost_output, \
                cost_reasoning, status, CASE WHEN length(CAST(COALESCE(error_message, '') AS BLOB)) > ",
        )
        .push_bind(
            i64::try_from(SUMMARY_COMPACTED_SOURCE_RECOVERY_ERROR_MESSAGE_BYTES).unwrap_or(65_536),
        )
        .push(
            " THEN '__summary_recovery_oversized_error__' ELSE error_message END AS error_message, ",
        )
        .push(
            "NULL AS failure_kind, NULL AS failure_class, NULL AS is_actionable, \
             0 AS estimated_bytes, 0 AS error_message_bytes FROM codex_invocations WHERE ",
        );
}

fn push_compacted_source_recovery_candidate_budget_columns(
    query: &mut sqlx::QueryBuilder<'_, sqlx::Sqlite>,
) {
    query
        .push("SELECT id, occurred_at, ")
        .push(INVOCATION_STATUS_NORMALIZED_SQL)
        .push(" AS status, ")
        .push(INVOCATION_FAILURE_KIND_SQL)
        .push(" AS failure_kind, ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" AS failure_class, CASE WHEN ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" = 'service_failure' THEN 1 ELSE 0 END AS is_actionable, (192");
    for expression in [
        "invoke_id",
        "occurred_at",
        "model",
        INVOCATION_RESPONSE_MODEL_SQL,
        INVOCATION_REASONING_EFFORT_SQL,
        "status",
        "error_message",
        INVOCATION_FAILURE_KIND_SQL,
        INVOCATION_RESOLVED_FAILURE_CLASS_SQL,
    ] {
        push_compacted_source_recovery_text_length(query, expression);
    }
    query.push(
        ") AS estimated_bytes, length(CAST(COALESCE(error_message, '') AS BLOB)) \
         AS error_message_bytes FROM codex_invocations WHERE ",
    );
}

async fn open_compacted_source_recovery_connection(
    state: &AppState,
    budget: Duration,
    context: &'static str,
) -> Result<Option<(SqliteConnection, Arc<AtomicBool>, Instant)>> {
    if budget.is_zero() {
        return Ok(None);
    }
    let deadline = Instant::now() + budget;
    let options = state
        .pool
        .connect_options()
        .as_ref()
        .clone()
        .busy_timeout(budget);
    let mut connection =
        match tokio::time::timeout(budget, SqliteConnection::connect_with(&options)).await {
            Ok(Ok(connection)) => connection,
            Ok(Err(error)) => {
                let error = anyhow!(error).context(context);
                if is_sqlite_contention(&error) {
                    return Ok(None);
                }
                return Err(error);
            }
            Err(_) => return Ok(None),
        };
    let interrupted = Arc::new(AtomicBool::new(false));
    {
        let interrupted = interrupted.clone();
        let mut handle = connection
            .lock_handle()
            .await
            .context("acquire compacted source recovery progress handle")?;
        handle.set_progress_handler(1_000, move || {
            let keep_running = Instant::now() < deadline;
            if !keep_running {
                interrupted.store(true, Ordering::Release);
            }
            keep_running
        });
    }
    if Instant::now() >= deadline {
        close_compacted_source_recovery_connection(connection).await?;
        return Ok(None);
    }
    Ok(Some((connection, interrupted, deadline)))
}

async fn load_compacted_source_recovery_candidates(
    state: &AppState,
    checkpoint: &CompactedSourceRecoveryCheckpoint,
    windows: &[(String, DateTime<Utc>, DateTime<Utc>)],
    page_size: usize,
    budget: Duration,
) -> Result<Option<CompactedSourceRecoveryCandidatePage>> {
    let started = Instant::now();
    let mut key_query = sqlx::QueryBuilder::<sqlx::Sqlite>::new("");
    push_compacted_source_recovery_key_page_query(&mut key_query, checkpoint, windows, page_size)?;
    let Some((mut connection, interrupted, deadline)) = open_compacted_source_recovery_connection(
        state,
        budget.saturating_sub(started.elapsed()),
        "open compacted source recovery candidate connection",
    )
    .await?
    else {
        return Ok(None);
    };
    let remaining = budget.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        let _ = close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
        return Ok(None);
    }
    let result = tokio::time::timeout(
        remaining,
        key_query
            .build_query_as::<CompactedSourceRecoveryKey>()
            .fetch_all(&mut connection),
    )
    .await;
    let keys = match result {
        Ok(Ok(rows)) => rows,
        Ok(Err(error)) => {
            let error = anyhow!(error).context("load bounded compacted source recovery candidates");
            if interrupted.load(Ordering::Acquire)
                || Instant::now() >= deadline
                || is_sqlite_contention(&error)
            {
                let _ = close_compacted_source_recovery_connection_bounded(connection, deadline)
                    .await?;
                return Ok(None);
            } else {
                let _ = close_compacted_source_recovery_connection_bounded(connection, deadline)
                    .await?;
                return Err(error);
            }
        }
        Err(_) => {
            let _ =
                close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
            return Ok(None);
        }
    };
    if keys.is_empty() {
        if !close_compacted_source_recovery_connection_bounded(connection, deadline).await? {
            return Ok(None);
        }
        return Ok(Some(CompactedSourceRecoveryCandidatePage::default()));
    }

    let remaining = budget.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        let _ = close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
        return Ok(None);
    }
    let begin_result =
        tokio::time::timeout(remaining, sqlx::query("BEGIN").execute(&mut connection)).await;
    match begin_result {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            let error = anyhow!(error).context("begin compacted source recovery read snapshot");
            let _ =
                close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
            if is_sqlite_contention(&error) {
                return Ok(None);
            }
            return Err(error);
        }
        Err(_) => {
            let _ =
                close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
            return Ok(None);
        }
    }
    let remaining = budget.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        let _ =
            rollback_and_close_compacted_source_recovery_connection_bounded(connection, deadline)
                .await?;
        return Ok(None);
    }
    let mut budget_query = sqlx::QueryBuilder::<sqlx::Sqlite>::new("");
    push_compacted_source_recovery_candidate_budget_columns(&mut budget_query);
    budget_query.push("id IN (");
    {
        let mut separated = budget_query.separated(", ");
        for key in &keys {
            separated.push_bind(key.id);
        }
        separated.push_unseparated(")");
    }
    budget_query
        .push(" AND id <= ")
        .push_bind(checkpoint.target_live_high_watermark_id);
    let budget_result = tokio::time::timeout(
        remaining,
        budget_query
            .build_query_as::<CompactedSourceRecoveryCandidateBudget>()
            .fetch_all(&mut connection),
    )
    .await;
    let budgets = match budget_result {
        Ok(Ok(budgets)) => budgets,
        Ok(Err(error)) => {
            let error =
                anyhow!(error).context("load bounded compacted source recovery byte estimates");
            if interrupted.load(Ordering::Acquire)
                || Instant::now() >= deadline
                || is_sqlite_contention(&error)
            {
                let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                    connection, deadline,
                )
                .await?;
                return Ok(None);
            } else {
                let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                    connection, deadline,
                )
                .await?;
                return Err(error);
            }
        }
        Err(_) => {
            let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                connection, deadline,
            )
            .await?;
            return Ok(None);
        }
    };
    let budget_candidates = budgets
        .into_iter()
        .filter_map(CompactedSourceRecoveryCandidateBudget::into_selection_candidate)
        .collect::<Vec<_>>();
    let budget_page = CompactedSourceRecoveryCandidatePage {
        keys: keys.clone(),
        candidates: budget_candidates,
    };
    let (selected_budget_candidates, _, _) =
        match select_compacted_source_recovery_candidate_page(budget_page, page_size) {
            Ok(selection) => selection,
            Err(error) => {
                let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                    connection, deadline,
                )
                .await?;
                return Err(error);
            }
        };
    let selected_budget_by_id = selected_budget_candidates
        .iter()
        .map(|candidate| (candidate.id, candidate.clone()))
        .collect::<HashMap<_, _>>();
    let selected_ids = selected_budget_candidates
        .iter()
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();
    if selected_ids.is_empty() {
        if !rollback_and_close_compacted_source_recovery_connection_bounded(connection, deadline)
            .await?
        {
            return Ok(None);
        }
        return Ok(Some(CompactedSourceRecoveryCandidatePage {
            keys,
            candidates: Vec::new(),
        }));
    }

    let remaining = budget.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        let _ =
            rollback_and_close_compacted_source_recovery_connection_bounded(connection, deadline)
                .await?;
        return Ok(None);
    }
    let mut candidate_query = sqlx::QueryBuilder::<sqlx::Sqlite>::new("");
    push_compacted_source_recovery_candidate_columns(&mut candidate_query);
    candidate_query.push("id IN (");
    {
        let mut separated = candidate_query.separated(", ");
        for id in selected_ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
    }
    candidate_query
        .push(" AND id <= ")
        .push_bind(checkpoint.target_live_high_watermark_id);
    let candidate_result = tokio::time::timeout(
        remaining,
        candidate_query
            .build_query_as::<CompactedSourceRecoveryCandidate>()
            .fetch_all(&mut connection),
    )
    .await;
    let closed =
        rollback_and_close_compacted_source_recovery_connection_bounded(connection, deadline)
            .await?;
    if !closed {
        return Ok(None);
    }
    match candidate_result {
        Ok(Ok(candidates)) => {
            let candidates = candidates
                .into_iter()
                .filter_map(|mut candidate| {
                    let budget = selected_budget_by_id.get(&candidate.id)?;
                    candidate.failure_kind = budget.failure_kind.clone();
                    candidate.failure_class = budget.failure_class.clone();
                    candidate.is_actionable = budget.is_actionable;
                    candidate.estimated_bytes = budget.estimated_bytes;
                    candidate.error_message_bytes = budget.error_message_bytes;
                    Some(candidate)
                })
                .collect();
            Ok(Some(CompactedSourceRecoveryCandidatePage {
                keys,
                candidates,
            }))
        }
        Ok(Err(error)) => {
            let error =
                anyhow!(error).context("load bounded compacted source recovery materialized page");
            if interrupted.load(Ordering::Acquire)
                || Instant::now() >= deadline
                || is_sqlite_contention(&error)
            {
                Ok(None)
            } else {
                Err(error)
            }
        }
        Err(_) => Ok(None),
    }
}

fn push_compacted_source_recovery_key_page_query(
    query: &mut sqlx::QueryBuilder<'_, sqlx::Sqlite>,
    checkpoint: &CompactedSourceRecoveryCheckpoint,
    windows: &[(String, DateTime<Utc>, DateTime<Utc>)],
    page_size: usize,
) -> Result<()> {
    query.push(
        "SELECT id, occurred_at FROM codex_invocations INDEXED BY \
         idx_codex_invocations_occurred_at_id WHERE ",
    );
    push_compacted_source_recovery_filter(query, checkpoint, windows)?;
    query
        .push(" ORDER BY occurred_at ASC, id ASC LIMIT ")
        .push_bind(i64::try_from(page_size.max(1)).unwrap_or(1));
    Ok(())
}

fn add_compacted_source_row_contributions(
    contributions: &mut CompactedSourceRecoveryContributions,
    row: &CompactedSourceRecoverySourceRow,
    windows: &[(String, DateTime<Utc>, DateTime<Utc>)],
) -> Result<()> {
    let occurred_at = crate::stats::parse_to_utc_datetime(&row.occurred_at)
        .ok_or_else(|| anyhow!("invalid canonical occurred_at for row {}", row.id))?;
    let classification = resolve_failure_classification(
        row.status.as_deref(),
        row.error_message.as_deref(),
        row.failure_kind.as_deref(),
        row.failure_class.as_deref(),
        row.is_actionable,
    );
    let status = normalized_runtime_text(row.status.as_deref());
    let success = matches!(
        status.as_str(),
        "success" | "completed" | INVOCATION_STATUS_WARNING_SUCCESS
    ) || (status == "http_200"
        && normalized_runtime_text(row.error_message.as_deref()).is_empty());
    let failure = matches!(
        classification.failure_class,
        FailureClass::ServiceFailure | FailureClass::ClientFailure | FailureClass::ClientAbort
    );
    let total_tokens = row.total_tokens.unwrap_or_default();
    let total_cost = row.cost.unwrap_or_default();
    let cache_read_tokens = row.cache_input_tokens.unwrap_or_default().max(0);
    let cache_write_tokens = row
        .input_tokens
        .unwrap_or_default()
        .max(0)
        .saturating_sub(cache_read_tokens);
    let output_tokens = row.output_tokens.unwrap_or_default().max(0);
    let model = row
        .response_model
        .as_deref()
        .or(row.model.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_string();
    let reasoning_effort = row
        .reasoning_effort
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("")
        .to_string();
    let cost_complete = row.cost_input.is_some()
        && row.cost_cache_write.is_some()
        && row.cost_cache_read.is_some()
        && row.cost_output.is_some()
        && row.cost_reasoning.is_some();
    let usage = CompactedSourceRecoveryUsageDelta {
        cache_write_tokens,
        cache_read_tokens,
        output_tokens,
        cost_input: row.cost_input.unwrap_or_default(),
        cost_cache_write: row.cost_cache_write.unwrap_or_default(),
        cost_cache_read: row.cost_cache_read.unwrap_or_default(),
        cost_output: row.cost_output.unwrap_or_default(),
        cost_reasoning: row.cost_reasoning.unwrap_or_default(),
        cost_unknown: if row.cost.is_some() && !cost_complete {
            total_cost
        } else {
            0.0
        },
        has_cost: i64::from(row.cost.is_some()),
    };
    for (window_name, start, end) in windows {
        if occurred_at < *start || occurred_at >= *end {
            continue;
        }
        let aggregate = CompactedSourceRecoveryAggregateDelta {
            total_count: 1,
            success_count: i64::from(success && classification.failure_class == FailureClass::None),
            failure_count: i64::from(failure),
            total_tokens,
            total_cost,
            non_success_cost: if failure { total_cost } else { 0.0 },
            non_success_tokens: if failure { total_tokens.max(0) } else { 0 },
        };
        let account_scopes =
            std::iter::once(0).chain(row.upstream_account_id.filter(|account| *account > 0));
        for account_scope in account_scopes {
            let aggregate_entry = contributions
                .aggregates
                .entry((window_name.clone(), account_scope))
                .or_default();
            aggregate_entry.total_count += aggregate.total_count;
            aggregate_entry.success_count += aggregate.success_count;
            aggregate_entry.failure_count += aggregate.failure_count;
            aggregate_entry.total_tokens += aggregate.total_tokens;
            aggregate_entry.total_cost += aggregate.total_cost;
            aggregate_entry.non_success_cost += aggregate.non_success_cost;
            aggregate_entry.non_success_tokens += aggregate.non_success_tokens;

            let usage_entry = contributions
                .usage
                .entry((
                    window_name.clone(),
                    account_scope,
                    model.clone(),
                    reasoning_effort.clone(),
                ))
                .or_default();
            usage_entry.cache_write_tokens += usage.cache_write_tokens;
            usage_entry.cache_read_tokens += usage.cache_read_tokens;
            usage_entry.output_tokens += usage.output_tokens;
            usage_entry.cost_input += usage.cost_input;
            usage_entry.cost_cache_write += usage.cost_cache_write;
            usage_entry.cost_cache_read += usage.cost_cache_read;
            usage_entry.cost_output += usage.cost_output;
            usage_entry.cost_reasoning += usage.cost_reasoning;
            usage_entry.cost_unknown += usage.cost_unknown;
            usage_entry.has_cost = i64::from(usage_entry.has_cost > 0 || usage.has_cost > 0);

            contributions.records.push((
                window_name.clone(),
                account_scope,
                row.id,
                row.invoke_id.clone(),
                row.occurred_at.clone(),
            ));
            contributions
                .record_scopes
                .insert((window_name.clone(), account_scope));
        }
    }
    Ok(())
}

fn select_compacted_source_recovery_candidates(
    page: Vec<CompactedSourceRecoveryCandidate>,
) -> Result<(Vec<CompactedSourceRecoveryCandidate>, bool)> {
    let keys = page
        .iter()
        .map(|candidate| CompactedSourceRecoveryKey {
            id: candidate.id,
            occurred_at: candidate.occurred_at.clone(),
        })
        .collect();
    let (selected, _cursor, completed) = select_compacted_source_recovery_candidate_page(
        CompactedSourceRecoveryCandidatePage {
            keys,
            candidates: page,
        },
        SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE,
    )?;
    Ok((selected, completed))
}

fn select_compacted_source_recovery_candidate_page(
    page: CompactedSourceRecoveryCandidatePage,
    page_size: usize,
) -> Result<(
    Vec<CompactedSourceRecoveryCandidate>,
    Option<(String, i64)>,
    bool,
)> {
    let fetched_count = page.keys.len();
    let candidates_by_id = page
        .candidates
        .into_iter()
        .map(|candidate| (candidate.id, candidate))
        .collect::<HashMap<_, _>>();
    let mut selected_bytes = 0usize;
    let mut selected_rows = Vec::with_capacity(candidates_by_id.len().min(fetched_count));
    let mut cursor = None;
    let mut stopped_for_materialization_budget = false;
    let mut stopped_for_unresolved_source_row = false;
    for key in page.keys {
        let Some(row) = candidates_by_id.get(&key.id) else {
            // Running and pending source rows are not eligible for aggregation yet. Keep them at
            // the page boundary so a later terminal transition remains visible to this recovery
            // generation instead of being skipped by the tuple cursor.
            stopped_for_unresolved_source_row = true;
            break;
        };
        if selected_rows.len() >= page_size.max(1) {
            stopped_for_materialization_budget = true;
            break;
        }
        let row_bytes = usize::try_from(row.estimated_bytes)
            .context("compacted source recovery byte estimate overflow")?;
        if selected_rows.is_empty() && row_bytes > SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_BYTES {
            let error_message_bytes = usize::try_from(row.error_message_bytes)
                .context("compacted source recovery error message byte estimate overflow")?;
            let non_error_bytes = row_bytes.saturating_sub(error_message_bytes);
            let bounded_materialized_bytes = non_error_bytes.saturating_add(
                error_message_bytes.min(SUMMARY_COMPACTED_SOURCE_RECOVERY_ERROR_MESSAGE_BYTES),
            );
            if bounded_materialized_bytes > SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_BYTES {
                bail!(
                    "canonical source row {} exceeds the bounded recovery materialization budget",
                    row.id
                );
            }
            // An oversized raw row is admitted only when the bounded error-message projection
            // brings its materialized text back under the page budget. Other oversized fields
            // fail closed before they can be loaded into Rust.
            selected_rows.push(row.clone());
            cursor = Some((key.occurred_at, key.id));
            stopped_for_materialization_budget = true;
            break;
        }
        if !selected_rows.is_empty()
            && selected_bytes.saturating_add(row_bytes)
                > SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_BYTES
        {
            stopped_for_materialization_budget = true;
            break;
        }
        selected_bytes = selected_bytes.saturating_add(row_bytes);
        selected_rows.push(row.clone());
        cursor = Some((key.occurred_at, key.id));
    }
    let completed = !stopped_for_materialization_budget
        && !stopped_for_unresolved_source_row
        && fetched_count < page_size.max(1);
    Ok((selected_rows, cursor, completed))
}

async fn stage_compacted_source_recovery_page(
    state: &AppState,
    checkpoint: &CompactedSourceRecoveryCheckpoint,
    rows: &[CompactedSourceRecoverySourceRow],
    cursor: Option<&(String, i64)>,
    windows: &[(String, DateTime<Utc>, DateTime<Utc>)],
    completed: bool,
    transaction_budget: Duration,
) -> Result<CompactedSourceRecoveryPageOutcome> {
    let transaction_started = Instant::now();
    let deadline = transaction_started + transaction_budget;
    let options = state
        .pool
        .connect_options()
        .as_ref()
        .clone()
        .busy_timeout(transaction_budget);
    let mut connection =
        match tokio::time::timeout(transaction_budget, SqliteConnection::connect_with(&options))
            .await
        {
            Ok(result) => result.context("open compacted source recovery page connection")?,
            Err(_) => {
                return Ok(CompactedSourceRecoveryPageOutcome::Progress {
                    completed: false,
                    deferred: true,
                });
            }
        };
    let interrupted = Arc::new(AtomicBool::new(false));
    {
        let interrupted = interrupted.clone();
        let mut handle = connection
            .lock_handle()
            .await
            .context("acquire compacted source recovery progress handle")?;
        handle.set_progress_handler(1_000, move || {
            let keep_running = Instant::now() < deadline;
            if !keep_running {
                interrupted.store(true, Ordering::Release);
            }
            keep_running
        });
    }
    let remaining_budget = transaction_budget.saturating_sub(transaction_started.elapsed());
    if remaining_budget.is_zero() {
        let _ = close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
        return Ok(CompactedSourceRecoveryPageOutcome::Progress {
            completed: false,
            deferred: true,
        });
    }
    let begin_result = tokio::time::timeout(
        remaining_budget,
        sqlx::query("BEGIN IMMEDIATE").execute(&mut connection),
    )
    .await;
    match begin_result {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            let budget_expired = interrupted.load(Ordering::Acquire) || Instant::now() >= deadline;
            let contention = is_sqlite_contention(&anyhow!(error.to_string()));
            let _ =
                close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
            if budget_expired || contention {
                return Ok(CompactedSourceRecoveryPageOutcome::Progress {
                    completed: false,
                    deferred: true,
                });
            }
            return Err(anyhow!(error).context("begin compacted source recovery page transaction"));
        }
        Err(_) => {
            let _ =
                close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
            return Ok(CompactedSourceRecoveryPageOutcome::Progress {
                completed: false,
                deferred: true,
            });
        }
    }
    let remaining_budget = transaction_budget.saturating_sub(transaction_started.elapsed());
    if remaining_budget.is_zero() {
        let _ =
            rollback_and_close_compacted_source_recovery_connection_bounded(connection, deadline)
                .await?;
        return Ok(CompactedSourceRecoveryPageOutcome::Progress {
            completed: false,
            deferred: true,
        });
    }
    let result = tokio::time::timeout(
        remaining_budget,
        stage_compacted_source_recovery_page_in_transaction(
            state,
            checkpoint,
            rows,
            cursor,
            windows,
            completed,
            transaction_started,
            &mut connection,
        ),
    )
    .await;
    match result {
        Ok(Ok(CompactedSourceRecoveryPageOutcome::Reset)) => {
            let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                connection, deadline,
            )
            .await?;
            Ok(CompactedSourceRecoveryPageOutcome::Reset)
        }
        Ok(Ok(CompactedSourceRecoveryPageOutcome::Stale)) => {
            let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                connection, deadline,
            )
            .await?;
            Ok(CompactedSourceRecoveryPageOutcome::Stale)
        }
        Ok(Ok(outcome)) => {
            let remaining_budget = transaction_budget.saturating_sub(transaction_started.elapsed());
            if remaining_budget.is_zero() {
                let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                    connection, deadline,
                )
                .await?;
                return Ok(CompactedSourceRecoveryPageOutcome::Progress {
                    completed: false,
                    deferred: true,
                });
            }
            // Commit is the transaction's atomic boundary. The dedicated connection is closed
            // on every branch, so a timeout cannot return an in-flight rollback to the pool.
            match tokio::time::timeout(
                remaining_budget,
                sqlx::query("COMMIT").execute(&mut connection),
            )
            .await
            {
                Ok(result) => match result {
                    Ok(_) => {
                        if close_compacted_source_recovery_connection_bounded(connection, deadline)
                            .await?
                        {
                            Ok(outcome)
                        } else {
                            Ok(CompactedSourceRecoveryPageOutcome::Progress {
                                completed: false,
                                deferred: true,
                            })
                        }
                    }
                    Err(error) => {
                        let budget_expired =
                            interrupted.load(Ordering::Acquire) || Instant::now() >= deadline;
                        let contention = is_sqlite_contention(&anyhow!(error.to_string()));
                        let closed = close_compacted_source_recovery_connection_bounded(
                            connection, deadline,
                        )
                        .await?;
                        if budget_expired || contention || !closed {
                            Ok(CompactedSourceRecoveryPageOutcome::Progress {
                                completed: false,
                                deferred: true,
                            })
                        } else {
                            Err(anyhow!(error).context("commit compacted source recovery page"))
                        }
                    }
                },
                Err(_) => {
                    let _ =
                        close_compacted_source_recovery_connection_bounded(connection, deadline)
                            .await?;
                    Ok(CompactedSourceRecoveryPageOutcome::Progress {
                        completed: false,
                        deferred: true,
                    })
                }
            }
        }
        Ok(Err(error)) => {
            let budget_expired = interrupted.load(Ordering::Acquire) || Instant::now() >= deadline;
            let cleaned = rollback_and_close_compacted_source_recovery_connection_bounded(
                connection, deadline,
            )
            .await?;
            if budget_expired || !cleaned {
                Ok(CompactedSourceRecoveryPageOutcome::Progress {
                    completed: false,
                    deferred: true,
                })
            } else {
                Err(error)
            }
        }
        Err(_) => {
            let _ = rollback_and_close_compacted_source_recovery_connection_bounded(
                connection, deadline,
            )
            .await?;
            Ok(CompactedSourceRecoveryPageOutcome::Progress {
                completed: false,
                deferred: true,
            })
        }
    }
}

async fn close_compacted_source_recovery_connection(
    mut connection: SqliteConnection,
) -> Result<()> {
    let cleanup = connection.lock_handle().await.map(|mut handle| {
        handle.remove_progress_handler();
    });
    let closed = connection.close().await;
    cleanup.context("remove compacted source recovery progress handler")?;
    closed.context("close compacted source recovery connection")?;
    Ok(())
}

async fn close_compacted_source_recovery_connection_bounded(
    connection: SqliteConnection,
    deadline: Instant,
) -> Result<bool> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        drop(connection);
        return Ok(false);
    }
    match tokio::time::timeout(
        remaining,
        close_compacted_source_recovery_connection(connection),
    )
    .await
    {
        Ok(result) => {
            result?;
            Ok(true)
        }
        Err(_) => Ok(false),
    }
}

async fn rollback_and_close_compacted_source_recovery_connection_bounded(
    mut connection: SqliteConnection,
    deadline: Instant,
) -> Result<bool> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        drop(connection);
        return Ok(false);
    }
    if tokio::time::timeout(remaining, sqlx::query("ROLLBACK").execute(&mut connection))
        .await
        .is_err()
    {
        drop(connection);
        return Ok(false);
    }
    close_compacted_source_recovery_connection_bounded(connection, deadline).await
}

async fn rollback_compacted_source_recovery_transaction_bounded(
    connection: &mut SqliteConnection,
    deadline: Instant,
) -> bool {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return false;
    }
    matches!(
        tokio::time::timeout(remaining, sqlx::query("ROLLBACK").execute(&mut *connection),).await,
        Ok(Ok(_))
    )
}

async fn stage_compacted_source_recovery_page_in_transaction(
    state: &AppState,
    checkpoint: &CompactedSourceRecoveryCheckpoint,
    rows: &[CompactedSourceRecoverySourceRow],
    cursor: Option<&(String, i64)>,
    windows: &[(String, DateTime<Utc>, DateTime<Utc>)],
    completed: bool,
    transaction_started: Instant,
    connection: &mut SqliteConnection,
) -> Result<CompactedSourceRecoveryPageOutcome> {
    let Some(current_checkpoint) = load_compacted_source_recovery_checkpoint_tx(connection).await?
    else {
        return Ok(CompactedSourceRecoveryPageOutcome::Stale);
    };
    if !compacted_source_recovery_checkpoint_progress_matches(checkpoint, &current_checkpoint) {
        return Ok(CompactedSourceRecoveryPageOutcome::Stale);
    }
    let current_fence = load_summary_projection_generation_fence_tx(state, connection).await?;
    if !current_fence.coverage_sources_match(compacted_source_recovery_fence(checkpoint)) {
        return Ok(CompactedSourceRecoveryPageOutcome::Reset);
    }

    let mut contributions = CompactedSourceRecoveryContributions::default();
    for row in rows {
        add_compacted_source_row_contributions(&mut contributions, row, windows)?;
    }

    if !contributions.aggregates.is_empty() {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "INSERT INTO summary_compacted_source_recovery_aggregate (\
                scope, window_name, upstream_account_id, total_count, success_count, failure_count,\
                total_tokens, total_cost, non_success_cost, non_success_tokens\
            ) ",
        );
        query.push_values(
            contributions.aggregates.iter(),
            |mut values, ((window, account), delta)| {
                values
                    .push_bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
                    .push_bind(window)
                    .push_bind(*account)
                    .push_bind(delta.total_count)
                    .push_bind(delta.success_count)
                    .push_bind(delta.failure_count)
                    .push_bind(delta.total_tokens)
                    .push_bind(delta.total_cost)
                    .push_bind(delta.non_success_cost)
                    .push_bind(delta.non_success_tokens);
            },
        );
        query.push(
            " ON CONFLICT(scope, window_name, upstream_account_id) DO UPDATE SET \
              total_count = total_count + excluded.total_count, \
              success_count = success_count + excluded.success_count, \
              failure_count = failure_count + excluded.failure_count, \
              total_tokens = total_tokens + excluded.total_tokens, \
              total_cost = total_cost + excluded.total_cost, \
              non_success_cost = non_success_cost + excluded.non_success_cost, \
              non_success_tokens = non_success_tokens + excluded.non_success_tokens",
        );
        query
            .build()
            .execute(&mut *connection)
            .await
            .context("stage compacted source recovery aggregates")?;
    }
    if !contributions.usage.is_empty() {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "INSERT INTO summary_compacted_source_recovery_usage (\
                scope, window_name, upstream_account_id, normalized_model, normalized_reasoning_effort,\
                cache_write_tokens, cache_read_tokens, output_tokens, cost_input, cost_cache_write,\
                cost_cache_read, cost_output, cost_reasoning, cost_unknown, has_cost\
            ) ",
        );
        query.push_values(
            contributions.usage.iter(),
            |mut values, ((window, account, model, reasoning), delta)| {
                values
                    .push_bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
                    .push_bind(window)
                    .push_bind(*account)
                    .push_bind(model)
                    .push_bind(reasoning)
                    .push_bind(delta.cache_write_tokens)
                    .push_bind(delta.cache_read_tokens)
                    .push_bind(delta.output_tokens)
                    .push_bind(delta.cost_input)
                    .push_bind(delta.cost_cache_write)
                    .push_bind(delta.cost_cache_read)
                    .push_bind(delta.cost_output)
                    .push_bind(delta.cost_reasoning)
                    .push_bind(delta.cost_unknown)
                    .push_bind(delta.has_cost);
            },
        );
        query.push(
            " ON CONFLICT(scope, window_name, upstream_account_id, normalized_model, normalized_reasoning_effort) DO UPDATE SET \
              cache_write_tokens = cache_write_tokens + excluded.cache_write_tokens, \
              cache_read_tokens = cache_read_tokens + excluded.cache_read_tokens, \
              output_tokens = output_tokens + excluded.output_tokens, \
              cost_input = cost_input + excluded.cost_input, \
              cost_cache_write = cost_cache_write + excluded.cost_cache_write, \
              cost_cache_read = cost_cache_read + excluded.cost_cache_read, \
              cost_output = cost_output + excluded.cost_output, \
              cost_reasoning = cost_reasoning + excluded.cost_reasoning, \
              cost_unknown = cost_unknown + excluded.cost_unknown, \
              has_cost = MAX(has_cost, excluded.has_cost)",
        );
        query
            .build()
            .execute(&mut *connection)
            .await
            .context("stage compacted source recovery usage")?;
    }
    if !contributions.records.is_empty() {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "INSERT OR IGNORE INTO summary_compacted_source_recovery_record (\
                scope, window_name, upstream_account_id, row_id, invoke_id, occurred_at\
            ) ",
        );
        query.push_values(
            contributions.records.iter(),
            |mut values, (window, account, row_id, invoke_id, occurred_at)| {
                values
                    .push_bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
                    .push_bind(window)
                    .push_bind(*account)
                    .push_bind(*row_id)
                    .push_bind(invoke_id)
                    .push_bind(occurred_at);
            },
        );
        query
            .build()
            .execute(&mut *connection)
            .await
            .context("stage compacted source recovery record identities")?;
    }
    for (window, account) in &contributions.record_scopes {
        sqlx::query(
            "DELETE FROM summary_compacted_source_recovery_record \
              WHERE scope = ?1 AND window_name = ?2 AND upstream_account_id = ?3 \
                AND row_id NOT IN (\
                    SELECT row_id FROM summary_compacted_source_recovery_record \
                     WHERE scope = ?1 AND window_name = ?2 AND upstream_account_id = ?3 \
                     ORDER BY occurred_at DESC, row_id DESC LIMIT ?4\
                )",
        )
        .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
        .bind(window)
        .bind(*account)
        .bind(checkpoint.list_limit_max.max(1))
        .execute(&mut *connection)
        .await
        .context("prune compacted source recovery record identities")?;
    }

    let next_state = if completed {
        "completed"
    } else if transaction_started.elapsed() >= SUMMARY_COMPACTED_SOURCE_RECOVERY_TRANSACTION_BUDGET
    {
        "deferred"
    } else {
        "running"
    };
    sqlx::query(
        "UPDATE summary_compacted_source_recovery_checkpoint SET \
            cursor_occurred_at = COALESCE(?1, cursor_occurred_at), \
            cursor_id = CASE WHEN ?2 > 0 THEN ?2 ELSE cursor_id END, \
            processed_rows = processed_rows + ?3, state = ?4, last_error_code = NULL, \
            last_error_message = NULL, updated_at = datetime('now') WHERE scope = ?5",
    )
    .bind(cursor.map(|(occurred_at, _)| occurred_at.clone()))
    .bind(cursor.map(|(_, id)| *id).unwrap_or_default())
    .bind(i64::try_from(rows.len()).context("compacted source recovery row count overflow")?)
    .bind(next_state)
    .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
    .execute(&mut *connection)
    .await
    .context("advance compacted source recovery checkpoint")?;
    Ok(CompactedSourceRecoveryPageOutcome::Progress {
        completed,
        deferred: next_state == "deferred",
    })
}

fn is_sqlite_contention(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let message = cause.to_string().to_ascii_lowercase();
        message.contains("database is locked")
            || message.contains("database table is locked")
            || message.contains("sqlite_busy")
            || message.contains("sqlite_locked")
    })
}

const COMPACTED_SOURCE_RECOVERY_STATE_UPDATE_SQL: &str = "UPDATE summary_compacted_source_recovery_checkpoint SET state = ?1, last_error_code = ?2,\
                last_error_message = ?3, updated_at = datetime('now') WHERE scope = ?4";

async fn update_compacted_source_recovery_state(
    state: &AppState,
    recovery_state: SummaryCompactedSourceRecoveryState,
    error_code: Option<&str>,
    error_message: Option<&str>,
) -> Result<()> {
    sqlx::query(COMPACTED_SOURCE_RECOVERY_STATE_UPDATE_SQL)
        .bind(recovery_state.as_str())
        .bind(error_code)
        .bind(error_message.map(|message| message.chars().take(256).collect::<String>()))
        .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
        .execute(&state.pool)
        .await
        .context("update compacted source recovery state")?;
    state
        .subscription_hub
        .set_summary_compacted_source_recovery_state(recovery_state)
        .await;
    Ok(())
}

async fn update_compacted_source_recovery_state_bounded(
    state: &AppState,
    recovery_state: SummaryCompactedSourceRecoveryState,
    error_code: Option<&str>,
    error_message: Option<&str>,
    budget: Duration,
) -> Result<bool> {
    if budget.is_zero() {
        state
            .subscription_hub
            .set_summary_compacted_source_recovery_state(recovery_state)
            .await;
        return Ok(false);
    }
    let started = Instant::now();
    let deadline = started + budget;
    let options = state
        .pool
        .connect_options()
        .as_ref()
        .clone()
        .busy_timeout(budget);
    let mut connection =
        match tokio::time::timeout(budget, SqliteConnection::connect_with(&options)).await {
            Ok(Ok(connection)) => connection,
            Ok(Err(error)) => {
                let error = anyhow!(error).context("open bounded recovery state connection");
                if is_sqlite_contention(&error) {
                    state
                        .subscription_hub
                        .set_summary_compacted_source_recovery_state(recovery_state)
                        .await;
                    return Ok(false);
                }
                return Err(error);
            }
            Err(_) => {
                state
                    .subscription_hub
                    .set_summary_compacted_source_recovery_state(recovery_state)
                    .await;
                return Ok(false);
            }
        };
    let interrupted = Arc::new(AtomicBool::new(false));
    {
        let interrupted = interrupted.clone();
        let mut handle = connection
            .lock_handle()
            .await
            .context("acquire bounded recovery state progress handle")?;
        handle.set_progress_handler(1_000, move || {
            let keep_running = Instant::now() < deadline;
            if !keep_running {
                interrupted.store(true, Ordering::Release);
            }
            keep_running
        });
    }
    let remaining = budget.saturating_sub(started.elapsed());
    let result = if remaining.is_zero() {
        Ok(false)
    } else {
        match tokio::time::timeout(
            remaining,
            sqlx::query(COMPACTED_SOURCE_RECOVERY_STATE_UPDATE_SQL)
                .bind(recovery_state.as_str())
                .bind(error_code)
                .bind(error_message.map(|message| message.chars().take(256).collect::<String>()))
                .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
                .execute(&mut connection),
        )
        .await
        {
            Ok(Ok(_)) => Ok(true),
            Ok(Err(error)) => {
                let budget_expired =
                    interrupted.load(Ordering::Acquire) || Instant::now() >= deadline;
                let error = anyhow!(error);
                if budget_expired || is_sqlite_contention(&error) {
                    Ok(false)
                } else {
                    Err(error.context("update bounded recovery state"))
                }
            }
            Err(_) => Ok(false),
        }
    };
    let _ = close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
    state
        .subscription_hub
        .set_summary_compacted_source_recovery_state(recovery_state)
        .await;
    result
}

async fn compacted_source_recovery_staged_totals_match_source(
    connection: &mut SqliteConnection,
    checkpoint: &CompactedSourceRecoveryCheckpoint,
    windows: &[(String, DateTime<Utc>, DateTime<Utc>)],
) -> Result<bool> {
    let source_count_sql = format!(
        "SELECT COUNT(*) FROM codex_invocations WHERE {INVOCATION_STATUS_NORMALIZED_SQL} \
         NOT IN ('running', 'pending') AND id <= ?1 AND occurred_at >= ?2 AND occurred_at < ?3"
    );
    for (window_name, start, end) in windows {
        let source_count = sqlx::query_scalar::<_, i64>(&source_count_sql)
            .bind(checkpoint.target_live_high_watermark_id)
            .bind(crate::stats::db_occurred_at_lower_bound(*start))
            .bind(crate::db_occurred_at_upper_bound(*end))
            .fetch_one(&mut *connection)
            .await
            .with_context(|| {
                format!("count canonical rows for compacted source recovery window {window_name}")
            })?;
        let staged_count = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(total_count, 0) \
               FROM summary_compacted_source_recovery_aggregate \
              WHERE scope = ?1 AND window_name = ?2 AND upstream_account_id = 0",
        )
        .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
        .bind(window_name)
        .fetch_optional(&mut *connection)
        .await
        .with_context(|| {
            format!("load staged total for compacted source recovery window {window_name}")
        })?
        .unwrap_or_default();
        if source_count != staged_count {
            warn!(
                window = %window_name,
                source_count,
                staged_count,
                "compacted source recovery staging no longer covers the canonical terminal rows"
            );
            return Ok(false);
        }
    }
    Ok(true)
}

async fn publish_compacted_source_recovery_snapshot(
    state: &AppState,
    budget: Duration,
) -> Result<bool> {
    let started = Instant::now();
    let transaction_budget = budget.min(SUMMARY_COMPACTED_SOURCE_RECOVERY_TRANSACTION_BUDGET);
    let Some((mut connection, interrupted, deadline)) = open_compacted_source_recovery_connection(
        state,
        transaction_budget,
        "open compacted source recovery publication connection",
    )
    .await?
    else {
        return Ok(false);
    };
    let remaining = transaction_budget.saturating_sub(started.elapsed());
    let result = if remaining.is_zero() {
        Ok(false)
    } else {
        match tokio::time::timeout(
            remaining,
            publish_compacted_source_recovery_snapshot_in_transaction(
                state,
                &mut connection,
                interrupted.as_ref(),
                deadline,
            ),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Ok(false),
        }
    };
    let _ = close_compacted_source_recovery_connection_bounded(connection, deadline).await?;
    match result {
        Ok(published) => Ok(published),
        Err(error)
            if interrupted.load(Ordering::Acquire)
                || Instant::now() >= deadline
                || is_sqlite_contention(&error) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

async fn publish_compacted_source_recovery_snapshot_in_transaction(
    state: &AppState,
    connection: &mut SqliteConnection,
    interrupted: &AtomicBool,
    deadline: Instant,
) -> Result<bool> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Ok(false);
    }
    match tokio::time::timeout(
        remaining,
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *connection),
    )
    .await
    {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            let error = anyhow!(error);
            if interrupted.load(Ordering::Acquire)
                || Instant::now() >= deadline
                || is_sqlite_contention(&error)
            {
                return Ok(false);
            }
            return Err(error.context("begin compacted source recovery publication"));
        }
        Err(_) => return Ok(false),
    }
    let Some(checkpoint) = load_compacted_source_recovery_checkpoint_tx(connection).await? else {
        let _ = rollback_compacted_source_recovery_transaction_bounded(connection, deadline).await;
        return Ok(false);
    };
    if checkpoint.state != SummaryCompactedSourceRecoveryState::Completed.as_str() {
        let _ = rollback_compacted_source_recovery_transaction_bounded(connection, deadline).await;
        return Ok(false);
    }
    let expected_fence = compacted_source_recovery_fence(&checkpoint);
    let current_fence = load_summary_projection_generation_fence_tx(state, connection).await?;
    if !current_fence.coverage_sources_match(expected_fence) {
        if rollback_compacted_source_recovery_transaction_bounded(connection, deadline).await {
            let _ = ensure_compacted_source_recovery_with_force_bounded(
                state,
                false,
                SUMMARY_COMPACTED_SOURCE_RECOVERY_TRANSACTION_BUDGET
                    .min(deadline.saturating_duration_since(Instant::now())),
            )
            .await?;
        }
        return Ok(false);
    }
    let windows = parse_compacted_source_recovery_windows(&checkpoint.window_ranges_json)?;
    if !compacted_source_recovery_staged_totals_match_source(connection, &checkpoint, &windows)
        .await?
    {
        if rollback_compacted_source_recovery_transaction_bounded(connection, deadline).await {
            let _ = ensure_compacted_source_recovery_with_force_bounded(
                state,
                true,
                SUMMARY_COMPACTED_SOURCE_RECOVERY_TRANSACTION_BUDGET
                    .min(deadline.saturating_duration_since(Instant::now())),
            )
            .await?;
        }
        return Ok(false);
    }
    let mut entries = HashMap::<(String, Option<i64>), SummaryDashboardRecoveryEntry>::new();
    let aggregate_rows = sqlx::query_as::<_, (String, i64, i64, i64, i64, i64, i64, f64, f64)>(
        "SELECT window_name, upstream_account_id, total_count, success_count, failure_count,\
                total_tokens, non_success_tokens, total_cost, non_success_cost \
           FROM summary_compacted_source_recovery_aggregate WHERE scope = ?1",
    )
    .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
    .fetch_all(&mut *connection)
    .await
    .context("load compacted source recovery aggregates")?;
    for (
        window_name,
        account_scope,
        total_count,
        success_count,
        failure_count,
        total_tokens,
        non_success_tokens,
        total_cost,
        non_success_cost,
    ) in aggregate_rows
    {
        entries.insert(
            (window_name, (account_scope != 0).then_some(account_scope)),
            SummaryDashboardRecoveryEntry {
                totals: StatsTotals {
                    total_count,
                    success_count,
                    failure_count,
                    total_cost,
                    total_tokens,
                    non_success_cost,
                },
                non_success_tokens,
                usage_breakdown: UsageBreakdownResponse {
                    cache_write_tokens: 0,
                    cache_read_tokens: 0,
                    output_tokens: 0,
                    costs: None,
                    models: Vec::new(),
                },
            },
        );
    }
    let usage_rows = sqlx::query_as::<
        _,
        (
            String,
            i64,
            String,
            String,
            i64,
            i64,
            i64,
            f64,
            f64,
            f64,
            f64,
            f64,
            f64,
            i64,
        ),
    >(
        "SELECT window_name, upstream_account_id, normalized_model, normalized_reasoning_effort,\
                cache_write_tokens, cache_read_tokens, output_tokens, cost_input, cost_cache_write,\
                cost_cache_read, cost_output, cost_reasoning, cost_unknown, has_cost \
           FROM summary_compacted_source_recovery_usage WHERE scope = ?1",
    )
    .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
    .fetch_all(&mut *connection)
    .await
    .context("load compacted source recovery usage")?;
    let mut usage_accumulators = HashMap::<(String, Option<i64>), UsageBreakdownAccumulator>::new();
    for (
        window_name,
        account_scope,
        model,
        reasoning_effort,
        cache_write_tokens,
        cache_read_tokens,
        output_tokens,
        cost_input,
        cost_cache_write,
        cost_cache_read,
        cost_output,
        cost_reasoning,
        cost_unknown,
        has_cost,
    ) in usage_rows
    {
        usage_accumulators
            .entry((window_name, (account_scope != 0).then_some(account_scope)))
            .or_default()
            .add_compacted_values(
                model,
                (!reasoning_effort.is_empty()).then_some(reasoning_effort),
                cache_write_tokens,
                cache_read_tokens,
                output_tokens,
                UsageCostBreakdownResponse {
                    input: cost_input,
                    cache_write: cost_cache_write,
                    cache_read: cost_cache_read,
                    output: cost_output,
                    reasoning: cost_reasoning,
                    unknown: cost_unknown,
                },
                has_cost > 0,
            );
    }
    for (key, accumulator) in usage_accumulators {
        entries
            .entry(key)
            .or_insert_with(|| SummaryDashboardRecoveryEntry {
                totals: StatsTotals::default(),
                non_success_tokens: 0,
                usage_breakdown: UsageBreakdownResponse {
                    cache_write_tokens: 0,
                    cache_read_tokens: 0,
                    output_tokens: 0,
                    costs: None,
                    models: Vec::new(),
                },
            })
            .usage_breakdown = accumulator.into_response();
    }
    let bounded_record_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM summary_compacted_source_recovery_record WHERE scope = ?1",
    )
    .bind(SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE)
    .fetch_one(&mut *connection)
    .await
    .context("count compacted source recovery record identities")?
    .max(0) as usize;
    let snapshot = SummaryDashboardRecoverySnapshot {
        time_zone: SUMMARY_COMPACTED_SOURCE_RECOVERY_TIME_ZONE.to_string(),
        window_ranges_json: checkpoint.window_ranges_json.clone(),
        generation_fence: expected_fence,
        entries,
        bounded_record_count,
    };
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Ok(false);
    }
    match tokio::time::timeout(remaining, sqlx::query("COMMIT").execute(&mut *connection)).await {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            let error = anyhow!(error);
            if interrupted.load(Ordering::Acquire)
                || Instant::now() >= deadline
                || is_sqlite_contention(&error)
            {
                return Ok(false);
            }
            state
                .subscription_hub
                .clear_summary_dashboard_recovery()
                .await;
            return Err(error.context("commit compacted source recovery publication"));
        }
        Err(_) => return Ok(false),
    }
    state
        .subscription_hub
        .store_summary_dashboard_recovery(snapshot)
        .await;
    state
        .subscription_hub
        .set_summary_compacted_source_recovery_state(SummaryCompactedSourceRecoveryState::Completed)
        .await;
    debug!(
        window_count = windows.len(),
        bounded_record_count, "published compacted source recovery snapshot"
    );
    Ok(true)
}

async fn run_compacted_source_recovery_turn_with_runtime(
    state: &AppState,
    runtime: &mut CompactedSourceRecoveryRuntime,
) -> Result<Option<SummaryCoverageRecoveryTurn>> {
    let started = Instant::now();
    let checkpoint = match tokio::time::timeout(
        SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET,
        load_compacted_source_recovery_checkpoint(&state.pool),
    )
    .await
    {
        Ok(result) => result?,
        Err(_) => {
            state
                .subscription_hub
                .set_summary_compacted_source_recovery_state(
                    SummaryCompactedSourceRecoveryState::Deferred,
                )
                .await;
            return Ok(Some(SummaryCoverageRecoveryTurn {
                next_turn: SummaryCoverageRecoveryNextTurn::Deferred,
                reservation: None,
            }));
        }
    };
    let Some(checkpoint) = checkpoint else {
        return Ok(None);
    };
    runtime.observe_checkpoint(&checkpoint);
    let checkpoint_state = SummaryCompactedSourceRecoveryState::from_storage(&checkpoint.state);
    state
        .subscription_hub
        .set_summary_compacted_source_recovery_state(checkpoint_state)
        .await;
    match checkpoint_state {
        SummaryCompactedSourceRecoveryState::Failed => return Ok(None),
        SummaryCompactedSourceRecoveryState::Completed => {
            let remaining =
                SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                state
                    .subscription_hub
                    .set_summary_compacted_source_recovery_state(
                        SummaryCompactedSourceRecoveryState::Deferred,
                    )
                    .await;
                return Ok(Some(SummaryCoverageRecoveryTurn {
                    next_turn: SummaryCoverageRecoveryNextTurn::Deferred,
                    reservation: None,
                }));
            }
            let published = match tokio::time::timeout(
                remaining,
                publish_compacted_source_recovery_snapshot(state, remaining),
            )
            .await
            {
                Ok(result) => result?,
                Err(_) => {
                    state
                        .subscription_hub
                        .set_summary_compacted_source_recovery_state(
                            SummaryCompactedSourceRecoveryState::Deferred,
                        )
                        .await;
                    return Ok(Some(SummaryCoverageRecoveryTurn {
                        next_turn: SummaryCoverageRecoveryNextTurn::Deferred,
                        reservation: None,
                    }));
                }
            };
            if published {
                return Ok(None);
            }
            let remaining =
                SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET.saturating_sub(started.elapsed());
            let refreshed = match tokio::time::timeout(
                remaining,
                load_compacted_source_recovery_checkpoint(&state.pool),
            )
            .await
            {
                Ok(result) => result?,
                Err(_) => {
                    state
                        .subscription_hub
                        .set_summary_compacted_source_recovery_state(
                            SummaryCompactedSourceRecoveryState::Deferred,
                        )
                        .await;
                    return Ok(Some(SummaryCoverageRecoveryTurn {
                        next_turn: SummaryCoverageRecoveryNextTurn::Deferred,
                        reservation: None,
                    }));
                }
            };
            if refreshed.as_ref().is_some_and(|row| {
                row.state == SummaryCompactedSourceRecoveryState::Pending.as_str()
            }) {
                return Ok(Some(SummaryCoverageRecoveryTurn {
                    next_turn: SummaryCoverageRecoveryNextTurn::Immediate,
                    reservation: Some(
                        crate::db_pressure::global_db_pressure_gate().reserve_priority_background(),
                    ),
                }));
            }
            return Ok(None);
        }
        SummaryCompactedSourceRecoveryState::Pending
        | SummaryCompactedSourceRecoveryState::Running
        | SummaryCompactedSourceRecoveryState::Deferred => {}
    }
    let remaining = SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET.saturating_sub(started.elapsed());
    if !update_compacted_source_recovery_state_bounded(
        state,
        SummaryCompactedSourceRecoveryState::Running,
        None,
        None,
        remaining,
    )
    .await?
    {
        state
            .subscription_hub
            .set_summary_compacted_source_recovery_state(
                SummaryCompactedSourceRecoveryState::Deferred,
            )
            .await;
        return Ok(Some(SummaryCoverageRecoveryTurn {
            next_turn: SummaryCoverageRecoveryNextTurn::Deferred,
            reservation: None,
        }));
    }
    loop {
        let remaining =
            SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            state
                .subscription_hub
                .set_summary_compacted_source_recovery_state(
                    SummaryCompactedSourceRecoveryState::Deferred,
                )
                .await;
            return Ok(Some(SummaryCoverageRecoveryTurn {
                next_turn: SummaryCoverageRecoveryNextTurn::Deferred,
                reservation: Some(
                    crate::db_pressure::global_db_pressure_gate().reserve_priority_background(),
                ),
            }));
        }
        let checkpoint = match tokio::time::timeout(
            remaining,
            load_compacted_source_recovery_checkpoint(&state.pool),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                state
                    .subscription_hub
                    .set_summary_compacted_source_recovery_state(
                        SummaryCompactedSourceRecoveryState::Deferred,
                    )
                    .await;
                return Ok(Some(SummaryCoverageRecoveryTurn {
                    next_turn: SummaryCoverageRecoveryNextTurn::Deferred,
                    reservation: Some(
                        crate::db_pressure::global_db_pressure_gate().reserve_priority_background(),
                    ),
                }));
            }
        };
        let Some(checkpoint) = checkpoint else {
            return Ok(None);
        };
        runtime.observe_checkpoint(&checkpoint);
        let windows = parse_compacted_source_recovery_windows(&checkpoint.window_ranges_json)?;
        if started.elapsed() >= SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET {
            state.observability.count_performance(
                PerformanceCounter::Recovery(RecoveryDiagnostic::SummaryRecoveryBudgetExhausted),
                1,
            );
            state
                .subscription_hub
                .set_summary_compacted_source_recovery_state(
                    SummaryCompactedSourceRecoveryState::Deferred,
                )
                .await;
            return Ok(Some(SummaryCoverageRecoveryTurn {
                next_turn: SummaryCoverageRecoveryNextTurn::Deferred,
                reservation: Some(
                    crate::db_pressure::global_db_pressure_gate().reserve_priority_background(),
                ),
            }));
        }
        let remaining =
            SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET.saturating_sub(started.elapsed());
        let candidates = match load_compacted_source_recovery_candidates(
            state,
            &checkpoint,
            &windows,
            runtime.page_size,
            remaining,
        )
        .await?
        {
            Some(candidates) => candidates,
            None => {
                runtime.halve_page_size();
                debug!(
                    page_size = runtime.page_size,
                    "halved compacted source recovery page size after bounded candidate read"
                );
                state.observability.count_performance(
                    PerformanceCounter::Recovery(
                        RecoveryDiagnostic::SummaryRecoveryBudgetExhausted,
                    ),
                    1,
                );
                let _ = update_compacted_source_recovery_state_bounded(
                    state,
                    SummaryCompactedSourceRecoveryState::Deferred,
                    None,
                    None,
                    SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET.saturating_sub(started.elapsed()),
                )
                .await?;
                return Ok(Some(SummaryCoverageRecoveryTurn {
                    next_turn: SummaryCoverageRecoveryNextTurn::Deferred,
                    reservation: Some(
                        crate::db_pressure::global_db_pressure_gate().reserve_priority_background(),
                    ),
                }));
            }
        };
        let scanned_keys = candidates.keys.len();
        state.observability.count_performance(
            PerformanceCounter::Recovery(RecoveryDiagnostic::SummaryRecoveryScannedKeys),
            scanned_keys as u64,
        );
        let (selected_candidates, cursor, completed) =
            match select_compacted_source_recovery_candidate_page(candidates, runtime.page_size) {
                Ok(selection) => selection,
                Err(error) => {
                    let message = error.to_string();
                    let _ = update_compacted_source_recovery_state_bounded(
                        state,
                        SummaryCompactedSourceRecoveryState::Failed,
                        Some(SUMMARY_RECOVERY_FAILED_CODE),
                        Some(&message),
                        SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET
                            .saturating_sub(started.elapsed()),
                    )
                    .await?;
                    return Ok(None);
                }
            };
        debug!(
            scanned_keys,
            selected_terminal_rows = selected_candidates.len(),
            cursor_advanced = cursor.is_some(),
            completed,
            "selected bounded compacted source recovery page"
        );
        if selected_candidates.is_empty() && cursor.is_none() && !completed {
            // The first key in the page is still running or pending. There is no durable cursor
            // progress to commit, so yield without issuing a no-op checkpoint write.
            state
                .subscription_hub
                .set_summary_compacted_source_recovery_state(
                    SummaryCompactedSourceRecoveryState::Deferred,
                )
                .await;
            state.observability.count_performance(
                PerformanceCounter::Recovery(
                    RecoveryDiagnostic::SummaryRecoveryLiveBoundaryDeferred,
                ),
                1,
            );
            return Ok(Some(SummaryCoverageRecoveryTurn {
                next_turn: SummaryCoverageRecoveryNextTurn::Deferred,
                reservation: None,
            }));
        }
        let selected_rows = selected_candidates
            .into_iter()
            .map(CompactedSourceRecoveryCandidate::into_source_row)
            .collect::<Vec<_>>();
        let transaction_budget = SUMMARY_COMPACTED_SOURCE_RECOVERY_TRANSACTION_BUDGET
            .min(SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET.saturating_sub(started.elapsed()));
        let outcome = match stage_compacted_source_recovery_page(
            state,
            &checkpoint,
            &selected_rows,
            cursor.as_ref(),
            &windows,
            completed,
            transaction_budget,
        )
        .await
        {
            Err(error) if is_sqlite_contention(&error) => {
                state.observability.count_performance(
                    PerformanceCounter::Recovery(
                        RecoveryDiagnostic::SummaryRecoveryContentionDeferred,
                    ),
                    1,
                );
                let _ = update_compacted_source_recovery_state_bounded(
                    state,
                    SummaryCompactedSourceRecoveryState::Deferred,
                    None,
                    None,
                    SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET.saturating_sub(started.elapsed()),
                )
                .await?;
                return Ok(Some(SummaryCoverageRecoveryTurn {
                    next_turn: SummaryCoverageRecoveryNextTurn::Deferred,
                    reservation: None,
                }));
            }
            Err(error) => {
                warn!(error = ?error, "compacted source recovery page failed");
                let _ = update_compacted_source_recovery_state_bounded(
                    state,
                    SummaryCompactedSourceRecoveryState::Failed,
                    Some(SUMMARY_RECOVERY_FAILED_CODE),
                    Some(&error.to_string()),
                    SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET.saturating_sub(started.elapsed()),
                )
                .await?;
                return Ok(None);
            }
            Ok(outcome) => outcome,
        };
        if let CompactedSourceRecoveryPageOutcome::Progress { deferred, .. } = outcome {
            if deferred {
                state.observability.count_performance(
                    PerformanceCounter::Recovery(
                        RecoveryDiagnostic::SummaryRecoveryBudgetExhausted,
                    ),
                    1,
                );
            } else {
                if !selected_rows.is_empty() || completed {
                    runtime.mark_progress();
                }
                state.observability.count_performance(
                    PerformanceCounter::Recovery(
                        RecoveryDiagnostic::SummaryRecoveryCommittedTerminalRows,
                    ),
                    selected_rows.len() as u64,
                );
                if cursor.is_some() {
                    state.observability.count_performance(
                        PerformanceCounter::Recovery(
                            RecoveryDiagnostic::SummaryRecoveryCursorAdvanced,
                        ),
                        1,
                    );
                }
            }
        }
        if outcome == CompactedSourceRecoveryPageOutcome::Reset {
            let remaining =
                SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET.saturating_sub(started.elapsed());
            match tokio::time::timeout(remaining, ensure_compacted_source_recovery(state)).await {
                Ok(result) => {
                    result?;
                    runtime.mark_progress();
                    return Ok(Some(SummaryCoverageRecoveryTurn {
                        next_turn: SummaryCoverageRecoveryNextTurn::Immediate,
                        reservation: Some(
                            crate::db_pressure::global_db_pressure_gate()
                                .reserve_priority_background(),
                        ),
                    }));
                }
                Err(_) => {
                    state
                        .subscription_hub
                        .set_summary_compacted_source_recovery_state(
                            SummaryCompactedSourceRecoveryState::Deferred,
                        )
                        .await;
                    return Ok(Some(SummaryCoverageRecoveryTurn {
                        next_turn: SummaryCoverageRecoveryNextTurn::Deferred,
                        reservation: Some(
                            crate::db_pressure::global_db_pressure_gate()
                                .reserve_priority_background(),
                        ),
                    }));
                }
            }
        }
        if outcome == CompactedSourceRecoveryPageOutcome::Stale {
            return Ok(Some(SummaryCoverageRecoveryTurn {
                next_turn: SummaryCoverageRecoveryNextTurn::Deferred,
                reservation: None,
            }));
        }
        let CompactedSourceRecoveryPageOutcome::Progress {
            completed,
            deferred,
        } = outcome
        else {
            unreachable!("compacted source recovery reset is handled above")
        };
        if completed {
            let remaining =
                SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET.saturating_sub(started.elapsed());
            let published = match tokio::time::timeout(
                remaining,
                publish_compacted_source_recovery_snapshot(state, remaining),
            )
            .await
            {
                Ok(result) => result?,
                Err(_) => {
                    state
                        .subscription_hub
                        .set_summary_compacted_source_recovery_state(
                            SummaryCompactedSourceRecoveryState::Deferred,
                        )
                        .await;
                    return Ok(Some(SummaryCoverageRecoveryTurn {
                        next_turn: SummaryCoverageRecoveryNextTurn::Deferred,
                        reservation: Some(
                            crate::db_pressure::global_db_pressure_gate()
                                .reserve_priority_background(),
                        ),
                    }));
                }
            };
            if published {
                runtime.mark_progress();
                state.observability.count_performance(
                    PerformanceCounter::Recovery(RecoveryDiagnostic::SummaryRecoveryPublication),
                    1,
                );
                return Ok(None);
            }
            return Ok(Some(SummaryCoverageRecoveryTurn {
                next_turn: SummaryCoverageRecoveryNextTurn::Deferred,
                reservation: Some(
                    crate::db_pressure::global_db_pressure_gate().reserve_priority_background(),
                ),
            }));
        }
        if deferred || started.elapsed() >= SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET {
            let _ = update_compacted_source_recovery_state_bounded(
                state,
                SummaryCompactedSourceRecoveryState::Deferred,
                None,
                None,
                SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET.saturating_sub(started.elapsed()),
            )
            .await?;
            return Ok(Some(SummaryCoverageRecoveryTurn {
                next_turn: SummaryCoverageRecoveryNextTurn::Deferred,
                reservation: Some(
                    crate::db_pressure::global_db_pressure_gate().reserve_priority_background(),
                ),
            }));
        }
    }
}

#[cfg(test)]
pub(crate) async fn run_compacted_source_recovery_one_page_for_test(
    state: &AppState,
) -> Result<()> {
    ensure_compacted_source_recovery(state).await?;
    update_compacted_source_recovery_state(
        state,
        SummaryCompactedSourceRecoveryState::Running,
        None,
        None,
    )
    .await?;
    let checkpoint = load_compacted_source_recovery_checkpoint(&state.pool)
        .await?
        .ok_or_else(|| anyhow!("test recovery checkpoint was not initialized"))?;
    let windows = parse_compacted_source_recovery_windows(&checkpoint.window_ranges_json)?;
    let candidates = load_compacted_source_recovery_candidates(
        state,
        &checkpoint,
        &windows,
        SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE,
        SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET,
    )
    .await?
    .ok_or_else(|| anyhow!("test recovery candidate read exceeded its budget"))?;
    let (selected_candidates, cursor, completed) = select_compacted_source_recovery_candidate_page(
        candidates,
        SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE,
    )?;
    if selected_candidates.is_empty() && cursor.is_none() && !completed {
        state
            .subscription_hub
            .set_summary_compacted_source_recovery_state(
                SummaryCompactedSourceRecoveryState::Deferred,
            )
            .await;
        return Ok(());
    }
    let selected_rows = selected_candidates
        .into_iter()
        .map(CompactedSourceRecoveryCandidate::into_source_row)
        .collect::<Vec<_>>();
    match stage_compacted_source_recovery_page(
        state,
        &checkpoint,
        &selected_rows,
        cursor.as_ref(),
        &windows,
        completed,
        SUMMARY_COMPACTED_SOURCE_RECOVERY_TRANSACTION_BUDGET,
    )
    .await?
    {
        CompactedSourceRecoveryPageOutcome::Progress { .. } => Ok(()),
        CompactedSourceRecoveryPageOutcome::Reset => {
            bail!("test recovery page observed a generation fence reset")
        }
        CompactedSourceRecoveryPageOutcome::Stale => {
            bail!("test recovery page observed a stale checkpoint")
        }
    }
}

#[cfg(test)]
pub(crate) async fn explain_compacted_source_recovery_key_page_for_test(
    pool: &sqlx::SqlitePool,
) -> Result<Vec<String>> {
    let parse_time = |value: &str| {
        DateTime::parse_from_rfc3339(value)
            .with_context(|| format!("parse test recovery plan time {value}"))
            .map(|parsed| parsed.with_timezone(&Utc))
    };
    let checkpoint = CompactedSourceRecoveryCheckpoint {
        scope: SUMMARY_COMPACTED_SOURCE_RECOVERY_SCOPE.to_string(),
        format_version: 1,
        time_zone: SUMMARY_COMPACTED_SOURCE_RECOVERY_TIME_ZONE.to_string(),
        range_start: "2026-10-01T00:00:00Z".to_string(),
        range_end: "2026-10-08T00:00:00Z".to_string(),
        window_ranges_json: "[]".to_string(),
        cursor_occurred_at: Some("2026-10-02T00:00:00Z".to_string()),
        cursor_id: 100,
        target_live_high_watermark_id: 100_001,
        target_rollup_live_cursor: 0,
        target_account_rollup_live_cursor: None,
        target_manifest_high_watermark_id: None,
        target_coverage_revision: 0,
        target_account_coverage_revision: 0,
        target_terminal_sequence: 0,
        list_limit_max: 512,
        processed_rows: 0,
        state: SummaryCompactedSourceRecoveryState::Running
            .as_str()
            .to_string(),
        last_error_code: None,
        last_error_message: None,
    };
    let windows = vec![
        (
            "today".to_string(),
            parse_time("2026-10-01T00:00:00Z")?,
            parse_time("2026-10-05T00:00:00Z")?,
        ),
        (
            "yesterday".to_string(),
            parse_time("2026-10-02T00:00:00Z")?,
            parse_time("2026-10-06T00:00:00Z")?,
        ),
        (
            "previous7d".to_string(),
            parse_time("2026-10-01T00:00:00Z")?,
            parse_time("2026-10-08T00:00:00Z")?,
        ),
    ];
    let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new("EXPLAIN QUERY PLAN ");
    push_compacted_source_recovery_key_page_query(
        &mut query,
        &checkpoint,
        &windows,
        SUMMARY_COMPACTED_SOURCE_RECOVERY_PAGE_SIZE,
    )?;
    let plan: Vec<(i64, i64, i64, String)> = query.build_query_as().fetch_all(pool).await?;
    Ok(plan.into_iter().map(|(_, _, _, detail)| detail).collect())
}

#[cfg(test)]
pub(crate) async fn run_compacted_source_recovery_until_complete_for_test(
    state: &AppState,
) -> Result<()> {
    ensure_compacted_source_recovery(state).await?;
    let mut runtime = CompactedSourceRecoveryRuntime::default();
    for _ in 0..128 {
        let Some(turn) =
            run_compacted_source_recovery_turn_with_runtime(state, &mut runtime).await?
        else {
            let checkpoint = load_compacted_source_recovery_checkpoint(&state.pool).await?;
            if checkpoint.as_ref().is_some_and(|row| {
                row.state == SummaryCompactedSourceRecoveryState::Failed.as_str()
            }) {
                bail!("test compacted source recovery entered failed state")
            }
            return Ok(());
        };
        if turn.next_turn == SummaryCoverageRecoveryNextTurn::Idle {
            break;
        }
    }
    bail!("test compacted source recovery did not complete within 128 turns")
}

/// Compatibility recovery for source writers predating the durable descriptor hook.  Bound the
/// read to the legal current prefix and leave a historical reconciliation marker so a missing
/// descriptor can never silently make an old range look exact.
async fn restore_legacy_summary_source_tail(state: &AppState) -> Result<bool> {
    let mut query = build_invocation_select_query();
    query.push(" ORDER BY occurred_at DESC, id DESC LIMIT ");
    query.push_bind(state.config.list_limit_max.min(400) as i64);
    let rows = query
        .build_query_as::<ApiInvocation>()
        .fetch_all(&state.pool)
        .await
        .context("legacy summary source tail reconstruction failed")?;
    let projection = state.subscription_hub.summary_projection().await;
    let Some(projection) = projection else {
        return Ok(false);
    };
    let mut restored = false;
    for row in rows {
        if projection.contains_persisted_live_terminal_identity(
            row.id,
            &row.invoke_id,
            &row.occurred_at,
        ) {
            continue;
        }
        let mut delta = persisted_dashboard_activity_terminal_delta(&row);
        delta.persisted_row_id = Some(row.id);
        state
            .subscription_hub
            .acknowledge_replayed_summary_delta(delta)
            .await;
        // A legacy writer can advance a historical row without a durable descriptor. Keep its
        // proof scoped to the row's time/account so an independent current selection remains
        // exact while the background historical reconciliation is in flight.
        state
            .subscription_hub
            .record_summary_source_change_scoped_gap(
                0,
                row.upstream_account_id,
                row.occurred_at.clone(),
                Some(row.id),
                Some(row.invoke_id.clone()),
            )
            .await;
        restored = true;
    }
    if restored {
        state
            .subscription_hub
            .mark_summary_projection_historical_recovery_required()
            .await;
    }
    Ok(restored)
}

pub(crate) async fn refresh_summary_snapshots_with_mode(
    state: &AppState,
    mode: SummaryProjectionBuildMode,
) -> Result<()> {
    if state.pg_analytics.backend() == AnalyticsBackend::Postgres {
        return Ok(());
    }
    if matches!(mode, SummaryProjectionBuildMode::AllTime) {
        return SummaryCoverageRecoverySupervisor::run(state).await;
    }
    if matches!(mode, SummaryProjectionBuildMode::RollingDelta) {
        let started = Instant::now();
        let durable_generation_fence = load_summary_projection_generation_fence(state).await?;
        if state
            .subscription_hub
            .renew_summary_projection_freshness_from_delta_journal_if_coverage_matches(
                durable_generation_fence,
            )
            .await
        {
            let (entry_count, gap_count) =
                state.subscription_hub.summary_delta_journal_counts().await;
            debug!(
                ?mode,
                stage = "delta_journal_reduce",
                elapsed_ms = started.elapsed().as_millis() as u64,
                entry_count,
                gap_count,
                "summary projection rolling delta renewed without live source admission"
            );
            return Ok(());
        }
        if renew_summary_projection_freshness_if_live_tail_matches(state).await? {
            debug!(
                ?mode,
                stage = "coverage_revision_renewal",
                "summary projection rolling freshness renewed without historical rebuild"
            );
            return Ok(());
        }
        let pressure_gate = crate::db_pressure::global_db_pressure_gate();
        let _reconciliation_permit = match pressure_gate
            .begin_priority_background_with_queue_wait(
                "summary_live_tail_reconciliation",
                SUMMARY_LIVE_TAIL_RECONCILIATION_QUEUE_WAIT,
            )
            .await
        {
            Ok(permit) => Arc::new(permit),
            Err(reason) => {
                debug!(
                    stage = "summary_live_tail_reconciliation_deferred",
                    reason = %reason,
                    "summary live-tail reconciliation deferred before durable reads"
                );
                record_summary_live_tail_readiness(
                    state,
                    "pressure_deferred",
                    "summary_live_tail_reconciliation_deferred",
                    0,
                    0,
                    0,
                    started,
                )
                .await;
                return Ok(());
            }
        };
        record_recovery(RecoveryDiagnostic::StatisticsLiveTail);
        record_summary_live_tail_readiness(
            state,
            "reconciling",
            "summary_live_tail_reconciliation_started",
            0,
            0,
            0,
            started,
        )
        .await;
        let (mut restored, source_target_fence) = restore_summary_source_change_tail(state).await?;
        if !restored
            && !state
                .subscription_hub
                .summary_source_compaction_gap_active()
                .await
        {
            restored = restore_legacy_summary_source_tail(state).await?;
        }
        #[cfg(test)]
        pause_summary_projection_test_interleave(
            &state.pool,
            mode,
            SummaryProjectionTestInterleaveStage::BeforeLiveTailPublication,
        )
        .await?;
        let live_tail_checkpoint =
            load_summary_live_tail_reconciliation_checkpoint(&state.pool).await?;
        let expected_revision = live_tail_checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.base_projection_revision.max(0) as u64)
            .unwrap_or_default();
        let Some(mut publication_transaction) =
            super::summary_live_tail_transaction::SummaryLiveTailTransaction::try_acquire(
                &state.pool,
                _reconciliation_permit.clone(),
            )
            .await?
        else {
            record_summary_live_tail_readiness(
                state,
                "write_admission_deferred",
                "summary_live_tail_publication_deferred",
                0,
                0,
                0,
                started,
            )
            .await;
            return Ok(());
        };
        publication_transaction.begin().await?;
        let publication_fence = load_summary_projection_generation_fence_tx(
            state,
            publication_transaction.connection_mut(),
        )
        .await?;
        let target_still_current =
            source_target_fence.is_none_or(|target| target == publication_fence);
        let reconciled_live_tail = source_target_fence.is_some()
            && target_still_current
            && state
                .subscription_hub
                .renew_summary_projection_after_live_tail_reconciliation_if_generation_matches(
                    expected_revision,
                    publication_fence,
                )
                .await;
        if source_target_fence.is_some() && !reconciled_live_tail {
            let projection_revision_conflict = state
                .subscription_hub
                .summary_projection()
                .await
                .is_none_or(|projection| projection.revision() != expected_revision);
            if !target_still_current || projection_revision_conflict {
                record_recovery(RecoveryDiagnostic::StatisticsPublicationConflict);
            }
        }
        if reconciled_live_tail {
            if let Some(checkpoint) = live_tail_checkpoint.as_ref()
                && checkpoint.target_source_cursor > 0
            {
                store_summary_source_change_checkpoint_tx(
                    publication_transaction.connection_mut(),
                    SUMMARY_SOURCE_CHANGE_CHECKPOINT_SCOPE,
                    checkpoint.target_source_cursor,
                )
                .await?;
                store_summary_live_tail_reconciliation_checkpoint_tx(
                    publication_transaction.connection_mut(),
                    SummaryLiveTailReconciliationCheckpoint {
                        format_version: checkpoint.format_version,
                        recovery_epoch: checkpoint.recovery_epoch,
                        base_projection_revision: checkpoint.base_projection_revision,
                        target_terminal_watermark: checkpoint.target_terminal_watermark,
                        target_source_cursor: checkpoint.target_source_cursor,
                        next_source_cursor: checkpoint.target_source_cursor,
                        state: "ready".to_string(),
                    },
                )
                .await?;
            }
            publication_transaction
                .finish(true)
                .await
                .context("commit Summary live-tail publication transaction")?;
            if let Some(checkpoint) = live_tail_checkpoint.as_ref()
                && checkpoint.target_source_cursor > 0
            {
                state
                    .subscription_hub
                    .advance_summary_source_change_cursor(checkpoint.target_source_cursor)
                    .await;
            }
        } else {
            publication_transaction
                .finish(false)
                .await
                .context("rollback stale Summary live-tail publication transaction")?;
        }
        let renewed_from_overlay = target_still_current
            && restored
            && state
                .subscription_hub
                .renew_summary_projection_freshness_from_delta_journal_if_coverage_matches(
                    load_summary_projection_generation_fence(state).await?,
                )
                .await;
        if reconciled_live_tail || renewed_from_overlay {
            let (entry_count, gap_count) =
                state.subscription_hub.summary_delta_journal_counts().await;
            debug!(
                ?mode,
                stage = "durable_source_tail_reconstruct",
                entry_count,
                gap_count,
                "summary projection rolling delta reconstructed from durable source descriptors"
            );
            let checkpoint = load_summary_live_tail_reconciliation_checkpoint(&state.pool)
                .await?
                .unwrap_or_default();
            record_summary_live_tail_readiness(
                state,
                "exact",
                "durable_source_tail_reconstruct",
                0,
                checkpoint.target_terminal_watermark,
                checkpoint.recovery_epoch as u64,
                started,
            )
            .await;
            return Ok(());
        }
        let live_tail_gap_count = state
            .subscription_hub
            .summary_delta_journal_gap_count()
            .await;
        if live_tail_gap_count > 0 {
            debug!(
                stage = "summary_live_tail_reconciliation_pending",
                gap_count = live_tail_gap_count,
                "summary live-tail gap remains bounded and fail-closed"
            );
            let checkpoint = load_summary_live_tail_reconciliation_checkpoint(&state.pool)
                .await?
                .unwrap_or_default();
            record_summary_live_tail_readiness(
                state,
                "live_tail_gap",
                "summary_live_tail_reconciliation_pending",
                live_tail_gap_count,
                checkpoint.target_terminal_watermark,
                checkpoint.recovery_epoch as u64,
                started,
            )
            .await;
            return Ok(());
        }
        // A coverage fence change belongs to the historical supervisor. Do not fall back to the
        // generic RollingDelta builder here: it performs live source admission and can turn a
        // bounded historical gap into a global refresh timeout. The already-published recent
        // projection remains usable while the supervisor attaches a verified coverage overlay.
        let _ = state
            .subscription_hub
            .revoke_summary_projection_stale_coverage(
                load_summary_projection_generation_fence(state).await?,
            )
            .await;
        state
            .subscription_hub
            .mark_summary_projection_historical_recovery_required()
            .await;
        record_summary_live_tail_readiness(
            state,
            "historical_coverage_pending",
            "summary_live_tail_historical_coverage_deferred",
            0,
            0,
            0,
            started,
        )
        .await;
        return Ok(());
    }
    let deadline = match mode {
        SummaryProjectionBuildMode::HistoricalLiveCoverage => {
            SUMMARY_PROJECTION_ALL_TIME_FINALIZATION_DEADLINE
        }
        SummaryProjectionBuildMode::Bootstrap
        | SummaryProjectionBuildMode::RollingDelta
        | SummaryProjectionBuildMode::Rolling => SUMMARY_PROJECTION_BUILD_DEADLINE,
        SummaryProjectionBuildMode::BootstrapFallback => {
            unreachable!("BootstrapFallback is only used after an admission failure")
        }
        SummaryProjectionBuildMode::AllTime => {
            unreachable!("AllTime is owned by coverage supervisor")
        }
    };
    refresh_summary_snapshots_with_deadline(state, mode, Some(deadline), true).await
}

#[cfg(test)]
pub(crate) async fn refresh_summary_snapshots_with_test_setup_deadline(
    state: &AppState,
    mode: SummaryProjectionBuildMode,
    setup_deadline: Duration,
    interleave: SummaryProjectionTestInterleave,
    stage_reached: std::sync::Arc<tokio::sync::Notify>,
) -> Result<()> {
    SUMMARY_PROJECTION_TEST_SETUP_DEADLINE
        .scope(
            (interleave, setup_deadline, stage_reached),
            refresh_summary_snapshots_with_deadline(state, mode, Some(setup_deadline), true),
        )
        .await
}

async fn refresh_summary_snapshots_with_deadline(
    state: &AppState,
    mode: SummaryProjectionBuildMode,
    deadline: Option<Duration>,
    coalesce_if_in_flight: bool,
) -> Result<()> {
    let Ok(_refresh_guard) = state.subscription_hub.try_lock_summary_projection_refresh() else {
        if coalesce_if_in_flight {
            debug!("summary projection refresh already in flight; coalescing trigger");
            return Ok(());
        }
        return Err(anyhow!(
            "summary projection startup hydration deferred because a refresh is already in flight"
        ));
    };
    let base_projection = state.subscription_hub.summary_projection().await;
    let expected_projection_revision = base_projection
        .as_ref()
        .map(|projection| projection.revision())
        .unwrap_or_default();
    let previous_all_time = base_projection.map(|projection| PreviousSummaryProjectionAllTime {
        all_time_by_account: projection.all_time_by_account.clone(),
        all_time_refreshed_at: projection.all_time_refreshed_at,
        all_time_manifest_admission_blocked_at: projection.all_time_manifest_admission_blocked_at,
        all_time_account_manifest_admission_blocked_at: projection
            .all_time_account_manifest_admission_blocked_at,
        all_time_account_refreshed_at: projection.all_time_account_refreshed_at.clone(),
        archive_account_ids_by_file: projection.archive_account_ids_by_file.clone(),
        archive_coverage_ranges_by_file: projection.archive_coverage_ranges_by_file.clone(),
        global_all_time_eligible: projection.freshness.global_all_time_eligible,
        account_all_time_eligible: projection.freshness.account_all_time_eligible.clone(),
        all_time_terminal_coverage_complete: projection.all_time_terminal_coverage_complete(),
        global_all_time_coverage_fence: projection.global_all_time_coverage_fence,
        account_all_time_coverage_fence: projection.account_all_time_coverage_fence,
        all_time_terminal_sequence_watermark: projection.all_time_terminal_sequence_watermark(),
        all_time_account_terminal_sequence_watermarks: projection
            .all_time_account_terminal_sequence_watermarks
            .clone(),
        all_time_account_persisted_live_terminal_invoke_ids: projection
            .all_time_account_persisted_live_terminal_invoke_ids
            .clone(),
        historical_live_coverage: projection.historical_live_coverage.clone(),
        coverage_overlay: projection.coverage_overlay.clone(),
        unavailable_exact_live_ranges: projection.unavailable_exact_live_ranges.clone(),
        unavailable_exact_live_account_ranges: projection
            .unavailable_exact_live_account_ranges
            .clone(),
        unavailable_unmaterialized_archive_ranges: projection
            .unavailable_unmaterialized_archive_ranges
            .clone(),
        unavailable_boundary_archive_ranges: projection.unavailable_boundary_archive_ranges.clone(),
        unavailable_unmaterialized_archive_current_ranges: projection
            .unavailable_unmaterialized_archive_current_ranges
            .clone(),
        unavailable_unmaterialized_archive_account_ranges: projection
            .unavailable_unmaterialized_archive_account_ranges
            .clone(),
        unavailable_boundary_archive_account_ranges: projection
            .unavailable_boundary_archive_account_ranges
            .clone(),
        unavailable_unmaterialized_archive_account_current_ranges: projection
            .unavailable_unmaterialized_archive_account_current_ranges
            .clone(),
        persisted_live_terminal_invoke_ids: projection.persisted_live_terminal_invoke_ids.clone(),
    });
    // Keep the publication CAS tied to the immutable base captured above. A slower generic
    // builder must not publish after a coverage overlay or another rolling refresh has swapped
    // the hub projection, even if it allocates a newer numeric revision at the end of the build.
    // The runtime read model advances this watermark only after SQLite ACKs every preceding
    // terminal in order. Capture it before the durable projection queries so a later successful
    // revision can prove that an overflowed Summary SSE overlay is no longer missing data.
    let durable_terminal_sequence_watermark = state
        .dashboard_activity_snapshot_cache
        .lock()
        .await
        .read_model
        .settled_terminal_sequence;
    let build_started_at = Instant::now();
    let fallback_previous_all_time = previous_all_time.clone();
    let build = build_summary_projection(
        state,
        mode,
        previous_all_time,
        durable_terminal_sequence_watermark,
    );
    let build_result = await_summary_projection_build(state, mode, deadline, build).await;
    let (projection, used_bootstrap_fallback) = match build_result {
        Ok(projection) => (projection, false),
        Err(error)
            if matches!(mode, SummaryProjectionBuildMode::Bootstrap)
                && summary_projection_bootstrap_fallback_error(&error) =>
        {
            warn!(
                error = ?error,
                "summary Bootstrap admission did not finish; publishing bounded fallback"
            );
            (
                build_summary_projection(
                    state,
                    SummaryProjectionBuildMode::BootstrapFallback,
                    fallback_previous_all_time,
                    durable_terminal_sequence_watermark,
                )
                .await?,
                true,
            )
        }
        Err(error) => return Err(error),
    };
    let projection_kind = if matches!(
        mode,
        SummaryProjectionBuildMode::HistoricalLiveCoverage | SummaryProjectionBuildMode::AllTime
    ) {
        crate::observability::contention::StatsProjection::SummaryHistorical
    } else {
        crate::observability::contention::StatsProjection::SummaryRecent
    };
    let pending_source_gaps = summary_projection_pending_source_gaps(&projection, projection_kind);
    info!(
        ?mode,
        stage = "generation_fence_snapshot",
        elapsed_ms = build_started_at.elapsed().as_millis() as u64,
        "summary projection build snapshot generation fence accepted"
    );
    if used_bootstrap_fallback {
        // The fallback is specifically used when the primary pool is under pressure. Do not
        // immediately re-enter that pool for an IMMEDIATE publication transaction: the immutable
        // snapshot already supplied the source fence, and the coverage supervisor will reconcile
        // any durable change before it publishes historical proof.
        if !state
            .subscription_hub
            .store_summary_projection_if_revision(projection, expected_projection_revision)
            .await
        {
            debug!(
                ?mode,
                "summary Bootstrap fallback publication lost its immutable base race"
            );
            return Ok(());
        }
        record_summary_projection_publication_metrics(projection_kind, pending_source_gaps);
        let generation_fence = state
            .subscription_hub
            .summary_projection()
            .await
            .map(|projection| projection.generation_fence());
        if let Some(generation_fence) = generation_fence {
            state
                .subscription_hub
                .mark_summary_bootstrap_fallback_fence(generation_fence)
                .await;
        }
        debug!(
            ?mode,
            elapsed_ms = build_started_at.elapsed().as_millis() as u64,
            "summary Bootstrap fallback published without a durable write transaction"
        );
        return Ok(());
    }
    // Serialize the final fence read and immutable hub swap against coverage trigger commits.
    // Without this short IMMEDIATE transaction, a proof/revoke can land between these two
    // operations and leave a stale all-time response published under the old fence.
    let mut publication_transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let durable_generation_fence =
        load_summary_projection_generation_fence_tx(state, publication_transaction.as_mut())
            .await?;
    if projection.generation_fence != durable_generation_fence {
        record_recovery(RecoveryDiagnostic::StatisticsPublicationConflict);
        // The read transaction may have opened before a coverage/proof commit. Never publish
        // that stale build. Revoke the affected all-time authority on the currently published
        // projection immediately so the memory-only handler cannot keep serving the old 200
        // while the next bounded refresh rebuilds against the new fence.
        if let Some(current) = state.subscription_hub.summary_projection().await {
            let mut current = Arc::unwrap_or_clone(current);
            let expected_revision = current.revision();
            if current.revoke_stale_all_time_coverage(durable_generation_fence) {
                state
                    .subscription_hub
                    .store_summary_projection_if_revision(current, expected_revision)
                    .await;
            }
        }
        debug!(
            ?mode,
            "summary projection build discarded because its durable generation fence advanced"
        );
        publication_transaction.rollback().await?;
        return Ok(());
    }
    let published = state
        .subscription_hub
        .store_summary_projection_if_revision(projection, expected_projection_revision)
        .await;
    if !published {
        record_recovery(RecoveryDiagnostic::StatisticsPublicationConflict);
        debug!(
            ?mode,
            "summary projection build discarded because its immutable base was replaced"
        );
        publication_transaction.commit().await?;
        return Ok(());
    }
    if used_bootstrap_fallback {
        let generation_fence = state
            .subscription_hub
            .summary_projection()
            .await
            .map(|projection| projection.generation_fence());
        if let Some(generation_fence) = generation_fence {
            state
                .subscription_hub
                .mark_summary_bootstrap_fallback_fence(generation_fence)
                .await;
        }
    } else {
        state
            .subscription_hub
            .clear_summary_bootstrap_fallback_fence()
            .await;
    }
    publication_transaction.commit().await?;
    record_summary_projection_publication_metrics(projection_kind, pending_source_gaps);
    debug!(
        ?mode,
        elapsed_ms = build_started_at.elapsed().as_millis() as u64,
        "summary projection build published"
    );
    Ok(())
}

async fn await_summary_projection_build(
    state: &AppState,
    mode: SummaryProjectionBuildMode,
    deadline: Option<Duration>,
    build: impl std::future::Future<Output = Result<SummaryProjection>>,
) -> Result<SummaryProjection> {
    #[cfg(test)]
    if matches!(mode, SummaryProjectionBuildMode::Rolling)
        && let Some((interleave, setup_deadline, stage_reached)) =
            SUMMARY_PROJECTION_TEST_SETUP_DEADLINE
                .try_with(|(interleave, setup_deadline, stage_reached)| {
                    (interleave.clone(), *setup_deadline, stage_reached.clone())
                })
                .ok()
    {
        return await_summary_projection_with_test_setup_deadline(
            build,
            setup_deadline,
            interleave,
            stage_reached,
        )
        .await;
    }
    match (mode, deadline) {
        (
            SummaryProjectionBuildMode::HistoricalLiveCoverage
            | SummaryProjectionBuildMode::AllTime,
            Some(deadline),
        ) => await_summary_projection_all_time_build(state, build, deadline).await,
        (_, Some(deadline)) => tokio::time::timeout(deadline, build)
            .await
            .map_err(|_| {
                #[cfg(test)]
                if matches!(mode, SummaryProjectionBuildMode::Bootstrap) {
                    let _ = BOOTSTRAP_TIMEOUT_ELAPSED.try_with(|elapsed| elapsed.set(true));
                }
                anyhow!("summary projection build exceeded {deadline:?}")
            })
            .and_then(|result| result),
        (_, None) => build.await,
    }
}

#[cfg(test)]
async fn await_summary_projection_with_test_setup_deadline(
    build: impl std::future::Future<Output = Result<SummaryProjection>>,
    setup_deadline: Duration,
    interleave: SummaryProjectionTestInterleave,
    stage_reached: std::sync::Arc<tokio::sync::Notify>,
) -> Result<SummaryProjection> {
    let mut build = Box::pin(build);
    let stage = interleave.wait_for_writer();
    let setup_timeout = tokio::time::sleep(setup_deadline);
    tokio::pin!(stage);
    tokio::pin!(setup_timeout);
    tokio::select! {
        result = &mut build => {
            stage_reached.notify_one();
            result
        }
        _ = &mut stage => {
            stage_reached.notify_one();
            tokio::time::timeout(SUMMARY_PROJECTION_BUILD_DEADLINE, &mut build)
                .await
                .map_err(|_| {
                    anyhow!(
                        "summary projection build exceeded {SUMMARY_PROJECTION_BUILD_DEADLINE:?}"
                    )
                })
                .and_then(|result| result)
        }
        _ = &mut setup_timeout => {
            stage_reached.notify_one();
            Err(anyhow!("summary projection setup exceeded {setup_deadline:?}"))
        }
    }
}

async fn await_summary_projection_all_time_build(
    state: &AppState,
    build: impl std::future::Future<Output = Result<SummaryProjection>>,
    deadline: Duration,
) -> Result<SummaryProjection> {
    let mut keepalive = tokio::time::interval_at(
        tokio::time::Instant::now() + SUMMARY_SNAPSHOT_MIN_REFRESH_INTERVAL,
        SUMMARY_SNAPSHOT_MIN_REFRESH_INTERVAL,
    );
    let timeout = tokio::time::sleep(deadline);
    tokio::pin!(build);
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            result = &mut build => return result,
            _ = &mut timeout => return Err(anyhow!("summary projection build exceeded {deadline:?}")),
            _ = keepalive.tick() => match renew_summary_projection_freshness_if_coverage_matches(state).await {
                Ok(true) => debug!("summary rolling projection kept fresh during all-time reconciliation"),
                Ok(false) => return Err(SummaryProjectionAllTimeGenerationChanged.into()),
                Err(error) => warn!(error = ?error, "summary all-time reconciliation could not renew rolling freshness"),
            },
        }
    }
}
