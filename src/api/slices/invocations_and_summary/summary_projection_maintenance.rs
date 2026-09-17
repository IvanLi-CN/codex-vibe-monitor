impl SummarySnapshotEntry {
    pub(crate) fn ready(response: StatsResponse) -> Self {
        Self {
            response: Some(response),
            refreshed_at: Some(Instant::now()),
        }
    }

    pub(crate) fn fresh_response(&self) -> Option<StatsResponse> {
        self.refreshed_at
            .filter(|refreshed_at| refreshed_at.elapsed() <= SUMMARY_SNAPSHOT_MAX_STALE)
            .and_then(|_| self.response.clone())
    }

    pub(crate) fn needs_refresh(&self) -> bool {
        self.fresh_response().is_none()
    }
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
    // Historical coverage is durable recovery work, not demand-driven query work. A 30-day
    // proof must keep progressing even while no client happens to hold an `all` subscription.
    if let Err(error) = SummaryCoverageRecoverySupervisor::run(state).await {
        if error
            .downcast_ref::<SummaryProjectionAllTimeGenerationChanged>()
            .is_some()
        {
            debug!(
                "summary historical coverage recovery observed a generation change; keeping rolling projection"
            );
        } else {
            warn!(error = ?error, "summary historical coverage recovery deferred; keeping fresh rolling projection");
        }
    }
    Ok(())
}

/// Owns historical Summary recovery independently from the availability-critical rolling
/// projection. Every pass advances at most one bounded proof/checkpoint page, then gives the
/// Snapshot V2 backfill a chance to recover the next missing authority. The generic projection
/// builder is intentionally never used here: archive raw hydration belongs to this supervisor.
pub(crate) struct SummaryCoverageRecoverySupervisor;

async fn summary_all_time_checkpoint_publication_required(
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
        Self::run_with_priority_reservation(state, None).await
    }

    pub(crate) async fn run_with_startup_priority_reservation(
        state: &AppState,
        reservation: crate::db_pressure::DbBackgroundPriorityReservation,
    ) -> Result<()> {
        Self::run_with_priority_reservation(state, Some(reservation)).await
    }

    async fn run_with_priority_reservation(
        state: &AppState,
        reservation: Option<crate::db_pressure::DbBackgroundPriorityReservation>,
    ) -> Result<()> {
        let Ok(_singleflight) = state.subscription_hub.try_lock_summary_coverage_recovery() else {
            return Ok(());
        };
        Self::run_with_priority_reservation_locked(state, reservation).await
    }

    async fn run_with_priority_reservation_locked(
        state: &AppState,
        mut reservation: Option<crate::db_pressure::DbBackgroundPriorityReservation>,
    ) -> Result<()> {
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
                    Ok(Some(next_reservation)) => {
                        reservation = Some(next_reservation);
                        unpublished_startup_pages = unpublished_startup_pages.saturating_add(1);
                        tokio::task::yield_now().await;
                    }
                    Ok(None) => return Ok(()),
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

    async fn publish_summary_coverage_overlay_for_recovery(
        state: &AppState,
        force_full_reduction: bool,
    ) -> Result<()> {
        let publication_started_at = Instant::now();
        match publish_summary_coverage_overlay(state, force_full_reduction).await {
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
        Ok(())
    }

    async fn run_summary_coverage_priority_backfill(
        state: &AppState,
        publish_overlay: bool,
    ) -> Result<Option<crate::db_pressure::DbBackgroundPriorityReservation>> {
        let started_at = Instant::now();
        let backfill = backfill_summary_archive_snapshots_v2_window(
            &state.pool,
            SUMMARY_HISTORICAL_COVERAGE_BACKFILL_BUDGET,
        )
        .await?;
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
        let pending = backfill.recent_candidate_count > 0;
        let recent_overlay_needed =
            !pending && summary_projection_recent_coverage_needs_overlay(state).await;
        if publish_overlay
            && (recent_overlay_needed || (backfill.pending_obligation_count == 0 && pending))
        {
            Self::publish_summary_coverage_overlay_for_recovery(
                state,
                backfill.pending_obligation_count == 0,
            )
            .await?;
        }
        if backfill.pending_obligation_count > 0 {
            return Ok(Some(
                crate::db_pressure::global_db_pressure_gate().reserve_priority_background(),
            ));
        }
        Ok(None)
    }

    async fn publish_summary_coverage_checkpoint_if_ready(
        state: &AppState,
        started_at: Instant,
    ) -> Result<()> {
        if let Some(global_checkpoint) =
            load_summary_all_time_projection_checkpoint(&state.pool).await?
            && global_checkpoint.global_ready()
            && summary_all_time_checkpoint_publication_required(state, &global_checkpoint).await?
        {
            let publication_started_at = Instant::now();
            match tokio::time::timeout(
                SUMMARY_PROJECTION_ALL_TIME_FINALIZATION_DEADLINE,
                publish_summary_all_time_projection_checkpoint(state, global_checkpoint),
            )
            .await
            {
                Ok(result) => result?,
                Err(_) => warn!(
                    stage = "historical_coverage_global_projection_publish_deferred",
                    elapsed_ms = publication_started_at.elapsed().as_millis() as u64,
                    "summary global all-time projection publication exceeded its bounded turn"
                ),
            }
        }
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
        if !(checkpoint.global_ready() || checkpoint.account_ready())
            || !summary_all_time_checkpoint_publication_required(state, &checkpoint).await?
        {
            return Ok(());
        }
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
        Ok(())
    }

    async fn finish_summary_coverage_recovery_turn(
        state: &AppState,
        started_at: Instant,
    ) -> Result<()> {
        let backfill = backfill_summary_archive_snapshots_v2_window(
            &state.pool,
            SUMMARY_HISTORICAL_COVERAGE_BACKFILL_BUDGET,
        )
        .await?;
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
        if started_at.elapsed() < SUMMARY_PROJECTION_ALL_TIME_FINALIZATION_DEADLINE {
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
        if backfill.pending_obligation_count == 0 {
            Self::publish_summary_coverage_overlay_for_recovery(state, false).await?;
        }
        Ok(())
    }

    async fn run_once(
        state: &AppState,
        reservation: Option<crate::db_pressure::DbBackgroundPriorityReservation>,
        _drain_startup_coverage: bool,
        publish_overlay: bool,
    ) -> Result<Option<crate::db_pressure::DbBackgroundPriorityReservation>> {
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
                return Ok(None);
            }
        };
        let started_at = Instant::now();
        if let Some(reservation) =
            Self::run_summary_coverage_priority_backfill(state, publish_overlay).await?
        {
            return Ok(Some(reservation));
        }
        Self::publish_summary_coverage_checkpoint_if_ready(state, started_at).await?;

        Self::finish_summary_coverage_recovery_turn(state, started_at).await?;
        Ok(None)
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
