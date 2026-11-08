use super::*;
pub(crate) async fn publish_summary_all_time_projection_checkpoint(
    state: &AppState,
    checkpoint: SummaryAllTimeProjectionCheckpointRow,
) -> Result<()> {
    if !checkpoint.global_ready() && !checkpoint.account_ready() {
        return Ok(());
    }
    let mut generation_fence = checkpoint.generation_fence();
    // Rollup repair and Snapshot proof commits can advance coverage revisions between the
    // checkpoint read and finalization. Use the latest immutable coverage fence for the
    // reduction; the checkpoint's live cursors remain the lower bounds for bounded tail reads.
    let observed_generation_fence = load_summary_projection_generation_fence(state).await?;
    // Finalization must reduce against one live watermark snapshot. The checkpoint may have
    // been created earlier, but publishing its old live fence after a newer terminal would let
    // the persisted-live set hide a row that was never included in the totals.
    generation_fence.live_high_watermark_id = observed_generation_fence.live_high_watermark_id;
    generation_fence.rollup_live_cursor = observed_generation_fence.rollup_live_cursor;
    generation_fence.account_rollup_live_cursor =
        observed_generation_fence.account_rollup_live_cursor;
    generation_fence.durable_terminal_sequence_watermark =
        observed_generation_fence.durable_terminal_sequence_watermark;
    generation_fence.completed_manifest_high_watermark_id = observed_generation_fence
        .completed_manifest_high_watermark_id
        .or(generation_fence.completed_manifest_high_watermark_id);
    generation_fence.coverage_revision = observed_generation_fence.coverage_revision;
    generation_fence.account_coverage_revision =
        observed_generation_fence.account_coverage_revision;
    let mut projection = state
        .subscription_hub
        .summary_projection()
        .await
        .ok_or_else(|| {
            anyhow!("summary all-time checkpoint requires a published rolling projection")
        })?;
    // A coverage revision may be committed while Bootstrap is still materializing its rolling
    // rows. Revoke the old historical authority before cloning the base so the finalizer can
    // publish against the current fence without retaining a stale V2 overlay or all-time 200.
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
    // An unmaterialized archive with only the global replay marker is not a complete Summary
    // authority: account and usage dimensions may still be missing. Keep the checkpoint
    // unpublished while its raw authority is unreadable; a later supervisor pass can retry it
    // when the source or the remaining replay proofs become available.
    let archive_scan_paths =
        load_summary_projection_all_time_archive_scan_paths(&state.pool).await?;
    if archive_scan_paths.global_unmaterialized_count > 0 {
        let manifest_sha256 = load_summary_projection_archive_manifest_sha256(
            &state.pool,
            &archive_scan_paths.global_unmaterialized,
        )
        .await?;
        if verify_summary_projection_archive_file_paths_sha256(
            &archive_scan_paths.global_unmaterialized,
            &manifest_sha256,
        )?
        .len()
            < archive_scan_paths.global_unmaterialized_count
        {
            return Ok(());
        }
    }
    let mut next = Arc::unwrap_or_clone(projection);
    // Freshness starts when the fully reduced projection is ready to swap, not when a potentially
    // long-running checkpoint reduction begins. Under SQLite contention the reduction can exceed
    // the serving freshness budget while still producing a current exact snapshot.
    let reduction_started_at = Instant::now();
    let mut account_ids = HashSet::new();

    let hourly_rollup_usage = if checkpoint.global_ready() || checkpoint.account_ready() {
        load_summary_projection_rollup_usage(&state.pool).await?
    } else {
        HashMap::new()
    };
    let mut snapshot_totals = if next.coverage_overlay.as_ref().is_some_and(|overlay| {
        // The overlay contains only verified archive contributions. Live-tail freshness is
        // fenced separately by the bounded query below, so a newer terminal must not force a
        // full V2 archive scan merely because the overlay was built at an older live cursor.
        overlay.coverage_fence == generation_fence.coverage_fence()
    }) {
        // The overlay is already the verified, immutable reduction of every V2 proof at this
        // fence. Reusing it keeps all-time publication bounded after recovery; rescanning every
        // snapshot page here was the source of the repeated 30-second finalization timeout.
        next.coverage_overlay
            .as_ref()
            .map(summary_v2_archive_totals_from_coverage_overlay)
            .unwrap_or_default()
    } else {
        load_summary_v2_archive_totals_excluding(&state.pool, &HashSet::new()).await?
    };
    // Materialized compact rollups are an aggregate baseline even when their archive has no V2
    // proof (or the archive file is unreadable). Merge only their bounded coverage metadata here;
    // never read the archive payload on the finalization path.
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
    if checkpoint.global_ready() {
        let live_high_watermark_id =
            load_summary_projection_live_high_watermark(&state.pool).await?;
        let observed_rollup_live_cursor =
            load_summary_projection_rollup_live_cursor(&state.pool).await?;
        let effective_rollup_live_cursor = generation_fence
            .rollup_live_cursor
            .max(observed_rollup_live_cursor);
        summary_all_time_projection_checkpoint_live_tail_count(
            &state.pool,
            effective_rollup_live_cursor,
            live_high_watermark_id,
            "global",
        )
        .await?;
        let live_tail = if live_high_watermark_id > effective_rollup_live_cursor {
            crate::stats::query_live_invocation_totals_after_id(
                &state.pool,
                InvocationSourceScope::All,
                effective_rollup_live_cursor,
                live_high_watermark_id,
            )
            .await
            .map_err(|error| {
                anyhow!("summary all-time staged global live-tail hydration failed: {error:?}")
            })?
        } else {
            StatsTotals::default()
        };
        let has_completed_archives = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM archive_batches WHERE dataset = 'codex_invocations' AND status = 'completed' AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror')",
        )
        .fetch_one(&state.pool)
        .await
        .context("summary all-time archive presence lookup failed")?
            != 0;
        if has_completed_archives
            && generation_fence.live_high_watermark_id > generation_fence.rollup_live_cursor
            && generation_fence.rollup_live_cursor == 0
            && observed_rollup_live_cursor == 0
        {
            // A compact historical baseline without a live cursor cannot prove whether any
            // retained live rows are already represented. Keep all-time fail-closed until the
            // background rollup/recovery owner establishes that boundary. If the repair marker
            // has advanced since this checkpoint was created, the bounded live-tail query above
            // is the exact replacement for the checkpoint's old zero cursor.
            return Ok(());
        }
        let mut totals = if has_completed_archives {
            // The checkpoint accumulator may predate an in-place rollup repair. Rebuild the
            // compact baseline from the current rollup rows so corrected classifications are
            // reflected, then add only rows newer than the effective rollup cursor.
            let rollup_baseline = hourly_rollup_totals
                .iter()
                .filter(|((_, account_id), _)| account_id.is_none())
                .fold(StatsTotals::default(), |totals, (_, value)| {
                    totals.add(*value)
                });
            rollup_baseline.add(live_tail)
        } else {
            StatsTotals::from(
                crate::stats::query_stats_row_through_id(
                    &state.pool,
                    InvocationSourceScope::All,
                    generation_fence.live_high_watermark_id,
                )
                .await
                .context("summary all-time live-only finalization failed")?,
            )
        };
        let mut exact_replacement_buckets = snapshot_totals.replacement_buckets.clone();
        // V2 pages from an unmaterialized sibling can share an hour with a materialized archive.
        // Keep the materialized rollup baseline in that mixed bucket; only subtract rollups for
        // buckets that have no independently proven materialized contribution.
        exact_replacement_buckets.retain(|bucket| {
            !snapshot_totals.materialized_buckets.contains(bucket)
                && Utc
                    .timestamp_opt(*bucket, 0)
                    .single()
                    .is_none_or(|timestamp| {
                        !snapshot_totals
                            .materialized_months_without_coverage
                            .contains(
                                &timestamp
                                    .with_timezone(&Shanghai)
                                    .format("%Y-%m")
                                    .to_string(),
                            )
                    })
        });
        // Legacy materialized manifests can carry only a local-month marker rather than exact
        // UTC bounds. In that case the compact rollup is the only safe baseline for the shared
        // bucket; keep it and add the independently proven sibling contribution instead of
        // subtracting the whole bucket as if the Snapshot replaced it.
        for record in next.records.iter().chain(next.current_records.iter()) {
            if summary_projection_all_time_uses_global_exact_record(
                record,
                effective_rollup_live_cursor,
            ) && record.archive_has_materialized_rollups
            {
                exact_replacement_buckets.insert(align_bucket_epoch(
                    record.occurred_at.timestamp(),
                    3_600,
                    0,
                ));
            }
        }
        for bucket in &exact_replacement_buckets {
            if let Some(rollup) = hourly_rollup_totals.get(&(*bucket, None)) {
                totals = subtract_summary_totals(totals, *rollup);
            }
        }
        for snapshot in snapshot_totals.global_by_bucket.values() {
            // V2 pages belong to unmaterialized archives and therefore add history that is not
            // present in the compact checkpoint. Replacement buckets were subtracted above so
            // their page totals are also added back exactly once; a sibling sharing a materialized
            // bucket still contributes its own rows without replacing that sibling's rollup.
            totals = totals.add(*snapshot);
        }
        let mut exact_record_identities = HashSet::new();
        for record in next.records.iter().chain(next.current_records.iter()) {
            // The global totals above already include every persisted live row through the
            // fenced live high-watermark (either via compact rollups or the bounded live tail).
            // Re-adding a live row from the resident projection would double-count it.
            if record.is_persisted_live_record {
                // In a live-only database the exact through-id query above is the complete
                // source, even when no rollup cursor has been established yet. The resident
                // records are still needed for rolling selections, but must not be folded into
                // the all-time aggregate a second time.
                if !has_completed_archives {
                    continue;
                }
                let bucket = align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0);
                // A fenced compact rollup is authoritative for live rows in its bucket even
                // when a hand-built repair fixture does not carry the per-record coverage bit.
                // Do not add that row again unless the bucket was explicitly replaced by an
                // exact archive contribution above.
                if record.row.id <= effective_rollup_live_cursor
                    && hourly_rollup_totals.contains_key(&(bucket, None))
                    && !exact_replacement_buckets.contains(&bucket)
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
            if !exact_record_identities.insert((
                record.row.id,
                record.row.invoke_id.as_str(),
                record.row.occurred_at.as_str(),
            )) || !summary_projection_all_time_uses_global_exact_record(
                record,
                effective_rollup_live_cursor,
            ) {
                continue;
            }
            totals = totals.add(summary_projection_all_time_record_totals(record));
        }
        let mut usage = UsageBreakdownAccumulator::default();
        for (key, value) in &hourly_rollup_usage {
            if key.1.is_none() && exact_replacement_buckets.contains(&key.0) {
                continue;
            }
            if key.1.is_none() {
                usage.merge_response(value);
            }
        }
        for snapshot in snapshot_totals.global_usage_by_bucket.values() {
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
        response.maintenance = next.maintenance.clone();
        let in_progress = next
            .in_progress_by_account
            .get(&None)
            .copied()
            .unwrap_or_default();
        response.in_progress_conversation_count = Some(in_progress.in_progress_count);
        response.in_progress_retry_conversation_count = Some(in_progress.retry_count);
        response.in_progress_avg_wait_ms = in_progress.avg_wait_ms;
        response.in_progress_phase_counts = Some(in_progress.phase_counts);
        next.all_time_by_account.insert(None, response);
        next.all_time_refreshed_at = Some(reduction_started_at);
        next.freshness.global_all_time_eligible = true;
        next.global_all_time_coverage_fence = Some(generation_fence.coverage_fence());
        next.all_time_terminal_coverage_complete = true;
        next.all_time_terminal_sequence_watermark =
            generation_fence.durable_terminal_sequence_watermark;
        let current_all_time_terminal_overlay = state
            .subscription_hub
            .summary_projection_with_terminal_overlay(true, None)
            .await
            .ok()
            .flatten()
            .map(|(_, overlay, _, _)| overlay)
            .unwrap_or_default();
        for delta in &current_all_time_terminal_overlay {
            if delta
                .persisted_row_id
                .is_some_and(|row_id| row_id <= generation_fence.live_high_watermark_id)
            {
                next.all_time_terminal_sequence_watermark = next
                    .all_time_terminal_sequence_watermark
                    .max(delta.terminal_sequence);
            }
        }
        next.all_time_persisted_live_terminal_invoke_ids = next
            .records
            .iter()
            .chain(next.current_records.iter())
            .filter(|record| record.is_persisted_live_record)
            .map(summary_projection_record_identity_key)
            .collect();
        // The bounded live-tail query above already includes committed overlay rows in the
        // exact aggregate. Mark their identities after rebuilding the resident set, otherwise
        // the HTTP all-time handler would append the same delta a second time after the swap.
        for delta in &current_all_time_terminal_overlay {
            if delta
                .persisted_row_id
                .is_some_and(|row_id| row_id <= generation_fence.live_high_watermark_id)
            {
                next.all_time_persisted_live_terminal_invoke_ids
                    .insert(summary_projection_delta_identity_key(delta));
            }
        }
    }

    if checkpoint.account_ready() {
        let observed_account_rollup_live_cursor =
            load_summary_projection_account_rollup_live_cursor(&state.pool).await?;
        let account_rollup_live_cursor = match (
            generation_fence.account_rollup_live_cursor,
            observed_account_rollup_live_cursor,
        ) {
            (Some(checkpoint_cursor), Some(observed_cursor)) => {
                checkpoint_cursor.max(observed_cursor)
            }
            (Some(cursor), None) | (None, Some(cursor)) => cursor,
            (None, None) => 0,
        };
        summary_all_time_projection_checkpoint_live_tail_count(
            &state.pool,
            account_rollup_live_cursor,
            generation_fence.live_high_watermark_id,
            "account",
        )
        .await?;
        let mut account_replacement_buckets = snapshot_totals.replacement_buckets.clone();
        account_replacement_buckets.retain(|bucket| {
            !snapshot_totals.materialized_buckets.contains(bucket)
                && Utc
                    .timestamp_opt(*bucket, 0)
                    .single()
                    .is_none_or(|timestamp| {
                        !snapshot_totals
                            .materialized_months_without_coverage
                            .contains(
                                &timestamp
                                    .with_timezone(&Shanghai)
                                    .format("%Y-%m")
                                    .to_string(),
                            )
                    })
        });
        for record in next.records.iter().chain(next.current_records.iter()) {
            if summary_projection_all_time_uses_account_exact_record(
                record,
                Some(account_rollup_live_cursor),
            ) && record.archive_has_materialized_rollups
            {
                account_replacement_buckets.insert(align_bucket_epoch(
                    record.occurred_at.timestamp(),
                    3_600,
                    0,
                ));
            }
        }
        let mut account_totals = sqlx::query_as::<_, SummaryAllTimeCheckpointAccountTotalsRow>(
            "SELECT upstream_account_id, total_count, success_count, failure_count, total_tokens, \
                    total_cost, non_success_cost \
             FROM summary_all_time_projection_account_checkpoint WHERE scope = ?1",
        )
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .fetch_all(&state.pool)
        .await
        .context("summary all-time projection account checkpoint finalization failed")?
        .into_iter()
        .map(|row| (row.upstream_account_id, row.totals()))
        .collect::<HashMap<_, _>>();
        let has_completed_archives = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM archive_batches WHERE dataset = 'codex_invocations' AND status = 'completed' AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror')",
        )
        .fetch_one(&state.pool)
        .await
        .context("summary all-time account archive presence lookup failed")?
            != 0;
        if has_completed_archives {
            account_totals = load_summary_projection_account_rollup_totals(
                &state.pool,
                &account_replacement_buckets,
            )
            .await?;
        }
        for (account_id, snapshot) in &snapshot_totals.accounts {
            let entry = account_totals.entry(*account_id).or_default();
            *entry = entry.add(*snapshot);
        }
        if generation_fence.live_high_watermark_id > account_rollup_live_cursor {
            for (account_id, totals) in load_summary_projection_live_tail_account_totals(
                &state.pool,
                Some(account_rollup_live_cursor),
                Some(generation_fence.live_high_watermark_id),
            )
            .await?
            {
                let entry = account_totals.entry(account_id).or_default();
                *entry = entry.add(totals);
            }
        }
        let current_all_time_terminal_overlay = state
            .subscription_hub
            .summary_projection_with_terminal_overlay(true, None)
            .await
            .ok()
            .flatten()
            .map(|(_, overlay, _, _)| overlay)
            .unwrap_or_default();
        for delta in &current_all_time_terminal_overlay {
            if delta
                .persisted_row_id
                .is_some_and(|row_id| row_id <= generation_fence.live_high_watermark_id)
                && let Some(account_id) = delta.upstream_account_id
            {
                let watermark = next
                    .all_time_account_terminal_sequence_watermarks
                    .entry(account_id)
                    .or_default();
                *watermark = (*watermark).max(delta.terminal_sequence);
                next.all_time_account_persisted_live_terminal_invoke_ids
                    .entry(account_id)
                    .or_default()
                    .insert(summary_projection_delta_identity_key(delta));
            }
        }
        let account_live_tail_loaded =
            generation_fence.live_high_watermark_id > account_rollup_live_cursor;
        let mut exact_record_identities = HashSet::new();
        for record in next.records.iter().chain(next.current_records.iter()) {
            let bucket = align_bucket_epoch(record.occurred_at.timestamp(), 3_600, 0);
            if record.is_persisted_live_record
                && record.row.id <= account_rollup_live_cursor
                && hourly_rollup_totals.contains_key(&(bucket, record.row.upstream_account_id))
            {
                continue;
            }
            // When the account cursor is absent, the bounded live-tail aggregate above owns
            // every persisted row through the fenced live high-watermark. Do not add the same
            // retained row again as an exact fallback; the fallback remains necessary only when
            // no live-tail aggregate was admitted for this generation.
            if record.is_persisted_live_record
                && account_live_tail_loaded
                && record.row.id > account_rollup_live_cursor
            {
                continue;
            }
            if !exact_record_identities.insert((
                record.row.id,
                record.row.invoke_id.as_str(),
                record.row.occurred_at.as_str(),
            )) || !summary_projection_all_time_uses_account_exact_record(
                record,
                Some(account_rollup_live_cursor),
            ) {
                continue;
            }
            let Some((account_id, totals)) = summary_projection_account_record_totals(record)
            else {
                continue;
            };
            let entry = account_totals.entry(account_id).or_default();
            *entry = entry.add(totals);
        }
        if account_totals.len() > SUMMARY_PROJECTION_MAX_ACCOUNTS {
            return Err(anyhow!(
                "summary all-time staged account finalization exceeded bounded account budget ({})",
                account_totals.len()
            ));
        }
        for (account_id, totals) in account_totals {
            let mut usage = UsageBreakdownAccumulator::default();
            for (key, value) in &hourly_rollup_usage {
                if key.1 == Some(account_id) && !account_replacement_buckets.contains(&key.0) {
                    usage.merge_response(value);
                }
            }
            for bucket in &snapshot_totals.replacement_buckets {
                if let Some(snapshot) = snapshot_totals
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
            response.maintenance = next.maintenance.clone();
            let in_progress = next
                .in_progress_by_account
                .get(&Some(account_id))
                .copied()
                .unwrap_or_default();
            response.in_progress_conversation_count = Some(in_progress.in_progress_count);
            response.in_progress_retry_conversation_count = Some(in_progress.retry_count);
            response.in_progress_avg_wait_ms = in_progress.avg_wait_ms;
            response.in_progress_phase_counts = Some(in_progress.phase_counts);
            next.all_time_by_account.insert(Some(account_id), response);
            next.all_time_account_refreshed_at
                .insert(account_id, reduction_started_at);
            next.freshness.account_all_time_eligible.insert(account_id);
            next.all_time_account_terminal_sequence_watermarks.insert(
                account_id,
                generation_fence.durable_terminal_sequence_watermark,
            );
            account_ids.insert(account_id);
        }
        // A complete account manifest also proves that a known account omitted from every
        // archive has an exact zero all-time response. Publish that bounded zero entry so a
        // previously unavailable account does not remain stuck after its manifest is repaired.
        for account_id in next.known_account_ids.clone() {
            if next.all_time_by_account.contains_key(&Some(account_id)) {
                continue;
            }
            let in_progress = next
                .in_progress_by_account
                .get(&Some(account_id))
                .copied()
                .unwrap_or_default();
            let mut response = StatsTotals::default().into_response();
            response.non_success_cost = Some(0.0);
            response.maintenance = next.maintenance.clone();
            response.in_progress_conversation_count = Some(in_progress.in_progress_count);
            response.in_progress_retry_conversation_count = Some(in_progress.retry_count);
            response.in_progress_avg_wait_ms = in_progress.avg_wait_ms;
            response.in_progress_phase_counts = Some(in_progress.phase_counts);
            next.all_time_by_account.insert(Some(account_id), response);
            next.all_time_account_refreshed_at
                .insert(account_id, reduction_started_at);
            next.freshness.account_all_time_eligible.insert(account_id);
        }
        next.all_time_account_ids_with_projection_data
            .extend(account_ids.iter().copied());
        next.known_account_ids.extend(account_ids.iter().copied());
        next.all_time_account_manifest_admission_blocked_at = None;
        next.account_all_time_coverage_fence = Some(generation_fence.coverage_fence());
    } else if checkpoint.account_unavailable != 0 || checkpoint.account_manifest_complete == 0 {
        // An account scope without a complete manifest/rollup proof is not an empty account;
        // retain a local unavailable marker so unknown or affected account selections fail closed
        // while the independent global projection remains exact.
        next.all_time_account_manifest_admission_blocked_at = Some(reduction_started_at);
    }

    // Only a projection that passes the final fence check is published. Record freshness at this
    // boundary so the serving budget measures the immutable snapshot's actual publication time.
    let published_at = Instant::now();
    if checkpoint.global_ready() {
        next.all_time_refreshed_at = Some(published_at);
    }
    if checkpoint.account_ready() {
        for refreshed_at in next.all_time_account_refreshed_at.values_mut() {
            *refreshed_at = published_at;
        }
    } else if checkpoint.account_unavailable != 0 || checkpoint.account_manifest_complete == 0 {
        next.all_time_account_manifest_admission_blocked_at = Some(published_at);
    }
    next.all_time_oldest_account_refreshed_at =
        next.all_time_account_refreshed_at.values().copied().min();
    let current_generation_fence = load_summary_projection_generation_fence(state).await?;
    if !current_generation_fence.coverage_sources_match(generation_fence)
        || !current_generation_fence
            .live_tail_cursor()
            .terminal_sources_match(generation_fence.live_tail_cursor())
    {
        return Err(SummaryProjectionAllTimeGenerationChanged.into());
    }
    // Keep the projection fence current for rolling freshness checks. The all-time terminal
    // watermark remains the checkpoint's proof watermark, so newly committed tail entries stay
    // visible through the in-memory all-time overlay until a later checkpoint absorbs them.
    next.generation_fence = current_generation_fence;
    let expected_revision = next.revision;
    if !publish_summary_projection_with_durable_coverage_fence(
        state,
        next,
        expected_revision,
        current_generation_fence,
    )
    .await?
    {
        return Err(SummaryProjectionAllTimeGenerationChanged.into());
    }
    // The immutable projection now contains the bounded tail through this fence. Persist the
    // live boundary only after the CAS succeeds so a failed/stale publication never skips rows;
    // a later supervisor pass can retry from the previous committed checkpoint.
    advance_summary_all_time_projection_checkpoint_live_fence(&state.pool, generation_fence)
        .await?;
    debug!(
        global_ready = checkpoint.global_ready(),
        account_ready = checkpoint.account_ready(),
        "summary all-time staged projection published"
    );
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SummaryProjectionOverflowedBoundaryCoverage {
    pub(crate) high_watermark_id: i64,
    pub(crate) unknown_coverage_ranges: Vec<ExactUtcRange>,
}

pub(crate) async fn summary_projection_overflowed_boundary_manifest_coverage(
    pool: &Pool<Sqlite>,
    exact_horizon: ExactUtcRange,
) -> Result<Option<SummaryProjectionOverflowedBoundaryCoverage>> {
    // The overflow path pages manifest metadata even when some partitions lack durable replay.
    // Bounded gaps are localized by the exact-range admission below. A legacy manifest without
    // bounds is localized from its immutable `month_key`; malformed keys retain the requested
    // horizon as the conservative fail-closed fallback.
    let Some(high_watermark_id) =
        summary_projection_completed_manifest_high_watermark(pool).await?
    else {
        return Ok(None);
    };
    let month_keys = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT batches.month_key FROM archive_batches AS batches \
         WHERE batches.dataset = 'codex_invocations' \
           AND batches.status = 'completed' \
           AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
           AND batches.id <= ?1 \
           AND (batches.coverage_start_at IS NULL OR batches.coverage_end_at IS NULL) \
         ORDER BY batches.month_key \
         LIMIT ?2",
    )
    .bind(high_watermark_id)
    .bind((SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES + 1) as i64)
    .fetch_all(pool)
    .await
    .context("summary projection overflowed boundary manifest coverage hydration failed")?;
    let mut unknown_coverage_ranges = Vec::new();
    let mut horizon_fallback_count = 0usize;
    for month_key in month_keys {
        let buckets = match crate::stats::archive_bucket_start_epochs_from_bounds(
            Some(&month_key),
            None,
            None,
        ) {
            Ok(buckets) if !buckets.is_empty() => buckets,
            // Without a parseable partition key, no finite source boundary exists. Preserve the
            // established fail-closed behavior, limited to the supported request horizon.
            Ok(_) | Err(_) => {
                horizon_fallback_count = horizon_fallback_count.saturating_add(1);
                unknown_coverage_ranges.push(exact_horizon);
                continue;
            }
        };
        let start_epoch = buckets
            .iter()
            .min()
            .copied()
            .expect("non-empty archive buckets");
        let end_epoch = buckets
            .iter()
            .max()
            .copied()
            .expect("non-empty archive buckets")
            + 3_600;
        let Some(start) = Utc.timestamp_opt(start_epoch, 0).single() else {
            horizon_fallback_count = horizon_fallback_count.saturating_add(1);
            unknown_coverage_ranges.push(exact_horizon);
            continue;
        };
        let Some(end) = Utc.timestamp_opt(end_epoch, 0).single() else {
            horizon_fallback_count = horizon_fallback_count.saturating_add(1);
            unknown_coverage_ranges.push(exact_horizon);
            continue;
        };
        let localized = ExactUtcRange {
            start: start.max(exact_horizon.start),
            end: end.min(exact_horizon.end),
        };
        if localized.start < localized.end {
            unknown_coverage_ranges.push(localized);
        }
    }
    let unknown_coverage_ranges = summary_projection_merge_exact_ranges(unknown_coverage_ranges);
    info!(
        stage = "overflowed_boundary_manifest_coverage_summary",
        unknown_range_count = unknown_coverage_ranges.len(),
        horizon_fallback_count,
        "summary projection overflowed boundary manifest coverage summarized"
    );
    Ok(Some(SummaryProjectionOverflowedBoundaryCoverage {
        high_watermark_id,
        unknown_coverage_ranges,
    }))
}

