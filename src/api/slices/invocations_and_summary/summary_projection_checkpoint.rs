async fn advance_summary_all_time_projection_account_rollup(
    pool: &Pool<Sqlite>,
    checkpoint: &SummaryAllTimeProjectionCheckpointRow,
) -> Result<()> {
    if checkpoint.account_rollup_complete != 0 || checkpoint.account_unavailable != 0 {
        return Ok(());
    }
    let rows = sqlx::query_as::<_, SummaryAllTimeCheckpointAccountRollupRow>(
        "SELECT rowid, upstream_account_id, total_count, success_count, failure_count, \
                total_tokens, total_cost, COALESCE(non_success_cost, 0.0) AS non_success_cost \
         FROM upstream_account_stats_hourly \
         WHERE rowid > ?1 AND upstream_account_id > 0 ORDER BY rowid ASC LIMIT ?2",
    )
    .bind(checkpoint.account_rollup_next_rowid)
    .bind(SUMMARY_PROJECTION_ALL_TIME_ROLLUP_PAGE_SIZE)
    .fetch_all(pool)
    .await
    .context("summary all-time projection account rollup page hydration failed")?;
    let mut transaction = pool
        .begin()
        .await
        .context("summary all-time projection account rollup transaction failed")?;
    if rows.is_empty() {
        sqlx::query(
            "UPDATE summary_all_time_projection_checkpoint \
             SET account_rollup_complete = 1, updated_at = datetime('now') WHERE scope = ?1",
        )
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .execute(&mut *transaction)
        .await
        .context("summary all-time projection account rollup completion failed")?;
        transaction
            .commit()
            .await
            .context("summary all-time projection account rollup completion commit failed")?;
        return Ok(());
    }
    for row in &rows {
        let totals = row.totals();
        sqlx::query(
            "INSERT INTO summary_all_time_projection_account_checkpoint \
             (scope, upstream_account_id, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT(scope, upstream_account_id) DO UPDATE SET \
               total_count = total_count + excluded.total_count, \
               success_count = success_count + excluded.success_count, \
               failure_count = failure_count + excluded.failure_count, \
               total_tokens = total_tokens + excluded.total_tokens, \
               total_cost = total_cost + excluded.total_cost, \
               non_success_cost = non_success_cost + excluded.non_success_cost",
        )
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .bind(row.upstream_account_id)
        .bind(totals.total_count)
        .bind(totals.success_count)
        .bind(totals.failure_count)
        .bind(totals.total_tokens)
        .bind(totals.total_cost)
        .bind(totals.non_success_cost)
        .execute(&mut *transaction)
        .await
        .context("summary all-time projection account rollup accumulation failed")?;
    }
    let account_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM summary_all_time_projection_account_checkpoint WHERE scope = ?1",
    )
    .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
    .fetch_one(&mut *transaction)
    .await
    .context("summary all-time projection account checkpoint count failed")?;
    let next_rowid = rows.last().map(|row| row.rowid).unwrap_or_default();
    if account_count > SUMMARY_PROJECTION_MAX_ACCOUNTS as i64 {
        sqlx::query(
            "UPDATE summary_all_time_projection_checkpoint SET \
             account_unavailable = 1, account_rollup_complete = 1, \
             updated_at = datetime('now') WHERE scope = ?1",
        )
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .execute(&mut *transaction)
        .await
        .context("summary all-time projection account cardinality checkpoint failed")?;
    } else {
        sqlx::query(
            "UPDATE summary_all_time_projection_checkpoint SET \
             account_rollup_next_rowid = ?1, updated_at = datetime('now') WHERE scope = ?2",
        )
        .bind(next_rowid)
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .execute(&mut *transaction)
        .await
        .context("summary all-time projection account rollup checkpoint advance failed")?;
    }
    transaction
        .commit()
        .await
        .context("summary all-time projection account rollup checkpoint commit failed")?;
    Ok(())
}

