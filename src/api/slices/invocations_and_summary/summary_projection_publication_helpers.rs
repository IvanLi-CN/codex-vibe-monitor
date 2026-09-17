struct SummaryProjectionCheckpointPublicationContext<'a> {
    state: &'a AppState,
    checkpoint: SummaryAllTimeProjectionCheckpointRow,
    generation_fence: SummaryProjectionGenerationFence,
    next: SummaryProjection,
    reduction_started_at: Instant,
    account_ids: HashSet<i64>,
    hourly_rollup_usage: HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    snapshot_totals: SummaryV2ArchiveTotals,
    hourly_rollup_totals: HashMap<(i64, Option<i64>), StatsTotals>,
}

async fn prepare_summary_projection_checkpoint_publication(
    state: &AppState,
    checkpoint: SummaryAllTimeProjectionCheckpointRow,
) -> Result<Option<SummaryProjectionCheckpointPublicationContext<'_>>> {
    if !checkpoint.global_ready() && !checkpoint.account_ready() {
        return Ok(None);
    }
    let (checkpoint, generation_fence, projection) =
        load_summary_projection_checkpoint_base(state, checkpoint).await?;
    if !summary_projection_checkpoint_archive_sources_ready(state).await? {
        return Ok(None);
    }
    let (snapshot_totals, hourly_rollup_usage, hourly_rollup_totals) =
        load_summary_projection_checkpoint_reduction_sources(
            state,
            &projection,
            generation_fence,
            &checkpoint,
        )
        .await?;
    Ok(Some(SummaryProjectionCheckpointPublicationContext {
        state,
        checkpoint,
        generation_fence,
        next: Arc::unwrap_or_clone(projection),
        reduction_started_at: Instant::now(),
        account_ids: HashSet::new(),
        hourly_rollup_usage,
        snapshot_totals,
        hourly_rollup_totals,
    }))
}

async fn load_summary_projection_checkpoint_base(
    state: &AppState,
    checkpoint: SummaryAllTimeProjectionCheckpointRow,
) -> Result<(
    SummaryAllTimeProjectionCheckpointRow,
    SummaryProjectionGenerationFence,
    Arc<SummaryProjection>,
)> {
    let observed = load_summary_projection_generation_fence(state).await?;
    let mut generation_fence = checkpoint.generation_fence();
    generation_fence.live_high_watermark_id = observed.live_high_watermark_id;
    generation_fence.rollup_live_cursor = observed.rollup_live_cursor;
    generation_fence.account_rollup_live_cursor = observed.account_rollup_live_cursor;
    generation_fence.durable_terminal_sequence_watermark =
        observed.durable_terminal_sequence_watermark;
    generation_fence.completed_manifest_high_watermark_id = observed
        .completed_manifest_high_watermark_id
        .or(generation_fence.completed_manifest_high_watermark_id);
    generation_fence.coverage_revision = observed.coverage_revision;
    generation_fence.account_coverage_revision = observed.account_coverage_revision;
    let mut projection = state
        .subscription_hub
        .summary_projection()
        .await
        .ok_or_else(|| {
            anyhow!("summary all-time checkpoint requires a published rolling projection")
        })?;
    if !projection
        .generation_fence()
        .coverage_sources_match(generation_fence)
    {
        let expected_revision = projection.revision();
        let mut current = Arc::unwrap_or_clone(projection);
        if current.revoke_stale_all_time_coverage(generation_fence) {
            state
                .subscription_hub
                .store_summary_projection_if_revision(current, expected_revision)
                .await;
        }
        projection = state
            .subscription_hub
            .summary_projection()
            .await
            .ok_or_else(|| {
                anyhow!("summary all-time checkpoint base disappeared during fence refresh")
            })?;
    }
    Ok((checkpoint, generation_fence, projection))
}

async fn summary_projection_checkpoint_archive_sources_ready(state: &AppState) -> Result<bool> {
    let paths = load_summary_projection_all_time_archive_scan_paths(&state.pool).await?;
    if paths.global_unmaterialized_count == 0 {
        return Ok(true);
    }
    let manifests =
        load_summary_projection_archive_manifest_sha256(&state.pool, &paths.global_unmaterialized)
            .await?;
    Ok(verify_summary_projection_archive_file_paths_sha256(
        &paths.global_unmaterialized,
        &manifests,
    )?
    .len()
        >= paths.global_unmaterialized_count)
}