#[derive(Debug, Default)]
pub(crate) struct SummaryProjectionOverflowedBoundaryUnprovenRanges {
    pub(crate) global: Vec<ExactUtcRange>,
    pub(crate) account: Vec<ExactUtcRange>,
    pub(crate) broad_fail_closed: bool,
}

/// Return finite global and account ranges whose compact/V2 authority is not yet complete when
/// the boundary manifest cardinality is over the Bootstrap admission bound.  The optional
/// in-memory rollup maps let Bootstrap distinguish a missing compact key from a merely old
/// manifest without opening any archive source; the overlay rebuild uses metadata-only mode.
pub(crate) async fn summary_projection_overflowed_boundary_unproven_ranges_scoped(
    pool: &Pool<Sqlite>,
    exact_horizon: ExactUtcRange,
    high_watermark_id: i64,
    rollup_totals: Option<&HashMap<(i64, Option<i64>), StatsTotals>>,
    rollup_usage: Option<&HashMap<(i64, Option<i64>), UsageBreakdownResponse>>,
) -> Result<SummaryProjectionOverflowedBoundaryUnprovenRanges> {
    let rows = sqlx::query_as::<_, (
        String,
        String,
        Option<i64>,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )>(
        "SELECT month_key, file_path, \
                COALESCE(coverage_start_epoch, CAST(strftime('%s', coverage_start_at) AS INTEGER)), \
                COALESCE(coverage_end_epoch, CAST(strftime('%s', coverage_end_at) AS INTEGER)), \
                coverage_start_at, coverage_end_at, \
                historical_rollups_materialized_at, upstream_activity_manifest_refreshed_at \
         FROM archive_batches AS batches \
         WHERE batches.dataset = 'codex_invocations' \
           AND batches.status = 'completed' \
           AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
           AND batches.id <= ?1 \
           AND (batches.coverage_end_epoch IS NULL OR batches.coverage_end_epoch > ?2) \
           AND (batches.coverage_start_epoch IS NULL OR batches.coverage_start_epoch < ?3) \
           AND NOT EXISTS ( \
               SELECT 1 FROM summary_archive_snapshot_v2_proof AS proof \
               WHERE proof.archive_batch_id = batches.id \
                 AND batches.sha256 IS NOT NULL \
                 AND TRIM(batches.sha256) <> '' \
                 AND proof.manifest_sha256 = batches.sha256 \
           ) \
         LIMIT ?4",
    )
    .bind(high_watermark_id)
    .bind(exact_horizon.start.timestamp())
    .bind(exact_horizon.end.timestamp())
    .bind((SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES + 1) as i64)
    .fetch_all(pool)
    .await
    .context("summary projection overflowed boundary unproven-range hydration failed")?;

    if rows.len() > SUMMARY_PROJECTION_MAX_ARCHIVE_BATCHES {
        // An overflow can be caused by old manifests that are already represented by compact
        // replay. Evaluate each manifest's own buckets instead of taking a min/max span across
        // the backlog; sparse historical partitions must not poison recent selections.
        let paths = rows
            .iter()
            .map(|(_, file_path, _, _, _, _, _, _)| file_path.clone())
            .collect::<Vec<_>>();
        let replay_coverage = load_summary_projection_archive_replay_coverage(pool, &paths).await?;
        let mut global = Vec::new();
        let mut account = Vec::new();
        let mut broad_fail_closed = false;
        for (
            _month_key,
            file_path,
            coverage_start_epoch,
            coverage_end_epoch,
            coverage_start_at,
            coverage_end_at,
            historical_rollups_materialized_at,
            upstream_activity_manifest_refreshed_at,
        ) in &rows
        {
            let (Some(start_epoch), Some(end_epoch)) = (
                coverage_start_epoch.or_else(|| {
                    coverage_start_at
                        .as_deref()
                        .and_then(crate::stats::parse_to_utc_datetime)
                        .map(|value| value.timestamp())
                }),
                coverage_end_epoch.or_else(|| {
                    coverage_end_at
                        .as_deref()
                        .and_then(crate::stats::parse_to_utc_datetime)
                        .map(|value| value.timestamp())
                }),
            ) else {
                global.push(exact_horizon);
                account.push(exact_horizon);
                broad_fail_closed = true;
                continue;
            };
            let (Some(start), Some(end)) = (
                Utc.timestamp_opt(start_epoch, 0).single(),
                Utc.timestamp_opt(end_epoch, 0).single(),
            ) else {
                continue;
            };
            let localized = ExactUtcRange {
                start: start.max(exact_horizon.start),
                end: end
                    .checked_add_signed(ChronoDuration::seconds(1))
                    .unwrap_or(end)
                    .min(exact_horizon.end),
            };
            if localized.start >= localized.end {
                continue;
            }
            let replay = replay_coverage.get(file_path).copied().unwrap_or_default();
            let mut global_gap = historical_rollups_materialized_at.is_none()
                || !replay.overall
                || !replay.usage_breakdown;
            if let (Some(totals), Some(usage)) = (rollup_totals, rollup_usage) {
                let mut bucket = align_bucket_epoch(localized.start.timestamp(), 3_600, 0);
                let last_bucket =
                    align_bucket_epoch(localized.end.timestamp().saturating_sub(1), 3_600, 0);
                while bucket <= last_bucket {
                    if !totals.contains_key(&(bucket, None)) || !usage.contains_key(&(bucket, None))
                    {
                        global_gap = true;
                        break;
                    }
                    bucket = bucket.saturating_add(3_600);
                }
            } else {
                global_gap = true;
            }
            if global_gap {
                global.push(localized);
                broad_fail_closed = true;
            }
            if upstream_activity_manifest_refreshed_at.is_none()
                || !replay.account_stats
                || !replay.usage_breakdown
            {
                account.push(localized);
            }
        }
        let global = summary_projection_merge_exact_ranges(global);
        let mut account = summary_projection_merge_exact_ranges(account);
        let (unproven_manifest_exists, unproven_account_manifest_exists) =
            sqlx::query_as::<_, (i64, i64)>(
                "SELECT \
             EXISTS(SELECT 1 FROM archive_batches AS batches \
             WHERE batches.dataset = 'codex_invocations' \
               AND batches.status = 'completed' \
               AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
               AND batches.id <= ?1 \
               AND (batches.coverage_end_epoch IS NULL OR batches.coverage_end_epoch > ?2) \
               AND (batches.coverage_start_epoch IS NULL OR batches.coverage_start_epoch < ?3) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM summary_archive_snapshot_v2_proof AS proof \
                   WHERE proof.archive_batch_id = batches.id \
                     AND batches.sha256 IS NOT NULL \
                     AND TRIM(batches.sha256) <> '' \
                     AND proof.manifest_sha256 = batches.sha256 \
               ) \
               AND ( \
                   batches.historical_rollups_materialized_at IS NULL \
                   OR NOT EXISTS ( \
                       SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                       WHERE replay.target = ?4 \
                         AND replay.dataset = batches.dataset \
                         AND replay.file_path = batches.file_path \
                         AND replay.archive_sha256 = batches.sha256 \
                   ) \
                   OR NOT EXISTS ( \
                       SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                       WHERE replay.target = ?5 \
                         AND replay.dataset = batches.dataset \
                         AND replay.file_path = batches.file_path \
                         AND replay.archive_sha256 = batches.sha256 \
                   ) \
               )), \
             EXISTS(SELECT 1 FROM archive_batches AS batches \
             WHERE batches.dataset = 'codex_invocations' \
               AND batches.status = 'completed' \
               AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
               AND batches.id <= ?1 \
               AND (batches.coverage_end_epoch IS NULL OR batches.coverage_end_epoch > ?2) \
               AND (batches.coverage_start_epoch IS NULL OR batches.coverage_start_epoch < ?3) \
               AND NOT EXISTS ( \
                   SELECT 1 FROM summary_archive_snapshot_v2_proof AS proof \
                   WHERE proof.archive_batch_id = batches.id \
                     AND batches.sha256 IS NOT NULL \
                     AND TRIM(batches.sha256) <> '' \
                     AND proof.manifest_sha256 = batches.sha256 \
               ) \
               AND ( \
                   batches.upstream_activity_manifest_refreshed_at IS NULL \
                   OR NOT EXISTS ( \
                       SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                       WHERE replay.target = ?6 \
                         AND replay.dataset = batches.dataset \
                         AND replay.file_path = batches.file_path \
                         AND replay.archive_sha256 = batches.sha256 \
                   ) \
                   OR NOT EXISTS ( \
                       SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                       WHERE replay.target = ?5 \
                         AND replay.dataset = batches.dataset \
                         AND replay.file_path = batches.file_path \
                         AND replay.archive_sha256 = batches.sha256 \
                   ) \
               ))",
            )
            .bind(high_watermark_id)
            .bind(exact_horizon.start.timestamp())
            .bind(exact_horizon.end.timestamp())
            .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
            .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
            .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
            .fetch_one(pool)
            .await
            .context("summary projection overflowed boundary unproven aggregate lookup failed")?;
        let broad_fail_closed = broad_fail_closed || unproven_manifest_exists != 0;
        if unproven_account_manifest_exists != 0 {
            account.push(exact_horizon);
        }
        info!(
            stage = "overflowed_boundary_unproven_range_budget",
            source_row_count = rows.len(),
            "summary projection metadata scan exceeded its bounded archive budget; retaining a bounded unavailable range"
        );
        return Ok(SummaryProjectionOverflowedBoundaryUnprovenRanges {
            global,
            account,
            broad_fail_closed,
        });
    }

    let paths = rows
        .iter()
        .map(|(_, file_path, _, _, _, _, _, _)| file_path.clone())
        .collect::<Vec<_>>();
    let replay_coverage = load_summary_projection_archive_replay_coverage(pool, &paths).await?;
    let account_manifest = load_summary_projection_archive_manifest_accounts(pool, &paths)
        .await?
        .into_iter()
        .fold(
            HashMap::<String, HashSet<i64>>::new(),
            |mut by_file, (path, id)| {
                by_file.entry(path).or_default().insert(id);
                by_file
            },
        );

    let mut global_ranges = Vec::new();
    let mut account_ranges = Vec::new();
    let mut source_row_count = 0usize;
    let mut horizon_fallback_count = 0usize;
    for (
        month_key,
        file_path,
        coverage_start_epoch,
        coverage_end_epoch,
        coverage_start_at,
        coverage_end_at,
        historical_rollups_materialized_at,
        upstream_activity_manifest_refreshed_at,
    ) in rows
    {
        source_row_count = source_row_count.saturating_add(1);
        let range = match (
            coverage_start_at
                .as_deref()
                .and_then(crate::stats::parse_to_utc_datetime),
            coverage_end_at
                .as_deref()
                .and_then(crate::stats::parse_to_utc_datetime),
        ) {
            (Some(start), Some(end)) if start <= end => ExactUtcRange {
                start,
                // A point-like legacy manifest records the inclusive endpoint in both columns.
                // Preserve that one-second contribution as a finite unavailable range instead of
                // silently dropping it during half-open range normalization.
                end: if start == end {
                    end.checked_add_signed(ChronoDuration::seconds(1))
                        .unwrap_or(end)
                } else {
                    end
                },
            },
            _ => match (coverage_start_epoch, coverage_end_epoch) {
                (Some(start_epoch), Some(end_epoch)) => match (
                    Utc.timestamp_opt(start_epoch, 0).single(),
                    Utc.timestamp_opt(end_epoch, 0).single(),
                ) {
                    (Some(start), Some(end)) if start <= end => ExactUtcRange {
                        start,
                        end: end
                            .checked_add_signed(ChronoDuration::seconds(1))
                            .unwrap_or(end),
                    },
                    _ => {
                        horizon_fallback_count = horizon_fallback_count.saturating_add(1);
                        global_ranges.push(exact_horizon);
                        account_ranges.push(exact_horizon);
                        continue;
                    }
                },
                _ => {
                    let buckets = crate::stats::archive_bucket_start_epochs_from_bounds(
                        Some(&month_key),
                        None,
                        None,
                    )
                    .ok()
                    .filter(|buckets| !buckets.is_empty());
                    let Some(buckets) = buckets else {
                        horizon_fallback_count = horizon_fallback_count.saturating_add(1);
                        global_ranges.push(exact_horizon);
                        account_ranges.push(exact_horizon);
                        continue;
                    };
                    let start_epoch = *buckets.iter().min().expect("non-empty archive buckets");
                    let end_epoch = buckets
                        .iter()
                        .max()
                        .copied()
                        .unwrap_or(start_epoch)
                        .saturating_add(3_600);
                    let (Some(start), Some(end)) = (
                        Utc.timestamp_opt(start_epoch, 0).single(),
                        Utc.timestamp_opt(end_epoch, 0).single(),
                    ) else {
                        horizon_fallback_count = horizon_fallback_count.saturating_add(1);
                        global_ranges.push(exact_horizon);
                        account_ranges.push(exact_horizon);
                        continue;
                    };
                    ExactUtcRange { start, end }
                }
            },
        };
        let localized = ExactUtcRange {
            start: range.start.max(exact_horizon.start),
            end: range.end.min(exact_horizon.end),
        };
        if localized.start < localized.end {
            let replay = replay_coverage.get(&file_path).copied().unwrap_or_default();
            let mut global_gap = !replay.overall || !replay.usage_breakdown;
            let mut account_gap = upstream_activity_manifest_refreshed_at.is_none()
                || !replay.account_stats
                || !replay.usage_breakdown;
            if rollup_totals.is_none() || rollup_usage.is_none() {
                global_gap |= historical_rollups_materialized_at.is_none();
            }
            if let (Some(rollup_totals), Some(rollup_usage)) = (rollup_totals, rollup_usage) {
                let first_bucket = align_bucket_epoch(localized.start.timestamp(), 3_600, 0);
                let last_bucket =
                    align_bucket_epoch(localized.end.timestamp().saturating_sub(1), 3_600, 0);
                let mut bucket = first_bucket;
                while bucket <= last_bucket {
                    global_gap |= !rollup_totals.contains_key(&(bucket, None))
                        || !rollup_usage.contains_key(&(bucket, None));
                    if let Some(account_ids) = account_manifest.get(&file_path) {
                        account_gap |= account_ids.iter().any(|account_id| {
                            !rollup_totals.contains_key(&(bucket, Some(*account_id)))
                                || !rollup_usage.contains_key(&(bucket, Some(*account_id)))
                        });
                    }
                    if global_gap && account_gap {
                        break;
                    }
                    bucket = bucket.saturating_add(3_600);
                }
            }
            if global_gap {
                global_ranges.push(localized);
            }
            if account_gap {
                account_ranges.push(localized);
            }
        }
    }
    let global_ranges = summary_projection_merge_exact_ranges(global_ranges);
    let account_ranges = summary_projection_merge_exact_ranges(account_ranges);
    info!(
        stage = "overflowed_boundary_unproven_range_summary",
        source_row_count,
        horizon_fallback_count,
        global_range_count = global_ranges.len(),
        account_range_count = account_ranges.len(),
        "summary projection overflowed boundary ranges summarized"
    );
    Ok(SummaryProjectionOverflowedBoundaryUnprovenRanges {
        global: global_ranges,
        account: account_ranges,
        broad_fail_closed: false,
    })
}

