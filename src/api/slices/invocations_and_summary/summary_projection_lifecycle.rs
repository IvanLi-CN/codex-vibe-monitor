use super::*;
use crate::observability::contention::{
    PerformanceCounter, PerformanceGauge, RecoveryDiagnostic, record_recovery,
};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(test)]
#[path = "summary_projection_compacted_source_recovery_budget_tests.rs"]
mod compacted_source_recovery_budget_tests;
#[path = "summary_projection_compacted_source_recovery_model.rs"]
mod compacted_source_recovery_model;
#[path = "summary_projection_compacted_source_recovery_publication.rs"]
mod compacted_source_recovery_publication;
#[path = "summary_projection_compacted_source_recovery_query.rs"]
mod compacted_source_recovery_query;
#[cfg(test)]
#[path = "summary_projection_compacted_source_recovery_tests.rs"]
mod compacted_source_recovery_tests;
#[path = "summary_projection_lifecycle_build_wait.rs"]
mod lifecycle_build_wait;
#[cfg(test)]
#[path = "summary_projection_lifecycle_test_support.rs"]
mod lifecycle_test_support;

use compacted_source_recovery_model::*;
use compacted_source_recovery_publication::*;
use compacted_source_recovery_query::*;
use lifecycle_build_wait::refresh_summary_snapshots_with_deadline;

pub(crate) use compacted_source_recovery_model::compacted_source_recovery_window_ranges_match_at;
#[cfg(test)]
pub(crate) use compacted_source_recovery_tests::{
    explain_compacted_source_recovery_key_page_for_test,
    run_compacted_source_recovery_one_page_for_test,
    run_compacted_source_recovery_until_complete_for_test,
};
pub(crate) use lifecycle_build_wait::refresh_summary_snapshots_with_mode;
#[cfg(test)]
pub(super) use lifecycle_test_support::observe_bootstrap_timeout;
#[cfg(test)]
pub(crate) use lifecycle_test_support::{
    await_summary_projection_with_test_setup_deadline, mark_bootstrap_timeout,
    note_summary_coverage_checkpoint_advance, note_summary_coverage_v2_window,
    note_summary_projection_boundary_archive_hydration,
    note_summary_projection_historical_identity_hydration,
    refresh_summary_snapshots_with_test_setup_deadline, summary_projection_test_setup_deadline,
};

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
    refresh_summary_snapshots_with_deadline(
        state,
        SummaryProjectionBuildMode::Bootstrap,
        Some(deadline),
        false,
    )
    .await
}

pub(crate) async fn refresh_summary_snapshots(state: &AppState) -> Result<()> {
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
