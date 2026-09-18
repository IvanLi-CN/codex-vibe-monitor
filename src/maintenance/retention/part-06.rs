pub(crate) async fn archive_timestamped_dataset(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    select_sql: &str,
    cutoff: String,
    dry_run: bool,
) -> Result<(usize, usize)> {
    if dry_run {
        return dry_run_timestamped_dataset(pool, spec, &cutoff).await;
    }

    let mut rows_archived = 0usize;
    let mut archive_batches = 0usize;

    loop {
        let candidate_limit = retention_candidate_limit(config, "timestamped_archive");
        let candidates = sqlx::query_as::<_, TimestampedArchiveCandidate>(select_sql)
            .bind(&cutoff)
            .bind(candidate_limit as i64)
            .fetch_all(pool)
            .await?;

        if candidates.is_empty() {
            break;
        }

        let candidate_remaining_hint = usize::from(candidates.len() >= candidate_limit);
        let mut by_month: BTreeMap<String, Vec<TimestampedArchiveCandidate>> = BTreeMap::new();
        for candidate in candidates {
            let month_key =
                archive_timestamped_dataset_month_key(spec.dataset, &candidate.timestamp_value)?;
            by_month.entry(month_key).or_default().push(candidate);
        }

        for (month_key, group) in by_month {
            let group = take_retention_micro_batch(group, |_| 256);
            let Some((rows, batch_count)) = archive_timestamped_dataset_group(
                pool,
                config,
                spec,
                &month_key,
                group,
                candidate_remaining_hint,
            )
            .await?
            else {
                return Ok((rows_archived, archive_batches));
            };
            rows_archived += rows;
            archive_batches += batch_count;
        }
    }

    Ok((rows_archived, archive_batches))
}

async fn dry_run_timestamped_dataset(
    pool: &Pool<Sqlite>,
    spec: ArchiveTableSpec,
    cutoff: &str,
) -> Result<(usize, usize)> {
    let dry_run_sql = match spec.dataset {
        "forward_proxy_attempts" => {
            r#"
            SELECT strftime('%Y-%m', datetime(occurred_at, '+8 hours')) AS month_key,
                   COUNT(*) AS row_count
            FROM forward_proxy_attempts
            WHERE occurred_at < ?1
            GROUP BY 1
            ORDER BY 1
            "#
        }
        "pool_upstream_request_attempts" => {
            r#"
            SELECT strftime('%Y-%m', occurred_at) AS month_key,
                   COUNT(*) AS row_count
            FROM pool_upstream_request_attempts
            WHERE occurred_at < ?1
            GROUP BY 1
            ORDER BY 1
            "#
        }
        other => bail!("unsupported dry-run archive dataset: {other}"),
    };
    let batch_counts = sqlx::query_as::<_, DryRunBatchCount>(dry_run_sql)
        .bind(cutoff)
        .fetch_all(pool)
        .await?;
    for batch in &batch_counts {
        info!(
            dataset = spec.dataset,
            month_key = %batch.month_key,
            rows = batch.row_count,
            "retention dry-run planned archive batch"
        );
    }
    Ok((
        batch_counts
            .iter()
            .map(|batch| batch.row_count as usize)
            .sum(),
        batch_counts.len(),
    ))
}

struct PreparedTimestampedArchiveBatch {
    outcome: ArchiveBatchOutcome,
    ids: Vec<i64>,
    pool_attempt_raw_paths: Vec<Option<String>>,
    materialized_forward_proxy_rows: Vec<ForwardProxyAttemptHourlySourceRecord>,
    prepare_elapsed: Duration,
    candidate_remaining_hint: usize,
    group_len: usize,
}