pub(crate) async fn summary_projection_overflowed_all_time_manifests_have_complete_rollups(
    pool: &Pool<Sqlite>,
) -> Result<Option<i64>> {
    let Some(high_watermark_id) =
        summary_projection_completed_manifest_high_watermark(pool).await?
    else {
        return Ok(None);
    };
    let incomplete = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS( \
             SELECT 1 FROM archive_batches AS batches \
             WHERE batches.dataset = 'codex_invocations' \
               AND batches.status = 'completed' \
               AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror' \
               AND batches.id <= ?1 \
               AND ( \
                    batches.coverage_start_at IS NULL \
                    OR batches.coverage_end_at IS NULL \
                    OR batches.historical_rollups_materialized_at IS NULL \
                    OR NOT EXISTS( \
                        SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                        WHERE replay.dataset = batches.dataset \
                          AND replay.file_path = batches.file_path \
                          AND replay.archive_sha256 = batches.sha256 \
                          AND replay.target = ?2 \
                    ) \
                    OR NOT EXISTS( \
                        SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                        WHERE replay.dataset = batches.dataset \
                          AND replay.file_path = batches.file_path \
                          AND replay.archive_sha256 = batches.sha256 \
                          AND replay.target = ?3 \
                    ) \
                    OR NOT EXISTS( \
                        SELECT 1 FROM hourly_rollup_archive_replay AS replay \
                        WHERE replay.dataset = batches.dataset \
                          AND replay.file_path = batches.file_path \
                          AND replay.archive_sha256 = batches.sha256 \
                          AND replay.target = ?4 \
                    ) \
               ) \
         )",
    )
    .bind(high_watermark_id)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY)
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN)
    .fetch_one(pool)
    .await
    .context("summary projection overflowed all-time manifest coverage hydration failed")?;
    Ok((incomplete == 0).then_some(high_watermark_id))
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct SummaryProjectionBoundaryManifestPageRow {
    pub(crate) id: i64,
    pub(crate) file_path: String,
    pub(crate) coverage_start_at: Option<String>,
    pub(crate) coverage_end_at: Option<String>,
    pub(crate) historical_rollups_materialized_at: Option<String>,
    pub(crate) upstream_activity_manifest_refreshed_at: Option<String>,
}