async fn load_summary_projection_checkpoint_reduction_sources(
    state: &AppState,
    projection: &SummaryProjection,
    generation_fence: SummaryProjectionGenerationFence,
    checkpoint: &SummaryAllTimeProjectionCheckpointRow,
) -> Result<(
    SummaryV2ArchiveTotals,
    HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    HashMap<(i64, Option<i64>), StatsTotals>,
)> {
    let hourly_rollup_usage = if checkpoint.global_ready() || checkpoint.account_ready() {
        load_summary_projection_rollup_usage(&state.pool).await?
    } else {
        HashMap::new()
    };
    let mut snapshot_totals = if projection
        .coverage_overlay
        .as_ref()
        .is_some_and(|overlay| overlay.coverage_fence == generation_fence.coverage_fence())
    {
        projection
            .coverage_overlay
            .as_ref()
            .map(summary_v2_archive_totals_from_coverage_overlay)
            .unwrap_or_default()
    } else {
        load_summary_v2_archive_totals_excluding(&state.pool, &HashSet::new()).await?
    };
    let (materialized_buckets, materialized_months) =
        load_summary_materialized_archive_coverage(&state.pool).await?;
    snapshot_totals
        .materialized_buckets
        .extend(materialized_buckets);
    snapshot_totals
        .materialized_months_without_coverage
        .extend(materialized_months);
    let hourly_rollup_totals = if checkpoint.global_ready() || checkpoint.account_ready() {
        load_summary_projection_rollup_totals(&state.pool).await?.0
    } else {
        HashMap::new()
    };
    Ok((snapshot_totals, hourly_rollup_usage, hourly_rollup_totals))
}

async fn finalize_summary_projection_checkpoint_global(
    context: &mut SummaryProjectionCheckpointPublicationContext<'_>,
) -> Result<bool> {
    if !context.checkpoint.global_ready() {
        return Ok(true);
    }
    let live_high_watermark_id =
        load_summary_projection_live_high_watermark(&context.state.pool).await?;
    let observed_rollup_live_cursor =
        load_summary_projection_rollup_live_cursor(&context.state.pool).await?;
    let effective_rollup_live_cursor = context
        .generation_fence
        .rollup_live_cursor
        .max(observed_rollup_live_cursor);
    summary_all_time_projection_checkpoint_live_tail_count(
        &context.state.pool,
        effective_rollup_live_cursor,
        live_high_watermark_id,
        "global",
    )
    .await?;
    let live_tail = load_summary_projection_checkpoint_global_live_tail(
        &context.state.pool,
        effective_rollup_live_cursor,
        live_high_watermark_id,
    )
    .await?;
    let has_completed_archives =
        summary_projection_checkpoint_has_archives(&context.state.pool).await?;
    if has_completed_archives
        && context.generation_fence.live_high_watermark_id
            > context.generation_fence.rollup_live_cursor
        && context.generation_fence.rollup_live_cursor == 0
        && observed_rollup_live_cursor == 0
    {
        return Ok(false);
    }
    let totals = build_summary_projection_checkpoint_global_totals(
        context,
        live_tail,
        has_completed_archives,
        effective_rollup_live_cursor,
    )
    .await?;
    publish_summary_projection_checkpoint_global(context, totals, effective_rollup_live_cursor)
        .await?;
    Ok(true)
}

async fn load_summary_projection_checkpoint_global_live_tail(
    pool: &Pool<Sqlite>,
    lower_bound_id: i64,
    upper_bound_id: i64,
) -> Result<StatsTotals> {
    if upper_bound_id <= lower_bound_id {
        return Ok(StatsTotals::default());
    }
    crate::stats::query_live_invocation_totals_after_id(
        pool,
        InvocationSourceScope::All,
        lower_bound_id,
        upper_bound_id,
    )
    .await
    .map_err(|error| {
        anyhow!("summary all-time staged global live-tail hydration failed: {error:?}")
    })
}