async fn archive_timestamped_dataset_group(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    month_key: &str,
    group: Vec<TimestampedArchiveCandidate>,
    candidate_remaining_hint: usize,
) -> Result<Option<(usize, usize)>> {
    let group_len = group.len();
    let batch = prepare_timestamped_archive_batch(
        pool,
        config,
        spec,
        month_key,
        group,
        candidate_remaining_hint,
    )
    .await?;
    let Some(batch) = batch else {
        return Ok(None);
    };
    let raw_paths = batch.pool_attempt_raw_paths.clone();
    if !commit_timestamped_archive_batch(pool, spec, batch).await? {
        return Ok(None);
    }
    if spec.dataset == "pool_upstream_request_attempts" {
        let raw_paths = filter_unreferenced_proxy_raw_paths(pool, &raw_paths).await?;
        let _ = delete_proxy_raw_paths(&raw_paths, config.database_path.parent())?;
    }
    Ok(Some((group_len, 1)))
}

async fn prepare_timestamped_archive_batch(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    spec: ArchiveTableSpec,
    month_key: &str,
    group: Vec<TimestampedArchiveCandidate>,
    candidate_remaining_hint: usize,
) -> Result<Option<PreparedTimestampedArchiveBatch>> {
    let prepare_started = Instant::now();
    let ids = group
        .iter()
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();
    let pool_attempt_raw_paths = if spec.dataset == "pool_upstream_request_attempts" {
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let query = format!(
            "SELECT response_raw_path FROM pool_upstream_request_attempts WHERE id IN ({placeholders})"
        );
        let mut query_builder = sqlx::query_scalar::<_, Option<String>>(&query);
        for id in &ids {
            query_builder = query_builder.bind(id);
        }
        query_builder.fetch_all(pool).await?
    } else {
        Vec::new()
    };
    let recreated_pool_upstream_month_archive = if spec.dataset == "pool_upstream_request_attempts"
    {
        let archive_file_path = archive_batch_file_path(config, spec.dataset, month_key)?
            .to_string_lossy()
            .to_string();
        pool_upstream_month_archive_reappeared_after_cleanup(pool, &archive_file_path).await?
    } else {
        false
    };
    let materialized_forward_proxy_rows = if spec.dataset == "forward_proxy_attempts" {
        group
            .iter()
            .map(|candidate| ForwardProxyAttemptHourlySourceRecord {
                id: candidate.id,
                proxy_key: String::new(),
                occurred_at: candidate.timestamp_value.clone(),
                is_success: 0,
                latency_ms: None,
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let Some(mut outcome) = retention_prepared_batch_or_deferred(
        archive_rows_into_month_batch(pool, config, spec, month_key, &ids).await,
    )?
    else {
        return Ok(None);
    };
    if spec.dataset == "pool_upstream_request_attempts" {
        set_archive_batch_coverage_from_local_rows(
            &mut outcome,
            group
                .iter()
                .map(|candidate| candidate.timestamp_value.as_str()),
            Some(config.pool_upstream_request_attempts_archive_ttl_days),
        )?;
        if recreated_pool_upstream_month_archive && outcome.row_count == ids.len() as i64 {
            outcome.archive_expires_at = Some(shanghai_archive_expiry_from_reference_timestamp(
                &format_utc_iso(Utc::now()),
                config.pool_upstream_request_attempts_archive_ttl_days,
            )?);
        }
    } else {
        set_archive_batch_coverage_from_utc_rows(
            &mut outcome,
            group
                .iter()
                .map(|candidate| candidate.timestamp_value.as_str()),
        )?;
    }
    Ok(Some(PreparedTimestampedArchiveBatch {
        outcome,
        ids,
        pool_attempt_raw_paths,
        materialized_forward_proxy_rows,
        prepare_elapsed: prepare_started.elapsed(),
        candidate_remaining_hint,
        group_len: group.len(),
    }))
}

async fn commit_timestamped_archive_batch(
    pool: &Pool<Sqlite>,
    spec: ArchiveTableSpec,
    batch: PreparedTimestampedArchiveBatch,
) -> Result<bool> {
    let Some(admission) = acquire_retention_write_admission("timestamped_archive").await else {
        return Ok(false);
    };
    let execute_started = Instant::now();
    let mut tx = pool.begin().await?;
    upsert_archive_batch_manifest(tx.as_mut(), &batch.outcome).await?;
    if spec.dataset == "pool_upstream_request_attempts" {
        persist_pool_timestamped_archive_rollup_state(tx.as_mut(), spec, &batch).await?;
    } else {
        mark_archive_batch_historical_rollups_materialized_tx(
            tx.as_mut(),
            spec.dataset,
            &batch.outcome.file_path,
        )
        .await?;
    }
    delete_rows_by_ids(tx.as_mut(), spec.dataset, &batch.ids).await?;
    mark_retention_archived_hourly_rollup_targets_tx(
        tx.as_mut(),
        spec.dataset,
        &[],
        &batch.materialized_forward_proxy_rows,
    )
    .await?;
    let commit_started = Instant::now();
    tx.commit().await?;
    retention_record_commit!(
        "timestamped_archive",
        admission.admission_mode(),
        batch.group_len,
        batch.group_len.saturating_mul(256),
        batch.prepare_elapsed,
        admission.lock_wait(),
        commit_started.duration_since(execute_started),
        commit_started.elapsed(),
        admission.p1_waiter_count,
        batch.candidate_remaining_hint,
    );
    drop(admission);
    Ok(true)
}

async fn persist_pool_timestamped_archive_rollup_state(
    tx: &mut SqliteConnection,
    spec: ArchiveTableSpec,
    batch: &PreparedTimestampedArchiveBatch,
) -> Result<()> {
    let archive_batch_id = load_archive_batch_id_for_file_tx(
        tx,
        spec.dataset,
        &batch.outcome.month_key,
        &batch.outcome.file_path,
    )
    .await?;
    let only_new_rows = batch.outcome.row_count == batch.ids.len() as i64;
    let health_replayed = hourly_rollup_archive_replayed_tx(
        tx,
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        spec.dataset,
        &batch.outcome.file_path,
    )
    .await?;
    let hourly_replayed = hourly_rollup_archive_replayed_tx(
        tx,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
        spec.dataset,
        &batch.outcome.file_path,
    )
    .await?;
    cache_pool_upstream_node_health_archive_rows_from_live_ids_tx(
        tx,
        &batch.outcome.file_path,
        &batch.ids,
    )
    .await?;
    refresh_pool_upstream_node_health_hourly_archive_rows_from_cache_tx(
        tx,
        archive_batch_id,
        &batch.outcome.file_path,
    )
    .await?;
    sync_pool_timestamped_archive_target(
        tx,
        spec.dataset,
        &batch.outcome.file_path,
        POOL_UPSTREAM_NODE_HEALTH_HOURLY_ARCHIVE_REPLAY_TARGET,
        only_new_rows,
        hourly_replayed,
        false,
    )
    .await?;
    sync_pool_timestamped_archive_target(
        tx,
        spec.dataset,
        &batch.outcome.file_path,
        POOL_UPSTREAM_NODE_HEALTH_ARCHIVE_REPLAY_TARGET,
        only_new_rows,
        health_replayed,
        true,
    )
    .await
}

async fn sync_pool_timestamped_archive_target(
    tx: &mut SqliteConnection,
    dataset: &str,
    file_path: &str,
    target: &str,
    only_new_rows: bool,
    already_replayed: bool,
    mark_materialized: bool,
) -> Result<()> {
    if only_new_rows || already_replayed {
        mark_hourly_rollup_archive_replayed_tx(tx, target, dataset, file_path).await?;
        if mark_materialized {
            mark_archive_batch_historical_rollups_materialized_tx(tx, dataset, file_path).await?;
        }
        return Ok(());
    }
    sqlx::query(
        "DELETE FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = ?2 AND file_path = ?3",
    )
    .bind(target)
    .bind(dataset)
    .bind(file_path)
    .execute(&mut *tx)
    .await?;
    if mark_materialized {
        sqlx::query(
            "UPDATE archive_batches SET historical_rollups_materialized_at = NULL WHERE dataset = ?1 AND file_path = ?2",
        )
        .bind(dataset)
        .bind(file_path)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

pub(crate) fn archive_timestamped_dataset_month_key(
    dataset: &str,
    timestamp_value: &str,
) -> Result<String> {
    match dataset {
        "pool_upstream_request_attempts" => shanghai_month_key_from_local_naive(timestamp_value),
        _ => shanghai_month_key_from_utc_naive(timestamp_value),
    }
}

pub(crate) fn set_archive_batch_coverage_from_local_rows<'a>(
    batch: &mut ArchiveBatchOutcome,
    rows: impl Iterator<Item = &'a str>,
    archive_ttl_days: Option<u64>,
) -> Result<()> {
    let values = rows.collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(());
    }
    let mut sorted = values.into_iter().map(str::to_string).collect::<Vec<_>>();
    sorted.sort();
    batch.coverage_start_at = sorted.first().cloned();
    batch.coverage_end_at = sorted.last().cloned();
    batch.archive_expires_at = match (batch.coverage_end_at.as_deref(), archive_ttl_days) {
        (Some(coverage_end_at), Some(ttl_days)) => Some(
            shanghai_archive_expiry_from_local_timestamp(coverage_end_at, ttl_days)?,
        ),
        _ => None,
    };
    Ok(())
}

pub(crate) async fn pool_upstream_month_archive_reappeared_after_cleanup(
    pool: &Pool<Sqlite>,
    archive_file_path: &str,
) -> Result<bool> {
    let existing_manifest_rows: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM archive_batches
        WHERE dataset = 'pool_upstream_request_attempts'
          AND file_path = ?1
        "#,
    )
    .bind(archive_file_path)
    .fetch_one(pool)
    .await?;
    if existing_manifest_rows > 0 {
        return Ok(false);
    }

    let existing_hourly_rows: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_node_health_hourly_archive
        WHERE archive_file_path = ?1
        "#,
    )
    .bind(archive_file_path)
    .fetch_one(pool)
    .await?;

    Ok(existing_hourly_rows > 0)
}

pub(crate) fn set_archive_batch_coverage_from_utc_rows<'a>(
    batch: &mut ArchiveBatchOutcome,
    rows: impl Iterator<Item = &'a str>,
) -> Result<()> {
    let values = rows.collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(());
    }
    let mut sorted = values.into_iter().map(str::to_string).collect::<Vec<_>>();
    sorted.sort();
    batch.coverage_start_at = sorted.first().cloned();
    batch.coverage_end_at = sorted.last().cloned();
    batch.archive_expires_at = None;
    Ok(())
}

pub(crate) fn shanghai_archive_expiry_from_local_timestamp(
    value: &str,
    archive_ttl_days: u64,
) -> Result<String> {
    let local = parse_shanghai_local_naive(value)?;
    shanghai_archive_expiry_from_local_naive(local, archive_ttl_days)
}

pub(crate) fn shanghai_archive_expiry_from_reference_timestamp(
    value: &str,
    archive_ttl_days: u64,
) -> Result<String> {
    let local = match parse_to_utc_datetime(value) {
        Some(value) => value.with_timezone(&Shanghai).naive_local(),
        None => parse_shanghai_local_naive(value)?,
    };
    shanghai_archive_expiry_from_local_naive(local, archive_ttl_days)
}

pub(crate) fn shanghai_archive_expiry_from_local_naive(
    local: NaiveDateTime,
    archive_ttl_days: u64,
) -> Result<String> {
    let expiry = start_of_local_day(local_naive_to_utc(local, Shanghai), Shanghai)
        + ChronoDuration::days(archive_ttl_days as i64 + 1);
    Ok(format_naive(expiry.with_timezone(&Shanghai).naive_local()))
}