pub(crate) struct SummaryProjectionBoundaryManifestPage {
    pub(crate) archives: Vec<crate::stats::ArchiveBatchPathRow>,
    pub(crate) account_manifest_refreshed_paths: HashSet<String>,
    pub(crate) next_after_id: Option<i64>,
}

pub(crate) async fn load_summary_projection_boundary_manifest_page(
    pool: &Pool<Sqlite>,
    range: Option<ExactUtcRange>,
    after_id: Option<i64>,
    high_watermark_id: i64,
) -> Result<SummaryProjectionBoundaryManifestPage> {
    load_summary_projection_boundary_manifest_page_with_limit(
        pool,
        range,
        after_id,
        high_watermark_id,
        SUMMARY_PROJECTION_ARCHIVE_MANIFEST_QUERY_CHUNK_SIZE,
    )
    .await
}

pub(crate) async fn load_summary_projection_boundary_manifest_page_with_limit(
    pool: &Pool<Sqlite>,
    range: Option<ExactUtcRange>,
    after_id: Option<i64>,
    high_watermark_id: i64,
    page_size: usize,
) -> Result<SummaryProjectionBoundaryManifestPage> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, file_path, coverage_start_at, coverage_end_at, \
                historical_rollups_materialized_at, upstream_activity_manifest_refreshed_at \
         FROM archive_batches \
         WHERE dataset = 'codex_invocations' \
           AND status = 'completed' \
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'",
    );
    if let Some(after_id) = after_id {
        query.push(" AND id > ").push_bind(after_id);
    }
    query.push(" AND id <= ").push_bind(high_watermark_id);
    if let Some(range) = range {
        query
            .push(" AND coverage_end_at >= ")
            .push_bind(db_occurred_at_upper_bound(range.start))
            .push(" AND coverage_start_at < ")
            .push_bind(crate::stats::db_occurred_at_lower_bound(range.end));
    }
    let rows = query
        .push(" ORDER BY id ASC LIMIT ")
        .push_bind(page_size as i64)
        .build_query_as::<SummaryProjectionBoundaryManifestPageRow>()
        .fetch_all(pool)
        .await
        .context("summary projection boundary manifest page hydration failed")?;
    let next_after_id = rows.last().map(|row| row.id);
    let account_manifest_refreshed_paths = rows
        .iter()
        .filter(|row| row.upstream_activity_manifest_refreshed_at.is_some())
        .map(|row| row.file_path.clone())
        .collect();
    let archives = rows
        .into_iter()
        .map(|row| {
            crate::stats::ArchiveBatchPathRow::with_coverage_and_historical_rollups(
                row.file_path,
                row.coverage_start_at,
                row.coverage_end_at,
                row.historical_rollups_materialized_at,
            )
        })
        .collect();
    Ok(SummaryProjectionBoundaryManifestPage {
        archives,
        account_manifest_refreshed_paths,
        next_after_id,
    })
}