async fn summary_projection_checkpoint_has_archives(pool: &Pool<Sqlite>) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM archive_batches WHERE dataset = 'codex_invocations' AND status = 'completed' AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror')",
    )
    .fetch_one(pool)
    .await
    .context("summary all-time archive presence lookup failed")?
        != 0)
}

async fn build_summary_projection_checkpoint_global_totals(
    context: &SummaryProjectionCheckpointPublicationContext<'_>,
    live_tail: StatsTotals,
    has_completed_archives: bool,
    effective_rollup_live_cursor: i64,
) -> Result<StatsTotals> {
    let mut totals = if has_completed_archives {
        context
            .hourly_rollup_totals
            .iter()
            .filter(|((_, account_id), _)| account_id.is_none())
            .fold(StatsTotals::default(), |totals, (_, value)| {
                totals.add(*value)
            })
            .add(live_tail)
    } else {
        return Ok(StatsTotals::from(
            crate::stats::query_stats_row_through_id(
                &context.state.pool,
                InvocationSourceScope::All,
                context.generation_fence.live_high_watermark_id,
            )
            .await
            .context("summary all-time live-only finalization failed")?,
        ));
    };
    let replacement_buckets = summary_projection_checkpoint_global_replacement_buckets(
        context,
        effective_rollup_live_cursor,
    );
    for bucket in &replacement_buckets {
        if let Some(rollup) = context.hourly_rollup_totals.get(&(*bucket, None)) {
            totals = subtract_summary_totals(totals, *rollup);
        }
    }
    for snapshot in context.snapshot_totals.global_by_bucket.values() {
        totals = totals.add(*snapshot);
    }
    add_summary_projection_checkpoint_global_exact_records(
        context,
        &mut totals,
        effective_rollup_live_cursor,
        &replacement_buckets,
        has_completed_archives,
    );
    Ok(totals)
}

fn summary_projection_checkpoint_global_replacement_buckets(
    context: &SummaryProjectionCheckpointPublicationContext<'_>,
    effective_rollup_live_cursor: i64,
) -> HashSet<i64> {
    let mut buckets = context.snapshot_totals.replacement_buckets.clone();
    buckets.retain(|bucket| {
        !context
            .snapshot_totals
            .materialized_buckets
            .contains(bucket)
            && Utc
                .timestamp_opt(*bucket, 0)
                .single()
                .is_none_or(|timestamp| {
                    !context
                        .snapshot_totals
                        .materialized_months_without_coverage
                        .contains(
                            &timestamp
                                .with_timezone(&Shanghai)
                                .format("%Y-%m")
                                .to_string(),
                        )
                })
    });
    for record in context
        .next
        .records
        .iter()
        .chain(context.next.current_records.iter())
    {
        if summary_projection_all_time_uses_global_exact_record(
            record,
            effective_rollup_live_cursor,
        ) && record.archive_has_materialized_rollups
        {
            buckets.insert(align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0));
        }
    }
    buckets
}

fn add_summary_projection_checkpoint_global_exact_records(
    context: &SummaryProjectionCheckpointPublicationContext<'_>,
    totals: &mut StatsTotals,
    effective_rollup_live_cursor: i64,
    replacement_buckets: &HashSet<i64>,
    has_completed_archives: bool,
) {
    let mut identities = HashSet::new();
    for record in context
        .next
        .records
        .iter()
        .chain(context.next.current_records.iter())
    {
        if record.is_persisted_live_record {
            if !has_completed_archives {
                continue;
            }
            let bucket = align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0);
            if record.row.id <= effective_rollup_live_cursor
                && context.hourly_rollup_totals.contains_key(&(bucket, None))
                && !replacement_buckets.contains(&bucket)
            {
                continue;
            }
            if !summary_projection_all_time_uses_global_exact_record(
                record,
                effective_rollup_live_cursor,
            ) {
                continue;
            }
        }
        if identities.insert((
            record.row.id,
            record.row.invoke_id.as_str(),
            record.row.occurred_at.as_str(),
        )) && summary_projection_all_time_uses_global_exact_record(
            record,
            effective_rollup_live_cursor,
        ) {
            *totals = totals.add(summary_projection_all_time_record_totals(record));
        }
    }
}

