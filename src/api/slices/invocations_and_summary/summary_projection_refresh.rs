pub(crate) async fn refresh_summary_snapshots_with_mode(
    state: &AppState,
    mode: SummaryProjectionBuildMode,
) -> Result<()> {
    if matches!(mode, SummaryProjectionBuildMode::AllTime) {
        return SummaryCoverageRecoverySupervisor::run(state).await;
    }
    if matches!(mode, SummaryProjectionBuildMode::RollingDelta) {
        return refresh_summary_snapshots_with_rolling_delta(state).await;
    }
    let deadline = match mode {
        SummaryProjectionBuildMode::HistoricalLiveCoverage => {
            SUMMARY_PROJECTION_ALL_TIME_FINALIZATION_DEADLINE
        }
        SummaryProjectionBuildMode::Bootstrap
        | SummaryProjectionBuildMode::RollingDelta
        | SummaryProjectionBuildMode::Rolling => SUMMARY_PROJECTION_BUILD_DEADLINE,
        SummaryProjectionBuildMode::AllTime => {
            unreachable!("AllTime is owned by coverage supervisor")
        }
    };
    refresh_summary_snapshots_with_deadline(state, mode, Some(deadline), true).await
}

async fn refresh_summary_snapshots_with_rolling_delta(state: &AppState) -> Result<()> {
    let started = Instant::now();
    let durable_generation_fence = load_summary_projection_generation_fence(state).await?;
    if state
        .subscription_hub
        .renew_summary_projection_freshness_from_delta_journal_if_coverage_matches(
            durable_generation_fence,
        )
        .await
    {
        let (entry_count, gap_count) = state.subscription_hub.summary_delta_journal_counts().await;
        debug!(
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
    let (restored, reconciled_live_tail) = reconcile_summary_live_tail(state).await?;
    if finish_summary_live_tail_refresh(state, started, restored, reconciled_live_tail).await? {
        return Ok(());
    }
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
    Ok(())
}

async fn reconcile_summary_live_tail(state: &AppState) -> Result<(bool, bool)> {
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
    Ok((restored, reconciled_live_tail))
}

async fn finish_summary_live_tail_refresh(
    state: &AppState,
    started: Instant,
    restored: bool,
    reconciled_live_tail: bool,
) -> Result<bool> {
    let renewed_from_overlay = restored
        && state
            .subscription_hub
            .renew_summary_projection_freshness_from_delta_journal_if_coverage_matches(
                load_summary_projection_generation_fence(state).await?,
            )
            .await;
    if reconciled_live_tail || renewed_from_overlay {
        let (entry_count, gap_count) = state.subscription_hub.summary_delta_journal_counts().await;
        debug!(
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
        return Ok(true);
    }
    let live_tail_gap_count = state
        .subscription_hub
        .summary_delta_journal_gap_count()
        .await;
    if live_tail_gap_count == 0 {
        return Ok(false);
    }
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
    Ok(true)
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
    let SummaryProjectionRefreshBase {
        expected_projection_revision,
        previous_all_time,
        durable_terminal_sequence_watermark,
    } = load_summary_projection_refresh_base(state).await;
    let build_started_at = Instant::now();
    let build = build_summary_projection(
        state,
        mode,
        previous_all_time,
        durable_terminal_sequence_watermark,
    );
    let projection = match (mode, deadline) {
        (
            SummaryProjectionBuildMode::HistoricalLiveCoverage
            | SummaryProjectionBuildMode::AllTime,
            Some(deadline),
        ) => await_summary_projection_all_time_build(state, build, deadline).await?,
        (_, Some(deadline)) => tokio::time::timeout(deadline, build)
            .await
            .map_err(|_| anyhow!("summary projection build exceeded {deadline:?}"))??,
        (_, None) => build.await?,
    };
    publish_summary_projection_refresh_result(
        state,
        mode,
        expected_projection_revision,
        build_started_at,
        projection,
    )
    .await
}

struct SummaryProjectionRefreshBase {
    expected_projection_revision: u64,
    previous_all_time: Option<PreviousSummaryProjectionAllTime>,
    durable_terminal_sequence_watermark: u64,
}

async fn load_summary_projection_refresh_base(state: &AppState) -> SummaryProjectionRefreshBase {
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
    SummaryProjectionRefreshBase {
        expected_projection_revision,
        previous_all_time,
        durable_terminal_sequence_watermark,
    }
}

async fn publish_summary_projection_refresh_result(
    state: &AppState,
    mode: SummaryProjectionBuildMode,
    expected_projection_revision: u64,
    build_started_at: Instant,
    projection: SummaryProjection,
) -> Result<()> {
    info!(
        ?mode,
        stage = "generation_fence_snapshot",
        elapsed_ms = build_started_at.elapsed().as_millis() as u64,
        "summary projection build snapshot generation fence accepted"
    );
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
    if !state
        .subscription_hub
        .store_summary_projection_if_revision(projection, expected_projection_revision)
        .await
    {
        debug!(
            ?mode,
            "summary projection build discarded because its immutable base was replaced"
        );
        publication_transaction.commit().await?;
        return Ok(());
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

async fn load_summary_projection_rollup_live_cursor(pool: &Pool<Sqlite>) -> Result<i64> {
    let mut connection = pool.acquire().await?;
    load_invocation_summary_rollup_live_cursor_tx(&mut connection)
        .await
        .map_err(|error| {
            anyhow!("summary projection live rollup cursor hydration failed: {error:?}")
        })
}

async fn load_summary_projection_account_rollup_live_cursor(
    pool: &Pool<Sqlite>,
) -> Result<Option<i64>> {
    // Account activity v2 advances independently from the global invocation rollup.  A global
    // cursor therefore cannot be used to suppress an account-scoped exact row when this marker
    // is behind it.
    sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress \
         WHERE dataset = 'invocation_account_activity_v2_repair_live_cursor'",
    )
    .fetch_optional(pool)
    .await
    .context("summary projection account rollup cursor hydration failed")
}

async fn load_summary_projection_archive_account_ids(
    pool: &Pool<Sqlite>,
    range: ExactUtcRange,
) -> Result<HashSet<i64>> {
    if !crate::stats::sqlite_table_has_column(pool, "codex_invocations", "payload").await? {
        return Ok(HashSet::new());
    }
    let lower = db_occurred_at_lower_bound(range.start);
    let upper = db_occurred_at_upper_bound(range.end);
    // Keep the result itself bounded. DISTINCT prevents a high-row-count archive with only a
    // handful of accounts from consuming the exact-record budget during account discovery.
    let rows = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT DISTINCT CASE WHEN json_valid(payload) \
                    THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) \
                    END \
         FROM codex_invocations \
         WHERE occurred_at >= ?1 AND occurred_at < ?2 \
         LIMIT ?3",
    )
    .bind(lower)
    .bind(upper)
    .bind((summary_projection_exact_record_limit() + 1) as i64)
    .fetch_all(pool)
    .await
    .context("summary projection archive account discovery failed")?;
    if rows.len() > summary_projection_exact_record_limit() {
        return Err(anyhow!(
            "summary projection archive account discovery exceeded bounded account budget ({SUMMARY_PROJECTION_MAX_EXACT_RECORDS})"
        ));
    }
    Ok(rows
        .into_iter()
        .flatten()
        .filter(|account_id| *account_id > 0)
        .collect())
}

async fn load_summary_projection_archive_row_counts(
    pool: &Pool<Sqlite>,
    archive_paths: &[String],
) -> Result<HashMap<String, i64>> {
    if archive_paths.is_empty() {
        return Ok(HashMap::new());
    }

    let mut counts = HashMap::new();
    for archive_paths in archive_paths.chunks(SUMMARY_PROJECTION_ARCHIVE_MANIFEST_QUERY_CHUNK_SIZE)
    {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT file_path, row_count FROM archive_batches \
             WHERE dataset = 'codex_invocations' AND status = 'completed' AND file_path IN (",
        );
        {
            let mut separated = query.separated(", ");
            for path in archive_paths {
                separated.push_bind(path);
            }
        }
        query.push(")");
        counts.extend(
            query
                .build_query_as::<(String, i64)>()
                .fetch_all(pool)
                .await
                .context("summary projection archive row-count hydration failed")?,
        );
    }
    Ok(counts)
}

async fn load_summary_projection_archive_manifest_sha256(
    pool: &Pool<Sqlite>,
    archive_paths: &[String],
) -> Result<HashMap<String, String>> {
    if archive_paths.is_empty() {
        return Ok(HashMap::new());
    }

    let expected_count = archive_paths.iter().collect::<HashSet<_>>().len();
    let mut sha256_by_file_path = HashMap::with_capacity(expected_count);
    for archive_paths in archive_paths.chunks(SUMMARY_PROJECTION_ARCHIVE_MANIFEST_QUERY_CHUNK_SIZE)
    {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT file_path, sha256 FROM archive_batches WHERE dataset = 'codex_invocations' \
             AND status = 'completed' AND file_path IN (",
        );
        {
            let mut separated = query.separated(", ");
            for path in archive_paths {
                separated.push_bind(path);
            }
        }
        query.push(")");
        for (file_path, sha256) in query
            .build_query_as::<(String, String)>()
            .fetch_all(pool)
            .await
            .context("summary projection archive manifest SHA hydration failed")?
        {
            if sha256.trim().is_empty()
                || sha256_by_file_path
                    .insert(file_path.clone(), sha256)
                    .is_some()
            {
                return Err(anyhow!(
                    "summary projection archive manifest SHA is missing or ambiguous for {file_path}"
                ));
            }
        }
    }
    if sha256_by_file_path.len() != expected_count {
        return Err(anyhow!(
            "summary projection archive manifest SHA hydration omitted snapshot rows"
        ));
    }
    Ok(sha256_by_file_path)
}

fn summary_projection_archive_file_is_readable(file_path: &str) -> bool {
    let Ok(file) = fs::File::open(file_path) else {
        return false;
    };
    let mut decoder = GzDecoder::new(file);
    io::copy(&mut decoder, &mut io::sink()).is_ok()
}