pub(crate) async fn load_summary_projection_archive_manifest_account_sets(
    pool: &Pool<Sqlite>,
    archive_paths: &[String],
) -> Result<HashMap<String, HashSet<i64>>> {
    let mut account_ids_by_file = archive_paths
        .iter()
        .cloned()
        .map(|path| (path, HashSet::new()))
        .collect::<HashMap<_, _>>();
    for (file_path, account_id) in
        load_summary_projection_archive_manifest_accounts(pool, archive_paths).await?
    {
        if let Some(account_ids) = account_ids_by_file.get_mut(&file_path) {
            account_ids.insert(account_id);
        }
    }
    Ok(account_ids_by_file)
}

pub(crate) fn summary_projection_all_time_manifest_scope_coverage(
    archive: &crate::stats::ArchiveBatchPathRow,
    replay_coverage: SummaryProjectionArchiveReplayCoverage,
    account_manifest_complete: bool,
    account_ids: &HashSet<i64>,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
) -> Result<(bool, bool)> {
    let Some(range) = summary_projection_archive_coverage_range(archive) else {
        return Ok((false, false));
    };
    if summary_projection_exact_range_fits_bucket_budget(range).is_err() {
        return Ok((false, false));
    }
    // Global totals include usage/cost dimensions. An invocation replay marker alone cannot
    // prove the complete StatsResponse, so keep the archive unavailable until its usage replay
    // (or a verified V2 replacement) is present as well.
    let mut global_covered = replay_coverage.overall && replay_coverage.usage_breakdown;
    // Account-scoped totals expose the same usage/cost dimensions as the global response. An
    // account replay marker without its usage-breakdown proof must therefore remain unavailable.
    let mut accounts_covered = replay_coverage.account_stats
        && replay_coverage.usage_breakdown
        && account_manifest_complete;
    let mut bucket = align_bucket_epoch(range.start.timestamp(), 3_600, 0);
    let last_bucket = align_bucket_epoch(range.end.timestamp().saturating_sub(1), 3_600, 0);
    while bucket <= last_bucket {
        global_covered &= hourly_rollup_totals.contains_key(&(bucket, None));
        accounts_covered &= account_ids
            .iter()
            .all(|account_id| hourly_rollup_totals.contains_key(&(bucket, Some(*account_id))));
        if !global_covered && !accounts_covered {
            break;
        }
        bucket = bucket.saturating_add(3_600);
    }
    Ok((global_covered, accounts_covered))
}