async fn load_summary_projection_generation_fence(
    state: &AppState,
) -> Result<SummaryProjectionGenerationFence> {
    let durable_terminal_sequence_watermark = state
        .dashboard_activity_snapshot_cache
        .lock()
        .await
        .read_model
        .settled_terminal_sequence;
    Ok(SummaryProjectionGenerationFence {
        live_high_watermark_id: load_summary_projection_live_high_watermark(&state.pool).await?,
        rollup_live_cursor: load_summary_projection_rollup_live_cursor(&state.pool).await?,
        account_rollup_live_cursor: load_summary_projection_account_rollup_live_cursor(&state.pool)
            .await?,
        completed_manifest_high_watermark_id: summary_projection_completed_manifest_high_watermark(
            &state.pool,
        )
        .await?,
        coverage_revision: summary_projection_coverage_revision(&state.pool).await?,
        account_coverage_revision: summary_projection_account_coverage_revision(&state.pool)
            .await?,
        durable_terminal_sequence_watermark,
    })
}

async fn load_summary_projection_generation_fence_tx(
    state: &AppState,
    connection: &mut SqliteConnection,
) -> Result<SummaryProjectionGenerationFence> {
    let durable_terminal_sequence_watermark = state
        .dashboard_activity_snapshot_cache
        .lock()
        .await
        .read_model
        .settled_terminal_sequence;
    let live_high_watermark_id =
        sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(id) FROM codex_invocations")
            .fetch_one(&mut *connection)
            .await
            .context("summary projection live high-watermark transaction hydration failed")?
            .unwrap_or_default();
    let rollup_live_cursor = load_invocation_summary_rollup_live_cursor_tx(connection)
        .await
        .context("summary projection live rollup cursor transaction hydration failed")?;
    let account_rollup_live_cursor = sqlx::query_scalar::<_, i64>(
        "SELECT cursor_id FROM hourly_rollup_live_progress \
         WHERE dataset = 'invocation_account_activity_v2_repair_live_cursor'",
    )
    .fetch_optional(&mut *connection)
    .await
    .context("summary projection account rollup cursor transaction hydration failed")?;
    let completed_manifest_high_watermark_id = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(id) FROM archive_batches \
         WHERE dataset = 'codex_invocations' AND status = 'completed' \
           AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'",
    )
    .fetch_one(&mut *connection)
    .await
    .context("summary projection completed manifest transaction hydration failed")?;
    let coverage_revision = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(revision, 0) FROM summary_coverage_revision WHERE id = 1",
    )
    .fetch_optional(&mut *connection)
    .await
    .context("summary projection coverage revision transaction hydration failed")?
    .unwrap_or_default();
    let account_coverage_revision = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(revision, 0) FROM summary_account_coverage_revision WHERE id = 1",
    )
    .fetch_optional(&mut *connection)
    .await
    .context("summary projection account coverage revision transaction hydration failed")?
    .unwrap_or_default();
    Ok(SummaryProjectionGenerationFence {
        live_high_watermark_id,
        rollup_live_cursor,
        account_rollup_live_cursor,
        completed_manifest_high_watermark_id,
        coverage_revision,
        account_coverage_revision,
        durable_terminal_sequence_watermark,
    })
}

/// Publish a coverage-derived projection while holding the SQLite writer fence used to verify its
/// durable inputs. A caller may spend substantial time reducing pages before reaching this point;
/// the immediate transaction re-check ensures a manifest, rollup, or source-tail mutation cannot
/// land between the caller's last fence read and the in-memory CAS.
async fn publish_summary_projection_with_durable_fence(
    state: &AppState,
    projection: SummaryProjection,
    expected_revision: u64,
    expected_generation_fence: SummaryProjectionGenerationFence,
) -> Result<bool> {
    let mut transaction = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("begin Summary durable-fence publication transaction")?;
    let current_generation_fence =
        load_summary_projection_generation_fence_tx(state, transaction.as_mut()).await?;
    if current_generation_fence != expected_generation_fence {
        transaction
            .rollback()
            .await
            .context("rollback stale Summary durable-fence publication")?;
        return Ok(false);
    }
    let published = state
        .subscription_hub
        .store_summary_projection_if_revision_and_generation(
            projection,
            expected_revision,
            expected_generation_fence,
        )
        .await;
    if !published {
        transaction
            .rollback()
            .await
            .context("rollback stale Summary durable-fence projection CAS")?;
        return Ok(false);
    }
    transaction
        .commit()
        .await
        .context("commit Summary durable-fence publication transaction")?;
    Ok(true)
}

