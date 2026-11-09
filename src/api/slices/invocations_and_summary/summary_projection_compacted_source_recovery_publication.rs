use super::*;
use serde_json::to_string;

pub(super) async fn ensure_compacted_source_recovery(state: &AppState) -> Result<()> {
    let _ = ensure_compacted_source_recovery_with_force_bounded(
        state,
        false,
        SUMMARY_COMPACTED_SOURCE_RECOVERY_TURN_BUDGET,
    )
    .await?;
    Ok(())
}

pub(super) async fn ensure_compacted_source_recovery_with_force_bounded(
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

pub(super) fn is_sqlite_contention(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let message = cause.to_string().to_ascii_lowercase();
        message.contains("database is locked")
            || message.contains("database table is locked")
            || message.contains("sqlite_busy")
            || message.contains("sqlite_locked")
    })
}

pub(super) const COMPACTED_SOURCE_RECOVERY_STATE_UPDATE_SQL: &str = "UPDATE summary_compacted_source_recovery_checkpoint SET state = ?1, last_error_code = ?2,\
                last_error_message = ?3, updated_at = datetime('now') WHERE scope = ?4";

pub(super) async fn update_compacted_source_recovery_state(
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

pub(super) async fn update_compacted_source_recovery_state_bounded(
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

pub(super) async fn compacted_source_recovery_staged_totals_match_source(
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

pub(super) async fn publish_compacted_source_recovery_snapshot(
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

pub(super) async fn publish_compacted_source_recovery_snapshot_in_transaction(
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

pub(super) async fn run_compacted_source_recovery_turn_with_runtime(
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
