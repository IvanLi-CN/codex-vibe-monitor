use super::*;

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

pub(super) async fn refresh_summary_snapshots_with_deadline(
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

pub(super) async fn await_summary_projection_build(
    state: &AppState,
    mode: SummaryProjectionBuildMode,
    deadline: Option<Duration>,
    build: impl std::future::Future<Output = Result<SummaryProjection>>,
) -> Result<SummaryProjection> {
    let build = Box::pin(build);
    #[cfg(test)]
    if matches!(mode, SummaryProjectionBuildMode::Rolling)
        && let Some((interleave, setup_deadline, stage_reached)) =
            summary_projection_test_setup_deadline()
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
                    mark_bootstrap_timeout();
                }
                anyhow!("summary projection build exceeded {deadline:?}")
            })
            .and_then(|result| result),
        (_, None) => build.await,
    }
}

pub(super) async fn await_summary_projection_all_time_build(
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