async fn publish_summary_projection_with_durable_coverage_fence(
    state: &AppState,
    projection: SummaryProjection,
    expected_revision: u64,
    expected_generation_fence: SummaryProjectionGenerationFence,
) -> Result<bool> {
    let mut transaction = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("begin Summary durable-coverage publication transaction")?;
    let current_generation_fence =
        load_summary_projection_generation_fence_tx(state, transaction.as_mut()).await?;
    if !current_generation_fence.coverage_sources_match(expected_generation_fence)
        || !current_generation_fence
            .live_tail_cursor()
            .at_or_behind(expected_generation_fence.live_tail_cursor())
    {
        transaction
            .rollback()
            .await
            .context("rollback stale Summary durable-coverage publication")?;
        return Ok(false);
    }
    let published = state
        .subscription_hub
        .store_summary_projection_if_revision_and_coverage_generation(
            projection,
            expected_revision,
            expected_generation_fence,
        )
        .await;
    if !published {
        transaction
            .rollback()
            .await
            .context("rollback stale Summary durable-coverage projection CAS")?;
        return Ok(false);
    }
    transaction
        .commit()
        .await
        .context("commit Summary durable-coverage publication transaction")?;
    Ok(true)
}

async fn advance_summary_all_time_projection_checkpoint(
    state: &AppState,
) -> Result<SummaryAllTimeProjectionCheckpointRow> {
    let generation_fence = load_summary_projection_generation_fence(state).await?;
    let mut checkpoint = match load_summary_all_time_projection_checkpoint(&state.pool).await? {
        Some(checkpoint)
            if checkpoint
                .generation_fence()
                .global_coverage_checkpoint_compatible(generation_fence) =>
        {
            if checkpoint
                .generation_fence()
                .account_coverage_checkpoint_compatible(generation_fence)
            {
                checkpoint
            } else {
                debug!(
                    "summary all-time account coverage revision changed; resetting only account recovery"
                );
                reset_summary_all_time_projection_account_scope(&state.pool, generation_fence)
                    .await?
            }
        }
        Some(_) => {
            debug!(
                "summary all-time projection checkpoint generation changed; restarting staged recovery"
            );
            reset_summary_all_time_projection_checkpoint(&state.pool, generation_fence).await?
        }
        None => reset_summary_all_time_projection_checkpoint(&state.pool, generation_fence).await?,
    };
    // Proof insertion is forward progress and therefore keeps the committed manifest/rollup
    // cursors.  Persist the newer coverage revisions on that same checkpoint before advancing
    // pages; otherwise finalization would retain the old fence and reject its own atomic swap.
    if checkpoint.coverage_revision != generation_fence.coverage_revision
        || checkpoint.account_coverage_revision != generation_fence.account_coverage_revision
    {
        sqlx::query(
            "UPDATE summary_all_time_projection_checkpoint SET \
             coverage_revision = ?1, account_coverage_revision = ?2, updated_at = datetime('now') \
             WHERE scope = ?3",
        )
        .bind(generation_fence.coverage_revision)
        .bind(generation_fence.account_coverage_revision)
        .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE)
        .execute(&state.pool)
        .await
        .context("summary all-time projection checkpoint coverage revision adoption failed")?;
        checkpoint = load_summary_all_time_projection_checkpoint(&state.pool)
            .await?
            .expect(
                "summary all-time projection checkpoint exists after coverage revision adoption",
            );
    }
    advance_summary_all_time_projection_manifest_scope(&state.pool, &checkpoint, false).await?;
    let checkpoint = load_summary_all_time_projection_checkpoint(&state.pool)
        .await?
        .expect("summary all-time projection checkpoint exists after global manifest advance");
    advance_summary_all_time_projection_manifest_scope(&state.pool, &checkpoint, true).await?;
    let checkpoint = load_summary_all_time_projection_checkpoint(&state.pool)
        .await?
        .expect("summary all-time projection checkpoint exists after account manifest advance");
    advance_summary_all_time_projection_global_rollup(&state.pool, &checkpoint).await?;
    let checkpoint = load_summary_all_time_projection_checkpoint(&state.pool)
        .await?
        .expect("summary all-time projection checkpoint exists after global rollup advance");
    advance_summary_all_time_projection_account_rollup(&state.pool, &checkpoint).await?;
    if checkpoint.usage_rollup_complete == 0 {
        // Usage is independently bounded by the existing rollup reader. Mark it ready only
        // once; repeating this full read after completion made every idle supervisor pass scan
        // the complete usage rollup again.
        let usage_ready = match load_summary_projection_rollup_usage(&state.pool).await {
            Ok(_) => (1_i64, 0_i64, 0_i64),
            Err(error) if error.to_string().contains("budget") => (1_i64, 1_i64, 1_i64),
            Err(error) => return Err(error),
        };
        sqlx::query("UPDATE summary_all_time_projection_checkpoint SET usage_rollup_complete = ?1, global_usage_unavailable = ?2, account_usage_unavailable = ?3, updated_at = datetime('now') WHERE scope = ?4")
            .bind(usage_ready.0).bind(usage_ready.1).bind(usage_ready.2)
            .bind(SUMMARY_ALL_TIME_PROJECTION_CHECKPOINT_SCOPE).execute(&state.pool).await?;
    }
    let checkpoint = load_summary_all_time_projection_checkpoint(&state.pool)
        .await?
        .expect("summary all-time projection checkpoint exists after account rollup advance");
    if !load_summary_projection_generation_fence(state)
        .await?
        .coverage_sources_match(generation_fence)
    {
        // The rolling projection may predate a newly verified Snapshot V2 page. Its recent
        // records remain exact; the finalizer can merge the historical totals and publish the
        // current fence atomically below. A concurrent live fence change is still checked before
        // the swap.
        debug!(
            "summary historical coverage finalization is merging into a projection with an older coverage fence"
        );
    }
    Ok(checkpoint)
}