pub(crate) async fn summary_projection_paged_all_time_materialized_scope_coverage(
    pool: &Pool<Sqlite>,
    high_watermark_id: i64,
    hourly_rollup_totals: &HashMap<(i64, Option<i64>), StatsTotals>,
) -> Result<(bool, bool, HashSet<i64>)> {
    let mut global_covered = true;
    let mut accounts_covered = true;
    let mut discovered_account_ids = HashSet::new();
    let mut after_id = None;
    loop {
        let page =
            load_summary_projection_boundary_manifest_page(pool, None, after_id, high_watermark_id)
                .await?;
        if page.archives.is_empty() {
            break;
        }
        let paths = page
            .archives
            .iter()
            .map(|archive| archive.file_path().to_string())
            .collect::<Vec<_>>();
        let replay_coverage = load_summary_projection_archive_replay_coverage(pool, &paths).await?;
        let account_ids_by_file =
            match load_summary_projection_archive_manifest_account_sets(pool, &paths).await {
                Ok(account_ids_by_file) => account_ids_by_file,
                Err(error)
                    if error.to_string().starts_with(
                        "summary projection archive account manifest exceeded bounded row budget",
                    ) =>
                {
                    accounts_covered = false;
                    paths
                        .iter()
                        .cloned()
                        .map(|path| (path, HashSet::new()))
                        .collect()
                }
                Err(error) => return Err(error),
            };
        for archive in &page.archives {
            let account_ids = account_ids_by_file
                .get(archive.file_path())
                .expect("page account manifest includes every requested archive path");
            if accounts_covered {
                discovered_account_ids.extend(account_ids.iter().copied());
                if discovered_account_ids.len() > SUMMARY_PROJECTION_MAX_ACCOUNTS {
                    accounts_covered = false;
                    discovered_account_ids.clear();
                }
            }
            let replay = replay_coverage
                .get(archive.file_path())
                .copied()
                .unwrap_or_default();
            let (archive_global_covered, archive_accounts_covered) =
                summary_projection_all_time_manifest_scope_coverage(
                    archive,
                    replay,
                    page.account_manifest_refreshed_paths
                        .contains(archive.file_path()),
                    account_ids,
                    hourly_rollup_totals,
                )?;
            global_covered &= archive_global_covered;
            accounts_covered &= archive_accounts_covered;
        }
        let Some(next_after_id) = page.next_after_id else {
            break;
        };
        after_id = Some(next_after_id);
    }
    Ok((global_covered, accounts_covered, discovered_account_ids))
}

