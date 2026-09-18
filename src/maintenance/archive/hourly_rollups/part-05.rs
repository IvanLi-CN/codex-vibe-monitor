async fn select_active_account_activity_v2_priority_buckets(
    pool: &Pool<Sqlite>,
    current_bucket: i64,
    started_at: Instant,
) -> Result<Option<Vec<i64>>> {
    select_active_account_activity_v2_priority_buckets_with_deadline(
        pool,
        current_bucket,
        started_at,
        started_at + ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUDGET,
        None,
        1_000,
        false,
    )
    .await
}

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

    let selection = timeout(
        remaining_budget,
        select_active_account_activity_v2_buckets_from_connection(
            &mut connection,
            current_bucket,
            selection_deadline,
        ),
    )
    .await;
    finish_active_account_activity_v2_selection(
        &mut connection,
        selection,
        selection_deadline,
        progress_abort_on_probe,
    )
    .await
}

async fn finish_active_account_activity_v2_selection(
    connection: &mut sqlx::pool::PoolConnection<Sqlite>,
    selection: std::result::Result<Result<Vec<i64>>, tokio::time::error::Elapsed>,
    selection_deadline: Instant,
    progress_abort_on_probe: bool,
) -> Result<Option<Vec<i64>>> {
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

struct ActiveAccountActivityV2SelectionInputs {
    oldest_bucket: i64,
    covered_buckets: HashSet<i64>,
    archive_epoch_coverage: Vec<(i64, i64)>,
    archive_legacy_month_keys: HashSet<String>,
}

async fn select_active_account_activity_v2_buckets_from_connection(
    connection: &mut sqlx::pool::PoolConnection<Sqlite>,
    current_bucket: i64,
    selection_deadline: Instant,
) -> Result<Vec<i64>> {
    let oldest_live_occurred_at =
        sqlx::query_scalar::<_, Option<String>>("SELECT MIN(occurred_at) FROM codex_invocations")
            .fetch_one(&mut **connection)
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
    let oldest_bucket = (current_bucket - 7 * 24 * 3_600).max(oldest_live_bucket);
    if oldest_bucket >= current_bucket {
        return Ok(Vec::new());
    }
    let inputs = load_active_account_activity_v2_selection_inputs(
        connection,
        oldest_bucket,
        current_bucket,
        selection_deadline,
    )
    .await?;
    let mut missing_buckets = Vec::with_capacity(ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUCKET_LIMIT);
    let mut bucket_start_epoch = current_bucket - 3_600;
    while bucket_start_epoch >= inputs.oldest_bucket
        && missing_buckets.len() < ACTIVE_ACCOUNT_ACTIVITY_V2_REPAIR_BUCKET_LIMIT
    {
        if Instant::now() >= selection_deadline {
            return Err(anyhow!(
                "active account activity v2 coverage selection deadline reached during archive scan"
            ));
        }
        let month_key = active_account_activity_v2_month_key(bucket_start_epoch)?;
        let is_archive_covered = active_account_activity_v2_bucket_is_archive_covered(
            bucket_start_epoch,
            &month_key,
            &inputs,
            selection_deadline,
        )?;
        if !inputs.covered_buckets.contains(&bucket_start_epoch) && !is_archive_covered {
            missing_buckets.push(bucket_start_epoch);
        }
        bucket_start_epoch -= 3_600;
    }
    Ok(missing_buckets)
}

async fn load_active_account_activity_v2_selection_inputs(
    connection: &mut sqlx::pool::PoolConnection<Sqlite>,
    oldest_bucket: i64,
    current_bucket: i64,
    selection_deadline: Instant,
) -> Result<ActiveAccountActivityV2SelectionInputs> {
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
    .fetch_all(&mut **connection)
    .await?;
    let covered_buckets = collect_active_account_activity_v2_covered_buckets(
        covered_bucket_rows,
        selection_deadline,
    )?;
    let archive_epoch_coverage = build_active_account_activity_v2_archive_epoch_coverage_query(
        "",
        oldest_bucket,
        current_bucket,
    )
    .build_query_as::<(i64, i64)>()
    .fetch_all(&mut **connection)
    .await?;
    let active_month_keys =
        active_account_activity_v2_month_keys(oldest_bucket, current_bucket, selection_deadline)?;
    let archive_legacy_month_rows =
        build_active_account_activity_v2_legacy_coverage_query("", &active_month_keys)
            .build_query_scalar::<String>()
            .fetch_all(&mut **connection)
            .await?;
    let archive_legacy_month_keys = collect_active_account_activity_v2_legacy_month_keys(
        archive_legacy_month_rows,
        selection_deadline,
    )?;
    Ok(ActiveAccountActivityV2SelectionInputs {
        oldest_bucket,
        covered_buckets,
        archive_epoch_coverage,
        archive_legacy_month_keys,
    })
}

fn collect_active_account_activity_v2_covered_buckets(
    bucket_rows: Vec<i64>,
    selection_deadline: Instant,
) -> Result<HashSet<i64>> {
    let mut covered_buckets = HashSet::new();
    for bucket_start_epoch in bucket_rows {
        if Instant::now() >= selection_deadline {
            return Err(anyhow!(
                "active account activity v2 coverage selection deadline reached during coverage scan"
            ));
        }
        covered_buckets.insert(bucket_start_epoch);
    }
    Ok(covered_buckets)
}

fn active_account_activity_v2_month_keys(
    oldest_bucket: i64,
    current_bucket: i64,
    selection_deadline: Instant,
) -> Result<Vec<String>> {
    let mut month_keys = Vec::new();
    let mut bucket_start_epoch = current_bucket - 3_600;
    while bucket_start_epoch >= oldest_bucket {
        if Instant::now() >= selection_deadline {
            return Err(anyhow!(
                "active account activity v2 coverage selection deadline reached during legacy month scan"
            ));
        }
        let month_key = active_account_activity_v2_month_key(bucket_start_epoch)?;
        if !month_keys.contains(&month_key) {
            month_keys.push(month_key);
        }
        bucket_start_epoch -= 3_600;
    }
    Ok(month_keys)
}

fn collect_active_account_activity_v2_legacy_month_keys(
    month_rows: Vec<String>,
    selection_deadline: Instant,
) -> Result<HashSet<String>> {
    let mut month_keys = HashSet::new();
    for month_key in month_rows {
        if Instant::now() >= selection_deadline {
            return Err(anyhow!(
                "active account activity v2 coverage selection deadline reached during legacy coverage scan"
            ));
        }
        month_keys.insert(month_key);
    }
    Ok(month_keys)
}

fn active_account_activity_v2_bucket_is_archive_covered(
    bucket_start_epoch: i64,
    month_key: &str,
    inputs: &ActiveAccountActivityV2SelectionInputs,
    selection_deadline: Instant,
) -> Result<bool> {
    if inputs.archive_legacy_month_keys.contains(month_key) {
        return Ok(true);
    }
    for (coverage_start_epoch, coverage_end_epoch) in &inputs.archive_epoch_coverage {
        if Instant::now() >= selection_deadline {
            return Err(anyhow!(
                "active account activity v2 coverage selection deadline reached during archive scan"
            ));
        }
        if *coverage_start_epoch < bucket_start_epoch + 3_600
            && *coverage_end_epoch >= bucket_start_epoch
        {
            return Ok(true);
        }
    }
    Ok(false)
}
