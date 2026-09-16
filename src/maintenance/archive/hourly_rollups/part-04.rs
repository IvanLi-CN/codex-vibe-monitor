pub(crate) async fn select_active_account_activity_v2_priority_buckets_with_deadline(
    pool: &Pool<Sqlite>,
    current_bucket: i64,
    started_at: Instant,
    selection_deadline: Instant,
    progress_probe: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    progress_handler_ops: i32,
    progress_abort_on_probe: bool,
) -> Result<Option<Vec<i64>>> {
    let selection_deadline = std::cmp::min(
        selection_deadline,
        started_at + ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUDGET,
    );
    let Some(remaining_budget) = selection_deadline.checked_duration_since(Instant::now()) else {
        return Ok(None);
    };
    let mut connection = match timeout(remaining_budget, pool.acquire()).await {
        Ok(connection) => connection?,
        Err(_) => return Ok(None),
    };
    let Some(remaining_budget) = selection_deadline.checked_duration_since(Instant::now()) else {
        connection.close_on_drop();
        return Ok(None);
    };
    let lock_timed_out = {
        let handle_result = timeout(remaining_budget, connection.lock_handle()).await;
        match handle_result {
            Ok(Ok(mut handle)) => {
                handle.set_progress_handler(progress_handler_ops, move || {
                    if let Some(progress_probe) = progress_probe.as_ref() {
                        progress_probe.store(true, std::sync::atomic::Ordering::Relaxed);
                        if progress_abort_on_probe {
                            return false;
                        }
                    }
                    Instant::now() < selection_deadline
                });
                false
            }
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => true,
        }
    };
    if lock_timed_out {
        connection.close_on_drop();
        return Ok(None);
    }
    let Some(remaining_budget) = selection_deadline.checked_duration_since(Instant::now()) else {
        connection.close_on_drop();
        return Ok(None);
    };

    let selection = timeout(remaining_budget, async {
        let oldest_live_occurred_at = sqlx::query_scalar::<_, Option<String>>(
            "SELECT MIN(occurred_at) FROM codex_invocations",
        )
        .fetch_one(&mut *connection)
        .await?;
        let Some(oldest_live_occurred_at) = oldest_live_occurred_at else {
            return Ok(Vec::new());
        };
        let oldest_live_bucket = align_bucket_epoch(
            parse_to_utc_datetime(&oldest_live_occurred_at)
                .ok_or_else(|| anyhow!("failed to parse oldest live invocation timestamp"))?
                .timestamp(),
            3_600,
            0,
        );
        let configured_oldest_bucket = current_bucket - 7 * 24 * 3_600;
        // This priority path is intentionally live-only. Archive buckets are repaired by archive
        // replay; selecting them here would clear valid archive-derived v2 values before loading no
        // live rows and then incorrectly mark the zero bucket covered.
        let oldest_bucket = configured_oldest_bucket.max(oldest_live_bucket);
        if oldest_bucket >= current_bucket {
            return Ok(Vec::new());
        }

        let covered_bucket_rows = sqlx::query_scalar::<_, i64>(
            "SELECT bucket_start_epoch \
             FROM hourly_rollup_materialized_buckets \
             WHERE target = ?1 AND source = ?2 \
               AND bucket_start_epoch >= ?3 AND bucket_start_epoch < ?4",
        )
        .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
        .bind(HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE)
        .bind(oldest_bucket)
        .bind(current_bucket)
        .fetch_all(&mut *connection)
        .await?;
        let mut covered_buckets = HashSet::new();
        for bucket_start_epoch in covered_bucket_rows {
            if Instant::now() >= selection_deadline {
                return Err(anyhow!(
                    "active account activity v2 coverage selection deadline reached during coverage scan"
                ));
            }
            covered_buckets.insert(bucket_start_epoch);
        }
        let archive_epoch_coverage = build_active_account_activity_v2_archive_epoch_coverage_query(
            "",
            oldest_bucket,
            current_bucket,
        )
        .build_query_as::<(i64, i64)>()
        .fetch_all(&mut *connection)
        .await?;

        let mut active_month_keys = Vec::new();
        let mut bucket_start_epoch = current_bucket - 3_600;
        while bucket_start_epoch >= oldest_bucket {
            if Instant::now() >= selection_deadline {
                return Err(anyhow!(
                    "active account activity v2 coverage selection deadline reached during legacy month scan"
                ));
            }
            let month_key = active_account_activity_v2_month_key(bucket_start_epoch)?;
            if !active_month_keys.contains(&month_key) {
                active_month_keys.push(month_key);
            }
            bucket_start_epoch -= 3_600;
        }
        let archive_legacy_month_rows =
            build_active_account_activity_v2_legacy_coverage_query("", &active_month_keys)
                .build_query_scalar::<String>()
                .fetch_all(&mut *connection)
                .await?;
        let mut archive_legacy_month_keys = HashSet::new();
        for month_key in archive_legacy_month_rows {
            if Instant::now() >= selection_deadline {
                return Err(anyhow!(
                    "active account activity v2 coverage selection deadline reached during legacy coverage scan"
                ));
            }
            archive_legacy_month_keys.insert(month_key);
        }

        let mut missing_buckets =
            Vec::with_capacity(ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUCKET_LIMIT);
        let mut bucket_start_epoch = current_bucket - 3_600;
        while bucket_start_epoch >= oldest_bucket
            && missing_buckets.len() < ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUCKET_LIMIT
        {
            if Instant::now() >= selection_deadline {
                return Err(anyhow!(
                    "active account activity v2 coverage selection deadline reached during archive scan"
                ));
            }
            let month_key = active_account_activity_v2_month_key(bucket_start_epoch)?;
            let is_archive_covered = if archive_legacy_month_keys.contains(&month_key) {
                true
            } else {
                let mut covered = false;
                for (coverage_start_epoch, coverage_end_epoch) in &archive_epoch_coverage {
                    if Instant::now() >= selection_deadline {
                        return Err(anyhow!(
                            "active account activity v2 coverage selection deadline reached during archive scan"
                        ));
                    }
                    if *coverage_start_epoch < bucket_start_epoch + 3_600
                        && *coverage_end_epoch >= bucket_start_epoch
                    {
                        covered = true;
                        break;
                    }
                }
                covered
            };
            if !covered_buckets.contains(&bucket_start_epoch) && !is_archive_covered {
                missing_buckets.push(bucket_start_epoch);
            }
            bucket_start_epoch -= 3_600;
        }
        Ok(missing_buckets)
    })
    .await;

    match selection {
        Err(_) => {
            connection.close_on_drop();
            warn!(
                priority_elapsed_budget_ms =
                    ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUDGET.as_millis() as u64,
                "active account activity v2 coverage selection exhausted its budget"
            );
            Ok(None)
        }
        Ok(Err(error)) if Instant::now() >= selection_deadline => {
            connection.close_on_drop();
            warn!(
                priority_elapsed_budget_ms =
                    ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUDGET.as_millis() as u64,
                error = %error,
                "active account activity v2 coverage selection interrupted at its SQLite budget"
            );
            Ok(None)
        }
        Ok(Err(error)) if progress_abort_on_probe => {
            connection.close_on_drop();
            warn!(
                error = %error,
                "active account activity v2 coverage selection interrupted by test progress probe"
            );
            Ok(None)
        }
        Ok(Err(error)) => {
            let Some(remaining_budget) = selection_deadline.checked_duration_since(Instant::now())
            else {
                connection.close_on_drop();
                return Ok(None);
            };
            let cleanup_succeeded = {
                let cleanup_result = timeout(remaining_budget, connection.lock_handle()).await;
                match cleanup_result {
                    Ok(Ok(mut handle)) => {
                        handle.remove_progress_handler();
                        true
                    }
                    Ok(Err(_)) | Err(_) => false,
                }
            };
            if cleanup_succeeded {
                Err(error)
            } else {
                connection.close_on_drop();
                Ok(None)
            }
        }
        Ok(Ok(_selection)) if Instant::now() >= selection_deadline => {
            connection.close_on_drop();
            warn!(
                priority_elapsed_budget_ms =
                    ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUDGET.as_millis() as u64,
                "active account activity v2 coverage selection exhausted its budget"
            );
            Ok(None)
        }
        Ok(Ok(selection)) => {
            let Some(remaining_budget) = selection_deadline.checked_duration_since(Instant::now())
            else {
                connection.close_on_drop();
                return Ok(None);
            };
            let cleanup_succeeded = {
                let cleanup_result = timeout(remaining_budget, connection.lock_handle()).await;
                match cleanup_result {
                    Ok(Ok(mut handle)) => {
                        handle.remove_progress_handler();
                        true
                    }
                    Ok(Err(_)) | Err(_) => false,
                }
            };
            if cleanup_succeeded {
                Ok(Some(selection))
            } else {
                connection.close_on_drop();
                Ok(None)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ActiveAccountActivityV2RepairOutcome {
    pub(crate) priority_bucket_count: usize,
    pub(crate) repaired_bucket_count: usize,
    pub(crate) elapsed_ms: u64,
}

pub(crate) async fn repair_active_account_activity_v2_coverage(
    pool: &Pool<Sqlite>,
) -> Result<ActiveAccountActivityV2RepairOutcome> {
    let started_at = Instant::now();
    if started_at.elapsed() >= ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUDGET {
        return Ok(ActiveAccountActivityV2RepairOutcome::default());
    }
    let Some(remaining_budget) =
        ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUDGET.checked_sub(started_at.elapsed())
    else {
        return Ok(ActiveAccountActivityV2RepairOutcome::default());
    };
    let mut generation_tx = match timeout(remaining_budget, pool.begin()).await {
        Ok(tx) => tx?,
        Err(_) => return Ok(ActiveAccountActivityV2RepairOutcome::default()),
    };
    ensure_account_activity_v2_repair_generation_tx(generation_tx.as_mut()).await?;
    generation_tx.commit().await?;
    let current_bucket = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0);
    let Some(missing_buckets) =
        select_active_account_activity_v2_priority_buckets(pool, current_bucket, started_at)
            .await?
    else {
        return Ok(ActiveAccountActivityV2RepairOutcome {
            elapsed_ms: started_at.elapsed().as_millis() as u64,
            ..ActiveAccountActivityV2RepairOutcome::default()
        });
    };
    let priority_bucket_count = missing_buckets.len();
    let mut repaired_bucket_count = 0usize;

    for bucket_start_epoch in missing_buckets {
        if started_at.elapsed() >= ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUDGET {
            break;
        }
        let Some(remaining_budget) =
            ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUDGET.checked_sub(started_at.elapsed())
        else {
            break;
        };
        let mut tx = match timeout(remaining_budget, pool.begin()).await {
            Ok(tx) => tx?,
            Err(_) => break,
        };
        ensure_account_activity_v2_repair_generation_tx(tx.as_mut()).await?;
        sqlx::query(
                r#"
                UPDATE upstream_account_stats_hourly
                SET activity_v2_request_count = 0,
                    activity_v2_success_count = 0,
                    activity_v2_failure_count = 0,
                    activity_v2_non_success_count = 0,
                    activity_v2_total_tokens = 0,
                    activity_v2_success_tokens = 0,
                    activity_v2_non_success_tokens = 0,
                    activity_v2_failure_tokens = 0,
                    activity_v2_failure_cost = 0,
                    activity_v2_non_success_cost = 0,
                    activity_v2_cache_input_tokens = 0,
                    activity_v2_total_cost = 0,
                    activity_v2_first_response_sample_count = 0,
                    activity_v2_first_response_sum_ms = 0,
                    activity_v2_first_token_sample_count = 0,
                    activity_v2_first_token_sum_ms = 0,
                    activity_v2_first_token_max_ms = 0,
                    activity_v2_first_token_histogram = '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
                    activity_v2_total_latency_sample_count = 0,
                    activity_v2_total_latency_sum_ms = 0,
                    activity_v2_last_invocation_at = NULL,
                    activity_v2_latest_unkeyed_conversation_at = NULL,
                    activity_v2_latest_first_response_at = NULL,
                    activity_v2_latest_first_response_ms = NULL,
                    activity_v2_latest_total_latency_at = NULL,
                    activity_v2_latest_total_latency_ms = NULL,
                    updated_at = datetime('now')
                WHERE bucket_start_epoch = ?1
                "#,
            )
            .bind(bucket_start_epoch)
            .execute(tx.as_mut())
            .await?;
        let rows = load_live_invocation_hourly_rows_for_bucket_epochs_tx(
            tx.as_mut(),
            &[bucket_start_epoch],
        )
        .await?;
        upsert_invocation_hourly_rollups_tx(
            tx.as_mut(),
            &rows,
            &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2],
        )
        .await?;
        let bucket_max_id = rows
            .iter()
            .filter(|row| invocation_row_counts_toward_account_activity_v2(row))
            .map(|row| row.id)
            .max()
            .unwrap_or_default();
        save_account_activity_v2_bucket_repair_watermark_tx(
            tx.as_mut(),
            bucket_start_epoch,
            bucket_max_id,
        )
        .await?;
        mark_hourly_rollup_bucket_materialized_tx(
            tx.as_mut(),
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2,
            bucket_start_epoch,
            HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
        )
        .await?;
        tx.commit().await?;
        repaired_bucket_count += 1;
    }

    let outcome = ActiveAccountActivityV2RepairOutcome {
        priority_bucket_count,
        repaired_bucket_count,
        elapsed_ms: started_at.elapsed().as_millis() as u64,
    };
    if repaired_bucket_count > 0 {
        tracing::info!(
            priority_mode = "active_dashboard_coverage",
            coverage_priority_bucket_count = priority_bucket_count,
            repaired_bucket_count,
            priority_batch_limit = ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUCKET_LIMIT,
            priority_elapsed_budget_ms =
                ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUDGET.as_millis() as u64,
            elapsed_ms = outcome.elapsed_ms,
            wake_reason = "active_window_coverage_hole",
            "repaired active Dashboard account activity v2 coverage"
        );
    }
    Ok(outcome)
}

fn invocation_row_counts_toward_account_activity_v2(row: &InvocationHourlySourceRecord) -> bool {
    !matches!(
        row.status
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "running" | "pending"
    )
}

async fn save_account_activity_v2_bucket_repair_watermark_tx(
    tx: &mut SqliteConnection,
    bucket_start_epoch: i64,
    cursor_id: i64,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO account_activity_v2_bucket_repair_watermarks (
            bucket_start_epoch, cursor_id, updated_at
        )
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(bucket_start_epoch) DO UPDATE SET
            cursor_id = MAX(account_activity_v2_bucket_repair_watermarks.cursor_id, excluded.cursor_id),
            updated_at = datetime('now')
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(cursor_id.max(0))
    .execute(&mut *tx)
    .await?;
    Ok(())
}

async fn load_account_activity_v2_bucket_repair_watermarks_for_rows_tx(
    tx: &mut SqliteConnection,
    rows: &[InvocationHourlySourceRecord],
) -> Result<BTreeMap<i64, i64>> {
    let mut bucket_epochs = rows
        .iter()
        .map(|row| invocation_bucket_start_epoch(&row.occurred_at))
        .collect::<Result<Vec<_>>>()?;
    bucket_epochs.sort_unstable();
    bucket_epochs.dedup();
    if bucket_epochs.is_empty() {
        return Ok(BTreeMap::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT bucket_start_epoch, cursor_id FROM account_activity_v2_bucket_repair_watermarks WHERE bucket_start_epoch IN (",
    );
    {
        let mut separated = query.separated(", ");
        for bucket_epoch in bucket_epochs {
            separated.push_bind(bucket_epoch);
        }
    }
    query.push(")");
    Ok(query
        .build_query_as::<(i64, i64)>()
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect::<BTreeMap<_, _>>())
}

async fn ensure_account_activity_v2_repair_generation_tx(tx: &mut SqliteConnection) -> Result<()> {
    let generation = load_hourly_rollup_live_progress_tx(
        tx,
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION_DATASET,
    )
    .await?;
    if generation >= INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION {
        return Ok(());
    }

    sqlx::query(
        r#"
        DELETE FROM hourly_rollup_materialized_buckets
        WHERE target = ?1 AND source = ?2
        "#,
    )
    .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
    .bind(HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM account_activity_v2_bucket_repair_watermarks")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        r#"
        UPDATE upstream_account_stats_hourly
        SET activity_v2_request_count = 0,
            activity_v2_success_count = 0,
            activity_v2_failure_count = 0,
            activity_v2_non_success_count = 0,
            activity_v2_total_tokens = 0,
            activity_v2_success_tokens = 0,
            activity_v2_non_success_tokens = 0,
            activity_v2_failure_tokens = 0,
            activity_v2_failure_cost = 0,
            activity_v2_non_success_cost = 0,
            activity_v2_cache_input_tokens = 0,
            activity_v2_total_cost = 0,
            activity_v2_first_response_sample_count = 0,
            activity_v2_first_response_sum_ms = 0,
            activity_v2_first_token_sample_count = 0,
            activity_v2_first_token_sum_ms = 0,
            activity_v2_first_token_max_ms = 0,
            activity_v2_first_token_histogram = '[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]',
            activity_v2_total_latency_sample_count = 0,
            activity_v2_total_latency_sum_ms = 0,
            activity_v2_last_invocation_at = NULL,
            activity_v2_latest_unkeyed_conversation_at = NULL,
            activity_v2_latest_first_response_at = NULL,
            activity_v2_latest_first_response_ms = NULL,
            activity_v2_latest_total_latency_at = NULL,
            activity_v2_latest_total_latency_ms = NULL,
            updated_at = datetime('now')
        "#,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM hourly_rollup_live_progress WHERE dataset = ?1")
        .bind(INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_CURSOR_DATASET)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM hourly_rollup_archive_progress WHERE dataset = ?1")
        .bind(INVOCATION_ACCOUNT_ACTIVITY_V2_ARCHIVE_PROGRESS_DATASET)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2")
        .bind(HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2)
        .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        r#"
        UPDATE archive_batches
        SET historical_rollups_materialized_at = NULL
        WHERE dataset = ?1
          AND status = 'completed'
          AND historical_rollups_materialized_at IS NOT NULL
        "#,
    )
    .bind(HOURLY_ROLLUP_DATASET_INVOCATIONS)
    .execute(&mut *tx)
    .await?;
    save_hourly_rollup_live_progress_tx(
        tx,
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION_DATASET,
        INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION,
    )
    .await?;
    tracing::info!(
        repair_generation = INVOCATION_ACCOUNT_ACTIVITY_V2_REPAIR_GENERATION,
        "initialized account activity v2 repair generation"
    );
    Ok(())
}

async fn mark_live_account_activity_v2_coverage_tx(
    tx: &mut SqliteConnection,
    repair_cursor: i64,
    rows: &[InvocationHourlySourceRecord],
) -> Result<()> {
    let current_bucket = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0);
    let mut bucket_epochs = rows
        .iter()
        .map(|row| invocation_bucket_start_epoch(&row.occurred_at))
        .collect::<Result<Vec<_>>>()?;
    bucket_epochs.sort_unstable();
    bucket_epochs.dedup();
    for bucket_start_epoch in bucket_epochs {
        if bucket_start_epoch >= current_bucket {
            continue;
        }
        let bucket_start = Utc
            .timestamp_opt(bucket_start_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid account activity coverage bucket start"))?;
        let bucket_end = bucket_start + ChronoDuration::hours(1);
        let bucket_max_id = sqlx::query_scalar::<_, Option<i64>>(
            r#"
            SELECT MAX(id)
            FROM codex_invocations
            WHERE occurred_at >= ?1 AND occurred_at < ?2
            "#,
        )
        .bind(db_occurred_at_lower_bound(bucket_start))
        .bind(db_occurred_at_upper_bound(bucket_end))
        .fetch_one(&mut *tx)
        .await?
        .unwrap_or_default();
        if bucket_max_id > repair_cursor {
            continue;
        }
        mark_hourly_rollup_bucket_materialized_tx(
            tx,
            HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_ACTIVITY_V2,
            bucket_start_epoch,
            HOURLY_ROLLUP_MATERIALIZED_SOURCE_NONE,
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn repair_live_invocation_usage_breakdown_rollups(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    if load_hourly_rollup_live_progress(
        pool,
        INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DATASET,
    )
    .await?
        >= INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DONE
    {
        return Ok(());
    }

    loop {
        let updated = repair_live_invocation_usage_breakdown_rollups_once(pool).await?;
        if updated == 0 {
            return Ok(());
        }
    }
}

async fn repair_live_invocation_usage_breakdown_rollups_once(pool: &Pool<Sqlite>) -> Result<u64> {
    let mut tx = pool.begin().await?;
    if load_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DATASET,
    )
    .await?
        >= INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DONE
    {
        tx.rollback().await?;
        return Ok(0);
    }

    let shared_live_cursor =
        load_hourly_rollup_live_progress_tx(tx.as_mut(), HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let repair_cursor = load_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_CURSOR_DATASET,
    )
    .await?;
    if repair_cursor >= shared_live_cursor {
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DATASET,
            INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DONE,
        )
        .await?;
        tx.commit().await?;
        return Ok(0);
    }

    let upstream_account_id_sql = live_invocation_upstream_account_id_sql(
        "codex_invocations",
        load_pool_attempt_fallback_capability_tx(tx.as_mut()).await?,
    );
    let first_token_ms_sql = live_invocation_first_token_ms_sql_tx(tx.as_mut()).await?;
    let rows = sqlx::query_as::<_, InvocationHourlySourceRecord>(&format!(
        r#"
        SELECT
            id,
            occurred_at,
            source,
            status,
            detail_level,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            total_tokens,
            cost,
            {} AS upstream_account_id,
            cost_input,
            cost_cache_write,
            cost_cache_read,
            cost_output,
            cost_reasoning,
            error_message,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            {} AS first_token_ms,
            t_upstream_stream_ms,
            t_resp_parse_ms,
            t_persist_ms
        FROM codex_invocations
        WHERE id > ?1
          AND id <= ?2
        ORDER BY id ASC
        LIMIT ?3
        "#,
        upstream_account_id_sql, first_token_ms_sql,
    ))
    .bind(repair_cursor)
    .bind(shared_live_cursor)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(tx.as_mut())
    .await?;

    if rows.is_empty() {
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_CURSOR_DATASET,
            shared_live_cursor,
        )
        .await?;
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DATASET,
            INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DONE,
        )
        .await?;
        tx.commit().await?;
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(repair_cursor);
    upsert_invocation_hourly_rollups_tx(
        tx.as_mut(),
        &rows,
        &[HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN],
    )
    .await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_CURSOR_DATASET,
        last_id,
    )
    .await?;
    if last_id >= shared_live_cursor {
        save_hourly_rollup_live_progress_tx(
            tx.as_mut(),
            INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DATASET,
            INVOCATION_USAGE_BREAKDOWN_ROLLUP_REPAIR_MARKER_DONE,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(rows.len() as u64)
}

pub(crate) async fn replay_live_forward_proxy_attempt_hourly_rollups(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress(pool, HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
            .await?;
    let rows = sqlx::query_as::<_, ForwardProxyAttemptHourlySourceRecord>(
        r#"
        SELECT
            id,
            proxy_key,
            occurred_at,
            is_success,
            latency_ms
        FROM forward_proxy_attempts
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    let mut tx = pool.begin().await?;
    upsert_forward_proxy_attempt_hourly_rollups_tx(tx.as_mut(), &rows).await?;
    save_hourly_rollup_live_progress_tx(
        tx.as_mut(),
        HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS,
        last_id,
    )
    .await?;
    tx.commit().await?;
    Ok(rows.len() as u64)
}

pub(crate) async fn replay_live_forward_proxy_attempt_hourly_rollups_tx(
    tx: &mut SqliteConnection,
) -> Result<u64> {
    let cursor_id =
        load_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS)
            .await?;
    let rows = sqlx::query_as::<_, ForwardProxyAttemptHourlySourceRecord>(
        r#"
        SELECT
            id,
            proxy_key,
            occurred_at,
            is_success,
            latency_ms
        FROM forward_proxy_attempts
        WHERE id > ?1
        ORDER BY id ASC
        LIMIT ?2
        "#,
    )
    .bind(cursor_id)
    .bind(BACKFILL_BATCH_SIZE)
    .fetch_all(&mut *tx)
    .await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let last_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
    upsert_forward_proxy_attempt_hourly_rollups_tx(tx, &rows).await?;
    save_hourly_rollup_live_progress_tx(tx, HOURLY_ROLLUP_DATASET_FORWARD_PROXY_ATTEMPTS, last_id)
        .await?;
    Ok(rows.len() as u64)
}

#[derive(Debug, Clone, FromRow)]
struct UpstreamHostNetworkMinuteSourceRow {
    id: i64,
    bucket_start_epoch: i64,
    source: String,
    upstream_base_url_host: String,
    upload_bytes: i64,
    download_bytes: i64,
}