async fn publish_summary_projection_checkpoint_global(
    context: &mut SummaryProjectionCheckpointPublicationContext<'_>,
    totals: StatsTotals,
    effective_rollup_live_cursor: i64,
) -> Result<()> {
    let replacement_buckets = summary_projection_checkpoint_global_replacement_buckets(
        context,
        effective_rollup_live_cursor,
    );
    let mut usage = UsageBreakdownAccumulator::default();
    for (key, value) in &context.hourly_rollup_usage {
        if key.1.is_none() && !replacement_buckets.contains(&key.0) {
            usage.merge_response(value);
        }
    }
    for snapshot in context.snapshot_totals.global_usage_by_bucket.values() {
        usage.merge_response(&snapshot.clone().into_response());
    }
    let mut response = totals.into_response();
    let usage_response = usage.into_response();
    if usage_response.cache_read_tokens > 0
        || usage_response.cache_write_tokens > 0
        || usage_response.output_tokens > 0
        || usage_response.costs.is_some()
        || !usage_response.models.is_empty()
    {
        response.usage_breakdown = Some(usage_response);
    }
    response.non_success_cost = Some(totals.non_success_cost);
    response.maintenance = context.next.maintenance.clone();
    let in_progress = context
        .next
        .in_progress_by_account
        .get(&None)
        .copied()
        .unwrap_or_default();
    response.in_progress_conversation_count = Some(in_progress.in_progress_count);
    response.in_progress_retry_conversation_count = Some(in_progress.retry_count);
    response.in_progress_avg_wait_ms = in_progress.avg_wait_ms;
    response.in_progress_phase_counts = Some(in_progress.phase_counts);
    context.next.all_time_by_account.insert(None, response);
    context.next.all_time_refreshed_at = Some(context.reduction_started_at);
    context.next.freshness.global_all_time_eligible = true;
    context.next.global_all_time_coverage_fence = Some(context.generation_fence.coverage_fence());
    context.next.all_time_terminal_coverage_complete = true;
    context.next.all_time_terminal_sequence_watermark =
        context.generation_fence.durable_terminal_sequence_watermark;
    let overlay = context
        .state
        .subscription_hub
        .summary_projection_with_terminal_overlay(true, None)
        .await
        .ok()
        .flatten()
        .map(|(_, overlay, _, _)| overlay)
        .unwrap_or_default();
    update_summary_projection_checkpoint_global_terminal_state(context, &overlay);
    Ok(())
}

fn update_summary_projection_checkpoint_global_terminal_state(
    context: &mut SummaryProjectionCheckpointPublicationContext<'_>,
    overlay: &[DashboardActivityTerminalDelta],
) {
    for delta in overlay {
        if delta
            .persisted_row_id
            .is_some_and(|row_id| row_id <= context.generation_fence.live_high_watermark_id)
        {
            context.next.all_time_terminal_sequence_watermark = context
                .next
                .all_time_terminal_sequence_watermark
                .max(delta.terminal_sequence);
        }
    }
    context.next.all_time_persisted_live_terminal_invoke_ids = context
        .next
        .records
        .iter()
        .chain(context.next.current_records.iter())
        .filter(|record| record.is_persisted_live_record)
        .map(summary_projection_record_identity_key)
        .collect();
    for delta in overlay {
        if delta
            .persisted_row_id
            .is_some_and(|row_id| row_id <= context.generation_fence.live_high_watermark_id)
        {
            context
                .next
                .all_time_persisted_live_terminal_invoke_ids
                .insert(summary_projection_delta_identity_key(delta));
        }
    }
}

