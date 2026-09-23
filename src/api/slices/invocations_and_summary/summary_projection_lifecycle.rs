use super::*;

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
        } else {
            state
                .subscription_hub
                .clear_summary_bootstrap_fallback_fence()
                .await;
            // A new live/manifest/coverage fence is a new admission identity. Give Bootstrap
            // exactly one chance to replace the fallback before the supervisor catches up.
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
}

#[derive(Debug)]
struct SummaryCoverageRecoveryTurn {
    next_turn: SummaryCoverageRecoveryNextTurn,
    reservation: Option<crate::db_pressure::DbBackgroundPriorityReservation>,
}

pub(crate) fn summary_coverage_recovery_next_turn_delay(
    next_turn: SummaryCoverageRecoveryNextTurn,
) -> Duration {
    match next_turn {
        SummaryCoverageRecoveryNextTurn::Immediate => Duration::ZERO,
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
    ) -> Result<SummaryCoverageRecoveryNextTurn> {
        Self::run_with_priority_reservation(state, Some(reservation)).await
    }

    pub(crate) async fn run_with_priority_reservation(
        state: &AppState,
        reservation: Option<crate::db_pressure::DbBackgroundPriorityReservation>,
    ) -> Result<SummaryCoverageRecoveryNextTurn> {
        let Ok(_singleflight) = state.subscription_hub.try_lock_summary_coverage_recovery() else {
            return Ok(SummaryCoverageRecoveryNextTurn::Idle);
        };
        Self::run_with_priority_reservation_locked(state, reservation).await
    }

    async fn run_with_priority_reservation_locked(
        state: &AppState,
        mut reservation: Option<crate::db_pressure::DbBackgroundPriorityReservation>,
    ) -> Result<SummaryCoverageRecoveryNextTurn> {
        let drain_startup_coverage = reservation.is_some();
        let mut unpublished_startup_pages = 0usize;
        let recovery = async {
            let mut retried_generation = false;
            loop {
                // The first turn must publish an already-proven recent range so 30d can become
                // exact independently.  After that, keep publication batch-bound: checking the
                // recent proof set alone is not enough because every newly verified page changes
                // the durable identity set, and cloning the immutable resident overlay for each
                // page can consume the entire historical recovery deadline.
                let publish_overlay = !drain_startup_coverage
                    || unpublished_startup_pages == 0
                    || (unpublished_startup_pages > 0
                        && unpublished_startup_pages.is_multiple_of(
                            SUMMARY_HISTORICAL_COVERAGE_OVERLAY_PUBLICATION_PAGE_BATCHES,
                        ));
                match Self::run_once(
                    state,
                    reservation.take(),
                    drain_startup_coverage,
                    publish_overlay,
                )
                .await
                {
                    Ok(turn) => {
                        let next_turn = turn.next_turn;
                        let Some(next_reservation) = turn.reservation else {
                            return Ok(next_turn);
                        };
                        reservation = Some(next_reservation);
                        unpublished_startup_pages = unpublished_startup_pages.saturating_add(1);
                        tokio::task::yield_now().await;
                    }
                    Err(error)
                        if error
                            .downcast_ref::<SummaryProjectionAllTimeGenerationChanged>()
                            .is_some()
                            && !retried_generation =>
                    {
                        retried_generation = true;
                        // V2 proof publication advances the coverage fence. Retry the bounded
                        // supervisor page against the new fence; never fall back to a generic
                        // RollingDelta or AllTime rebuild.
                        reservation = None;
                        unpublished_startup_pages = 0;
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }
        };
        tokio::pin!(recovery);
        // Do not renew the lease on the interval's immediate first tick.  A just-published
        // projection must still honor its recorded `refreshed_at`; the first keepalive is only
        // needed after one cadence interval while a long historical page is in flight.
        let mut renew = tokio::time::interval_at(
            tokio::time::Instant::now() + SUMMARY_SNAPSHOT_MAX_STALE / 3,
            SUMMARY_SNAPSHOT_MAX_STALE / 3,
        );
        loop {
            tokio::select! {
                result = &mut recovery => return result,
                _ = renew.tick() => {
                    if let Err(error) = renew_summary_projection_freshness_if_generation_matches(state).await {
                        debug!(error = ?error, "summary historical coverage freshness renewal deferred");
                    }
                }
            }
        }
    }

    async fn run_once(
        state: &AppState,
        reservation: Option<crate::db_pressure::DbBackgroundPriorityReservation>,
        _drain_startup_coverage: bool,
        publish_overlay: bool,
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
                    .begin_reserved_priority_background(reservation, queue_wait)
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
                return Ok(SummaryCoverageRecoveryTurn {
                    next_turn: SummaryCoverageRecoveryNextTurn::Idle,
                    reservation: None,
                });
            }
        };
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

        let backfill = if summary_coverage_recovery_requires_second_v2_turn(&priority_backfill) {
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
    tokio::spawn(async move {
        let mut next_turn =
            match SummaryCoverageRecoverySupervisor::run_with_startup_priority_reservation(
                state.as_ref(),
                startup_priority,
            )
            .await
            {
                Ok(next_turn) => next_turn,
                Err(error) => {
                    warn!(error = ?error, "initial summary historical coverage recovery deferred");
                    SummaryCoverageRecoveryNextTurn::Idle
                }
            };
        loop {
            let delay = summary_coverage_recovery_next_turn_delay(next_turn);
            tokio::select! {
                _ = state.shutdown.cancelled() => return,
                _ = tokio::time::sleep(delay) => {}
            }
            next_turn = match SummaryCoverageRecoverySupervisor::run_with_priority_reservation(
                state.as_ref(),
                None,
            )
            .await
            {
                Ok(next_turn) => next_turn,
                Err(error) => {
                    warn!(error = ?error, "summary historical coverage recovery deferred");
                    SummaryCoverageRecoveryNextTurn::Idle
                }
            };
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

async fn restore_summary_source_change_tail(state: &AppState) -> Result<bool> {
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
        return restore_compacted_summary_source_tail(state).await;
    }
    let tail = load_summary_source_change_tail(
        &state.pool,
        after_cursor,
        SUMMARY_SOURCE_CHANGE_JOURNAL_MAX_ENTRIES,
    )
    .await?;
    let Some(last_cursor) = tail.last().map(|record| record.cursor) else {
        return Ok(false);
    };
    let mut expected_cursor = after_cursor.saturating_add(1);
    for record in &tail {
        if record.cursor != expected_cursor {
            state
                .subscription_hub
                .record_summary_source_change_gap(expected_cursor)
                .await;
            return Ok(false);
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
        query.push(" WHERE id IN (");
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
    let base_projection_revision = state
        .subscription_hub
        .summary_projection()
        .await
        .map(|projection| projection.revision() as i64)
        .unwrap_or_default();
    let base_generation_fence = load_summary_projection_generation_fence(state).await?;
    let previous_epoch = load_summary_live_tail_reconciliation_checkpoint(&state.pool)
        .await?
        .map(|checkpoint| checkpoint.recovery_epoch)
        .unwrap_or_default();
    store_summary_live_tail_reconciliation_checkpoint(
        &state.pool,
        SummaryLiveTailReconciliationCheckpoint {
            format_version: 1,
            recovery_epoch: previous_epoch.saturating_add(1),
            base_projection_revision,
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
            base_projection_revision as u64,
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
                base_projection_revision,
                target_terminal_watermark,
                target_source_cursor: last_cursor,
                next_source_cursor: after_cursor,
                state: "deferred".to_string(),
            },
        )
        .await?;
    }
    Ok(restored || reconciled)
}

async fn restore_compacted_summary_source_tail(state: &AppState) -> Result<bool> {
    const MAX_REPLAY_DELTAS: usize = 10_000;
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations \
         WHERE LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')",
    )
    .fetch_one(&state.pool)
    .await
    .context("summary compacted source tail count failed")?;
    if count < 0 || count as usize > MAX_REPLAY_DELTAS {
        return Ok(false);
    }
    let mut query = build_invocation_select_query();
    query.push(" WHERE ");
    query.push(" LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending')");
    query.push(" ORDER BY occurred_at ASC, id ASC LIMIT ");
    query.push_bind(MAX_REPLAY_DELTAS as i64 + 1);
    let rows = query
        .build_query_as::<ApiInvocation>()
        .fetch_all(&state.pool)
        .await
        .context("summary compacted source tail reconstruction failed")?;
    if rows.len() != count as usize {
        return Ok(false);
    }
    let projection = state.subscription_hub.summary_projection().await;
    let Some(projection) = projection else {
        return Ok(false);
    };
    let (_, initial_gap_count) = state.subscription_hub.summary_delta_journal_counts().await;
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
        // Replay entries intentionally use sequence zero; their durable identity is sufficient
        // for exact deduplication while the ordered journal cursor is unavailable after compaction.
        state
            .subscription_hub
            .acknowledge_replayed_summary_delta(delta)
            .await;
    }
    let (_, gap_count) = state.subscription_hub.summary_delta_journal_counts().await;
    if gap_count == initial_gap_count {
        state
            .subscription_hub
            .clear_summary_source_compaction_gap()
            .await;
        return Ok(true);
    }
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

pub(crate) async fn refresh_summary_snapshots_with_mode(
    state: &AppState,
    mode: SummaryProjectionBuildMode,
) -> Result<()> {
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
            Ok(permit) => permit,
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
        let mut restored = restore_summary_source_change_tail(state).await?;
        if !restored {
            restored = restore_legacy_summary_source_tail(state).await?;
        }
        let live_tail_checkpoint =
            load_summary_live_tail_reconciliation_checkpoint(&state.pool).await?;
        let expected_revision = live_tail_checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.base_projection_revision.max(0) as u64)
            .unwrap_or_default();
        let mut publication_transaction = state
            .pool
            .begin()
            .await
            .context("begin Summary live-tail publication transaction")?;
        let publication_fence =
            load_summary_projection_generation_fence_tx(state, publication_transaction.as_mut())
                .await?;
        let reconciled_live_tail = state
            .subscription_hub
            .renew_summary_projection_after_live_tail_reconciliation_if_generation_matches(
                expected_revision,
                publication_fence,
            )
            .await;
        if reconciled_live_tail {
            if let Some(checkpoint) = live_tail_checkpoint.as_ref()
                && checkpoint.target_source_cursor > 0
            {
                store_summary_source_change_checkpoint_tx(
                    publication_transaction.as_mut(),
                    SUMMARY_SOURCE_CHANGE_CHECKPOINT_SCOPE,
                    checkpoint.target_source_cursor,
                )
                .await?;
                store_summary_live_tail_reconciliation_checkpoint_tx(
                    publication_transaction.as_mut(),
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
                .commit()
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
                .rollback()
                .await
                .context("rollback stale Summary live-tail publication transaction")?;
        }
        let renewed_from_overlay = restored
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
    let build_result = match (mode, deadline) {
        (
            SummaryProjectionBuildMode::HistoricalLiveCoverage
            | SummaryProjectionBuildMode::AllTime,
            Some(deadline),
        ) => await_summary_projection_all_time_build(state, build, deadline).await,
        (_, Some(deadline)) => tokio::time::timeout(deadline, build)
            .await
            .map_err(|_| anyhow!("summary projection build exceeded {deadline:?}"))
            .and_then(|result| result),
        (_, None) => build.await,
    };
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
    debug!(
        ?mode,
        elapsed_ms = build_started_at.elapsed().as_millis() as u64,
        "summary projection build published"
    );
    Ok(())
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