async fn summary_all_time_projection_checkpoint_live_tail_count(
    pool: &Pool<Sqlite>,
    lower_bound_id: i64,
    upper_bound_id: i64,
    scope: &'static str,
) -> Result<()> {
    if upper_bound_id <= lower_bound_id {
        return Ok(());
    }
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations WHERE id > ?1 AND id <= ?2",
    )
    .bind(lower_bound_id)
    .bind(upper_bound_id)
    .fetch_one(pool)
    .await
    .with_context(|| format!("summary all-time {scope} live-tail admission count failed"))?;
    if count > summary_projection_exact_record_limit() as i64 {
        return Err(anyhow!(
            "summary all-time {scope} live-tail admission exceeded bounded record budget ({count} > {})",
            summary_projection_exact_record_limit()
        ));
    }
    Ok(())
}

fn summary_snapshot_v2_record_preview(
    record: SummaryArchiveSnapshotV2Record,
) -> UpstreamAccountInvocationPreviewRow {
    UpstreamAccountInvocationPreviewRow {
        upstream_account_id: record.upstream_account_id,
        id: record.id,
        invoke_id: record.invoke_id,
        prompt_cache_key: None,
        occurred_at: record.occurred_at,
        conversation_created_at: None,
        status: record.status,
        live_phase: None,
        failure_class: record.failure_class,
        route_mode: None,
        model: record.model,
        request_model: None,
        response_model: record.response_model,
        total_tokens: record.total_tokens,
        cost: record.cost,
        cost_input: record.cost_input,
        cost_cache_write: record.cost_cache_write,
        cost_cache_read: record.cost_cache_read,
        cost_output: record.cost_output,
        cost_reasoning: record.cost_reasoning,
        source: Some(record.source),
        input_tokens: Some(record.input_tokens),
        output_tokens: Some(record.output_tokens),
        cache_input_tokens: Some(record.cache_input_tokens),
        reasoning_tokens: Some(record.reasoning_tokens),
        reasoning_effort: record.reasoning_effort,
        error_message: record.error_message,
        downstream_status_code: None,
        downstream_error_message: None,
        failure_kind: record.failure_kind,
        is_actionable: Some(i64::from(record.is_actionable)),
        proxy_display_name: None,
        upstream_account_name: None,
        upstream_account_plan_type: None,
        response_content_encoding: None,
        request_compression_algorithm: None,
        transport: None,
        requested_service_tier: None,
        service_tier: None,
        billing_service_tier: None,
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        first_token_ms: None,
        t_upstream_stream_ms: None,
        t_resp_parse_ms: None,
        t_persist_ms: None,
        t_total_ms: None,
        endpoint: None,
        compaction_request_kind: None,
        compaction_response_kind: None,
        image_intent: None,
    }
}