async fn finalize_summary_projection_checkpoint_accounts(
    context: &mut SummaryProjectionCheckpointPublicationContext<'_>,
) -> Result<()> {
    if !context.checkpoint.account_ready() {
        if context.checkpoint.account_unavailable != 0
            || context.checkpoint.account_manifest_complete == 0
        {
            context.next.all_time_account_manifest_admission_blocked_at =
                Some(context.reduction_started_at);
        }
        return Ok(());
    }
    let account_cursor = summary_projection_checkpoint_account_cursor(context).await?;
    summary_all_time_projection_checkpoint_live_tail_count(
        &context.state.pool,
        account_cursor,
        context.generation_fence.live_high_watermark_id,
        "account",
    )
    .await?;
    let mut account_totals =
        load_summary_projection_checkpoint_account_totals(context, account_cursor).await?;
    let account_replacement_buckets =
        summary_projection_checkpoint_account_replacement_buckets(context, account_cursor);
    add_summary_projection_checkpoint_account_exact_records(
        context,
        &mut account_totals,
        account_cursor,
    );
    if account_totals.len() > SUMMARY_PROJECTION_MAX_ACCOUNTS {
        return Err(anyhow!(
            "summary all-time staged account finalization exceeded bounded account budget ({})",
            account_totals.len()
        ));
    }
    publish_summary_projection_checkpoint_accounts(
        context,
        account_totals,
        account_replacement_buckets,
    )?;
    let overlay = context
        .state
        .subscription_hub
        .summary_projection_with_terminal_overlay(true, None)
        .await
        .ok()
        .flatten()
        .map(|(_, overlay, _, _)| overlay)
        .unwrap_or_default();
    update_summary_projection_checkpoint_account_terminal_state(context, &overlay);
    Ok(())
}

async fn summary_projection_checkpoint_account_cursor(
    context: &SummaryProjectionCheckpointPublicationContext<'_>,
) -> Result<i64> {
    let observed = load_summary_projection_account_rollup_live_cursor(&context.state.pool).await?;
    Ok(
        match (
            context.generation_fence.account_rollup_live_cursor,
            observed,
        ) {
            (Some(checkpoint), Some(observed)) => checkpoint.max(observed),
            (Some(cursor), None) | (None, Some(cursor)) => cursor,
            (None, None) => 0,
        },
    )
}

async fn load_summary_projection_checkpoint_account_totals(
    context: &mut SummaryProjectionCheckpointPublicationContext<'_>,
    account_cursor: i64,
) -> Result<HashMap<i64, StatsTotals>> {
    let mut totals = sqlx::query_as::<_, SummaryAllTimeCheckpointAccountTotalsRow>(
        "SELECT upstream_account_id, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost
         FROM summary_all_time_projection_account_checkpoint WHERE scope = ?1",
    )
    .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
    .fetch_all(&context.state.pool)
    .await
    .context("summary all-time projection account checkpoint finalization failed")?
    .into_iter()
    .map(|row| (row.upstream_account_id, row.totals()))
    .collect::<HashMap<_, _>>();
    if summary_projection_checkpoint_has_archives(&context.state.pool).await? {
        let buckets =
            summary_projection_checkpoint_account_replacement_buckets(context, account_cursor);
        totals =
            load_summary_projection_account_rollup_totals(&context.state.pool, &buckets).await?;
    }
    for (account_id, snapshot) in &context.snapshot_totals.accounts {
        let entry = totals.entry(*account_id).or_default();
        *entry = entry.add(*snapshot);
    }
    if context.generation_fence.live_high_watermark_id > account_cursor {
        for (account_id, value) in load_summary_projection_live_tail_account_totals(
            &context.state.pool,
            Some(account_cursor),
            Some(context.generation_fence.live_high_watermark_id),
        )
        .await?
        {
            let entry = totals.entry(account_id).or_default();
            *entry = entry.add(value);
        }
    }
    Ok(totals)
}

fn update_summary_projection_checkpoint_account_terminal_state(
    context: &mut SummaryProjectionCheckpointPublicationContext<'_>,
    overlay: &[DashboardActivityTerminalDelta],
) {
    for delta in overlay {
        let Some(row_id) = delta.persisted_row_id else {
            continue;
        };
        if row_id > context.generation_fence.live_high_watermark_id {
            continue;
        }
        let Some(account_id) = delta.upstream_account_id else {
            continue;
        };
        let watermark = context
            .next
            .all_time_account_terminal_sequence_watermarks
            .entry(account_id)
            .or_default();
        *watermark = (*watermark).max(delta.terminal_sequence);
        context
            .next
            .all_time_account_persisted_live_terminal_invoke_ids
            .entry(account_id)
            .or_default()
            .insert(summary_projection_delta_identity_key(delta));
    }
}