pub(crate) async fn load_summary_projection_durable_account_ids(
    pool: &Pool<Sqlite>,
) -> Result<HashSet<i64>> {
    let pool_rows =
        sqlx::query_scalar::<_, i64>("SELECT id FROM pool_upstream_accounts WHERE id > 0 LIMIT ?1")
            .bind((SUMMARY_PROJECTION_MAX_ACCOUNTS + 1) as i64)
            .fetch_all(pool)
            .await
            .context("summary projection account hydration failed")?;
    let rollup_rows = sqlx::query_scalar::<_, i64>(
        "SELECT DISTINCT upstream_account_id \
         FROM upstream_account_stats_hourly \
         WHERE upstream_account_id > 0 \
         LIMIT ?1",
    )
    .bind((SUMMARY_PROJECTION_MAX_ACCOUNTS + 1) as i64)
    .fetch_all(pool)
    .await
    .context("summary projection historical account hydration failed")?;
    if pool_rows.len() > SUMMARY_PROJECTION_MAX_ACCOUNTS
        || rollup_rows.len() > SUMMARY_PROJECTION_MAX_ACCOUNTS
    {
        return Err(anyhow!(
            "summary projection durable account cardinality exceeded bounded budget ({SUMMARY_PROJECTION_MAX_ACCOUNTS})"
        ));
    }
    Ok(pool_rows.into_iter().chain(rollup_rows).collect())
}

pub(crate) async fn restore_summary_projection_persisted_statuses(
    connection: &mut SqliteConnection,
    rows: &mut [UpstreamAccountInvocationPreviewRow],
) -> Result<()> {
    const STATUS_HYDRATION_CHUNK_SIZE: usize = 500;
    let ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let mut statuses = HashMap::<i64, Option<String>>::new();
    for chunk in ids.chunks(STATUS_HYDRATION_CHUNK_SIZE) {
        let mut query =
            QueryBuilder::<Sqlite>::new("SELECT id, status FROM codex_invocations WHERE id IN (");
        {
            let mut separated = query.separated(", ");
            for id in chunk {
                separated.push_bind(*id);
            }
        }
        query.push(")");
        statuses.extend(
            query
                .build_query_as::<(i64, Option<String>)>()
                .fetch_all(&mut *connection)
                .await?,
        );
    }
    for row in rows {
        if let Some(status) = statuses.get(&row.id).and_then(|status| status.clone()) {
            row.status = status;
        }
    }
    Ok(())
}