fn summary_projection_checkpoint_account_replacement_buckets(
    context: &SummaryProjectionCheckpointPublicationContext<'_>,
    account_cursor: i64,
) -> HashSet<i64> {
    let mut buckets = context.snapshot_totals.replacement_buckets.clone();
    buckets.retain(|bucket| {
        !context
            .snapshot_totals
            .materialized_buckets
            .contains(bucket)
            && Utc
                .timestamp_opt(*bucket, 0)
                .single()
                .is_none_or(|timestamp| {
                    !context
                        .snapshot_totals
                        .materialized_months_without_coverage
                        .contains(
                            &timestamp
                                .with_timezone(&Shanghai)
                                .format("%Y-%m")
                                .to_string(),
                        )
                })
    });
    for record in context
        .next
        .records
        .iter()
        .chain(context.next.current_records.iter())
    {
        if summary_projection_all_time_uses_account_exact_record(record, Some(account_cursor))
            && record.archive_has_materialized_rollups
        {
            buckets.insert(align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0));
        }
    }
    buckets
}

fn add_summary_projection_checkpoint_account_exact_records(
    context: &SummaryProjectionCheckpointPublicationContext<'_>,
    totals: &mut HashMap<i64, StatsTotals>,
    account_cursor: i64,
) {
    let live_tail_loaded = context.generation_fence.live_high_watermark_id > account_cursor;
    let mut identities = HashSet::new();
    for record in context
        .next
        .records
        .iter()
        .chain(context.next.current_records.iter())
    {
        let bucket = align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0);
        if record.is_persisted_live_record
            && record.row.id <= account_cursor
            && context
                .hourly_rollup_totals
                .contains_key(&(bucket, record.row.upstream_account_id))
        {
            continue;
        }
        if record.is_persisted_live_record && live_tail_loaded && record.row.id > account_cursor {
            continue;
        }
        if !identities.insert((
            record.row.id,
            record.row.invoke_id.as_str(),
            record.row.occurred_at.as_str(),
        )) || !summary_projection_all_time_uses_account_exact_record(
            record,
            Some(account_cursor),
        ) {
            continue;
        }
        let Some((account_id, record_totals)) = summary_projection_account_record_totals(record)
        else {
            continue;
        };
        let entry = totals.entry(account_id).or_default();
        *entry = entry.add(record_totals);
    }
}

fn publish_summary_projection_checkpoint_account(
    context: &mut SummaryProjectionCheckpointPublicationContext<'_>,
    account_id: i64,
    totals: StatsTotals,
    replacement_buckets: &HashSet<i64>,
) {
    let mut usage = UsageBreakdownAccumulator::default();
    for (key, value) in &context.hourly_rollup_usage {
        if key.1 == Some(account_id) && !replacement_buckets.contains(&key.0) {
            usage.merge_response(value);
        }
    }
    for bucket in &context.snapshot_totals.replacement_buckets {
        if let Some(snapshot) = context
            .snapshot_totals
            .account_usage_by_bucket
            .get(&(*bucket, account_id))
        {
            usage.merge_response(&snapshot.clone().into_response());
        }
    }
    let mut response = totals.into_response();
    let usage_response = usage.into_response();
    if usage_response.cache_read_tokens > 0
        || usage_response.cache_write_tokens > 0
        || usage_response.output_tokens > 0
        || usage_response.costs.is_some()
        || !usage_response.models.is_empty()
    {
        response.usage_breakdown = Some(usage_response);
    }
    response.non_success_cost = Some(totals.non_success_cost);
    response.maintenance = context.next.maintenance.clone();
    let in_progress = context
        .next
        .in_progress_by_account
        .get(&Some(account_id))
        .copied()
        .unwrap_or_default();
    response.in_progress_conversation_count = Some(in_progress.in_progress_count);
    response.in_progress_retry_conversation_count = Some(in_progress.retry_count);
    response.in_progress_avg_wait_ms = in_progress.avg_wait_ms;
    response.in_progress_phase_counts = Some(in_progress.phase_counts);
    context
        .next
        .all_time_by_account
        .insert(Some(account_id), response);
    context
        .next
        .all_time_account_refreshed_at
        .insert(account_id, context.reduction_started_at);
    context
        .next
        .freshness
        .account_all_time_eligible
        .insert(account_id);
    context
        .next
        .all_time_account_terminal_sequence_watermarks
        .entry(account_id)
        .and_modify(|watermark| {
            *watermark =
                (*watermark).max(context.generation_fence.durable_terminal_sequence_watermark)
        })
        .or_insert(context.generation_fence.durable_terminal_sequence_watermark);
    context.account_ids.insert(account_id);
}

fn publish_summary_projection_checkpoint_empty_account(
    context: &mut SummaryProjectionCheckpointPublicationContext<'_>,
    account_id: i64,
) {
    let in_progress = context
        .next
        .in_progress_by_account
        .get(&Some(account_id))
        .copied()
        .unwrap_or_default();
    let mut response = StatsTotals::default().into_response();
    response.non_success_cost = Some(0.0);
    response.maintenance = context.next.maintenance.clone();
    response.in_progress_conversation_count = Some(in_progress.in_progress_count);
    response.in_progress_retry_conversation_count = Some(in_progress.retry_count);
    response.in_progress_avg_wait_ms = in_progress.avg_wait_ms;
    response.in_progress_phase_counts = Some(in_progress.phase_counts);
    context
        .next
        .all_time_by_account
        .insert(Some(account_id), response);
    context
        .next
        .all_time_account_refreshed_at
        .insert(account_id, context.reduction_started_at);
    context
        .next
        .freshness
        .account_all_time_eligible
        .insert(account_id);
}

fn publish_summary_projection_checkpoint_accounts(
    context: &mut SummaryProjectionCheckpointPublicationContext<'_>,
    totals: HashMap<i64, StatsTotals>,
    replacement_buckets: HashSet<i64>,
) -> Result<()> {
    for (account_id, totals) in totals {
        publish_summary_projection_checkpoint_account(
            context,
            account_id,
            totals,
            &replacement_buckets,
        );
    }
    for account_id in context.next.known_account_ids.clone() {
        if context
            .next
            .all_time_by_account
            .contains_key(&Some(account_id))
        {
            continue;
        }
        publish_summary_projection_checkpoint_empty_account(context, account_id);
    }
    context
        .next
        .all_time_account_ids_with_projection_data
        .extend(context.account_ids.iter().copied());
    context
        .next
        .known_account_ids
        .extend(context.account_ids.iter().copied());
    context.next.all_time_account_manifest_admission_blocked_at = None;
    context.next.account_all_time_coverage_fence = Some(context.generation_fence.coverage_fence());
    Ok(())
}

async fn publish_summary_projection_checkpoint_finish(
    mut context: SummaryProjectionCheckpointPublicationContext<'_>,
) -> Result<()> {
    let published_at = Instant::now();
    if context.checkpoint.global_ready() {
        context.next.all_time_refreshed_at = Some(published_at);
    }
    if context.checkpoint.account_ready() {
        for refreshed_at in context.next.all_time_account_refreshed_at.values_mut() {
            *refreshed_at = published_at;
        }
    } else if context.checkpoint.account_unavailable != 0
        || context.checkpoint.account_manifest_complete == 0
    {
        context.next.all_time_account_manifest_admission_blocked_at = Some(published_at);
    }
    context.next.all_time_oldest_account_refreshed_at = context
        .next
        .all_time_account_refreshed_at
        .values()
        .copied()
        .min();
    let current_fence = load_summary_projection_generation_fence(context.state).await?;
    if !current_fence.coverage_sources_match(context.generation_fence)
        || !current_fence
            .live_tail_cursor()
            .terminal_sources_match(context.generation_fence.live_tail_cursor())
    {
        return Err(SummaryProjectionAllTimeGenerationChanged.into());
    }
    context.next.generation_fence = current_fence;
    let expected_revision = context.next.revision;
    if !publish_summary_projection_with_durable_coverage_fence(
        context.state,
        context.next,
        expected_revision,
        current_fence,
    )
    .await?
    {
        return Err(SummaryProjectionAllTimeGenerationChanged.into());
    }
    advance_summary_all_time_projection_checkpoint_live_fence(
        &context.state.pool,
        context.generation_fence,
    )
    .await?;
    debug!(
        global_ready = context.checkpoint.global_ready(),
        account_ready = context.checkpoint.account_ready(),
        "summary all-time staged projection published"
    );
    Ok(())
}
