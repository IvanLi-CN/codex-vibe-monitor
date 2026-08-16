use super::*;

pub(crate) fn collect_account_window_usage_plans(
    items: &[UpstreamAccountSummary],
    now: DateTime<Utc>,
) -> Option<(
    HashMap<i64, AccountWindowUsagePlan>,
    DateTime<Utc>,
    DateTime<Utc>,
)> {
    let mut plans = HashMap::new();
    let mut earliest_start_at: Option<DateTime<Utc>> = None;
    let mut latest_end_at: Option<DateTime<Utc>> = None;

    for item in items {
        let primary = item.primary_window.as_ref().and_then(|window| {
            build_window_usage_range(
                now,
                window.window_duration_mins,
                window.resets_at.as_deref(),
            )
        });
        let secondary = item.secondary_window.as_ref().and_then(|window| {
            build_window_usage_range(
                now,
                window.window_duration_mins,
                window.resets_at.as_deref(),
            )
        });
        if primary.is_none() && secondary.is_none() {
            continue;
        }

        for range in [primary.as_ref(), secondary.as_ref()].into_iter().flatten() {
            earliest_start_at = Some(
                earliest_start_at
                    .map(|value| value.min(range.start_at))
                    .unwrap_or(range.start_at),
            );
            latest_end_at = Some(
                latest_end_at
                    .map(|value| value.max(range.end_at))
                    .unwrap_or(range.end_at),
            );
        }

        plans.insert(
            item.id,
            AccountWindowUsagePlan {
                primary: primary.map(AccountWindowUsageRangeBounds::into_range),
                secondary: secondary.map(AccountWindowUsageRangeBounds::into_range),
            },
        );
    }

    Some((plans, earliest_start_at?, latest_end_at?))
}

pub(crate) fn build_window_usage_range(
    now: DateTime<Utc>,
    window_duration_mins: i64,
    resets_at: Option<&str>,
) -> Option<AccountWindowUsageRangeBounds> {
    if window_duration_mins <= 0 {
        return None;
    }
    let reset_anchor = resets_at.and_then(parse_rfc3339_utc);
    let window_anchor = reset_anchor.unwrap_or(now);
    Some(AccountWindowUsageRangeBounds {
        start_at: window_anchor - ChronoDuration::minutes(window_duration_mins),
        end_at: window_anchor.min(now),
        // A moving window can only change membership on a minute boundary. A future reset
        // still grows until that boundary, so include both its anchor and the minute generation.
        selection_generation: match reset_anchor {
            Some(anchor) if anchor <= now => format!("reset:{}", anchor.timestamp()),
            Some(anchor) => format!(
                "reset-pending:{}:minute:{}",
                anchor.timestamp(),
                now.timestamp().div_euclid(60)
            ),
            None => format!("minute:{}", now.timestamp().div_euclid(60)),
        },
        window_duration_secs: window_duration_mins.saturating_mul(60),
    })
}

async fn detail_exact_upstream_account_id_sql(pool: &Pool<Sqlite>) -> Result<&'static str> {
    let has_structured_column = sqlx::query_scalar::<_, String>(
        "SELECT name FROM pragma_table_info('codex_invocations') WHERE name = 'upstream_account_id' LIMIT 1",
    )
    .fetch_optional(pool)
    .await?
    .is_some();
    Ok(if has_structured_column {
        "COALESCE(upstream_account_id, CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END)"
    } else {
        "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END"
    })
}

pub(crate) async fn load_window_actual_usage_rows_for_detail_exact_from_pool(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
    start_at: &str,
    end_at: &str,
) -> Result<Vec<AccountWindowUsageRow>> {
    if account_ids.is_empty() {
        return Ok(Vec::new());
    }

    let upstream_account_id_sql = detail_exact_upstream_account_id_sql(pool).await?;
    let mut query = QueryBuilder::<Sqlite>::new("SELECT id, occurred_at, ");
    query
        .push(upstream_account_id_sql)
        .push(
            " AS upstream_account_id, input_tokens, output_tokens, cache_input_tokens, total_tokens, cost \
             FROM codex_invocations WHERE occurred_at >= ",
        )
        .push_bind(start_at)
        .push(" AND occurred_at < ")
        .push_bind(end_at)
        .push(" AND ")
        .push(upstream_account_id_sql)
        .push(" IN (");
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    query.push(") ORDER BY occurred_at ASC, id ASC");

    query
        .build_query_as::<AccountWindowUsageRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn load_window_actual_usage_rows_for_plans_from_pool(
    pool: &Pool<Sqlite>,
    plans: &HashMap<i64, AccountWindowUsagePlan>,
    min_id_exclusive: Option<i64>,
    max_id_inclusive: Option<i64>,
    limit: Option<usize>,
) -> Result<Vec<AccountWindowUsageRow>> {
    let ranges = collect_account_window_exact_ranges(plans);
    if ranges.is_empty() {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            id,
            occurred_at,
            upstream_account_id,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            total_tokens,
            cost
        FROM codex_invocations
        WHERE upstream_account_id IS NOT NULL
        "#,
    );
    if let Some(min_id_exclusive) = min_id_exclusive {
        query.push(" AND id > ").push_bind(min_id_exclusive.max(0));
    }
    if let Some(max_id_inclusive) = max_id_inclusive {
        query.push(" AND id <= ").push_bind(max_id_inclusive.max(0));
    }
    query.push(" AND (");
    for (index, (account_id, start_at, end_at)) in ranges.iter().enumerate() {
        if index > 0 {
            query.push(" OR ");
        }
        query
            .push("(upstream_account_id = ")
            .push_bind(account_id)
            .push(" AND occurred_at >= ")
            .push_bind(start_at)
            .push(" AND occurred_at < ")
            .push_bind(end_at)
            .push(")");
    }
    query.push(") ORDER BY occurred_at ASC, id ASC");
    if let Some(limit) = limit {
        query.push(" LIMIT ").push_bind(limit as i64);
    }

    query
        .build_query_as::<AccountWindowUsageRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn load_window_actual_usage_rows_for_bucket_epochs_from_pool(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
    bucket_epochs: &HashSet<i64>,
    bucket_seconds: i64,
    min_id_exclusive: Option<i64>,
    max_id_inclusive: Option<i64>,
) -> Result<Vec<AccountWindowUsageRow>> {
    if account_ids.is_empty() || bucket_epochs.is_empty() || bucket_seconds <= 0 {
        return Ok(Vec::new());
    }

    let mut sorted_bucket_epochs = bucket_epochs.iter().copied().collect::<Vec<_>>();
    sorted_bucket_epochs.sort_unstable();
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            id,
            occurred_at,
            upstream_account_id,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            total_tokens,
            cost
        FROM codex_invocations
        WHERE
        "#,
    );

    if let Some(min_id_exclusive) = min_id_exclusive {
        query
            .push(" id > ")
            .push_bind(min_id_exclusive.max(0))
            .push(" AND");
    }
    if let Some(max_id_inclusive) = max_id_inclusive {
        query
            .push(" id <= ")
            .push_bind(max_id_inclusive.max(0))
            .push(" AND");
    }

    query.push(" (");
    for (index, bucket_epoch) in sorted_bucket_epochs.iter().enumerate() {
        if index > 0 {
            query.push(" OR ");
        }
        let bucket_start = Utc
            .timestamp_opt(*bucket_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid usage bucket epoch: {bucket_epoch}"))?;
        let bucket_end = Utc
            .timestamp_opt(bucket_epoch.saturating_add(bucket_seconds), 0)
            .single()
            .ok_or_else(|| anyhow!("invalid usage bucket end epoch: {bucket_epoch}"))?;
        query
            .push("(occurred_at >= ")
            .push_bind(format_naive(
                bucket_start.with_timezone(&Shanghai).naive_local(),
            ))
            .push(" AND occurred_at < ")
            .push_bind(format_naive(
                bucket_end.with_timezone(&Shanghai).naive_local(),
            ))
            .push(")");
    }
    query.push(") AND upstream_account_id IS NOT NULL AND upstream_account_id IN (");
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    query.push(") ORDER BY occurred_at ASC");

    query
        .build_query_as::<AccountWindowUsageRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn load_window_actual_usage_rows_for_account_bucket_keys_from_pool(
    pool: &Pool<Sqlite>,
    bucket_keys: &HashSet<(i64, i64)>,
    bucket_seconds: i64,
    min_id_exclusive: Option<i64>,
    max_id_inclusive: Option<i64>,
    limit: Option<usize>,
) -> Result<Vec<AccountWindowUsageRow>> {
    if bucket_keys.is_empty() || bucket_seconds <= 0 {
        return Ok(Vec::new());
    }

    let mut sorted_bucket_keys = bucket_keys.iter().copied().collect::<Vec<_>>();
    sorted_bucket_keys.sort_unstable();
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            id,
            occurred_at,
            upstream_account_id,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            total_tokens,
            cost
        FROM codex_invocations
        WHERE upstream_account_id IS NOT NULL
        "#,
    );

    if let Some(min_id_exclusive) = min_id_exclusive {
        query.push(" AND id > ").push_bind(min_id_exclusive.max(0));
    }
    if let Some(max_id_inclusive) = max_id_inclusive {
        query.push(" AND id <= ").push_bind(max_id_inclusive.max(0));
    }

    query.push(" AND (");
    for (index, (account_id, bucket_epoch)) in sorted_bucket_keys.iter().enumerate() {
        if index > 0 {
            query.push(" OR ");
        }
        let bucket_start = Utc
            .timestamp_opt(*bucket_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid usage bucket epoch: {bucket_epoch}"))?;
        let bucket_end = Utc
            .timestamp_opt(bucket_epoch.saturating_add(bucket_seconds), 0)
            .single()
            .ok_or_else(|| anyhow!("invalid usage bucket end epoch: {bucket_epoch}"))?;
        query
            .push("(upstream_account_id = ")
            .push_bind(account_id)
            .push(" AND occurred_at >= ")
            .push_bind(format_naive(
                bucket_start.with_timezone(&Shanghai).naive_local(),
            ))
            .push(" AND occurred_at < ")
            .push_bind(format_naive(
                bucket_end.with_timezone(&Shanghai).naive_local(),
            ))
            .push(")");
    }
    query.push(") ORDER BY occurred_at ASC, id ASC");
    if let Some(limit) = limit {
        query.push(" LIMIT ").push_bind(limit as i64);
    }

    query
        .build_query_as::<AccountWindowUsageRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn load_window_actual_usage_hourly_rows_from_pool(
    pool: &Pool<Sqlite>,
    account_ranges: &[(i64, i64, i64)],
) -> Result<Vec<AccountWindowUsageHourlyRow>> {
    if account_ranges.is_empty() {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            bucket_start_epoch,
            upstream_account_id,
            request_count,
            total_tokens,
            total_cost,
            input_tokens,
            output_tokens,
            cache_input_tokens
        FROM upstream_account_usage_hourly
        WHERE
        "#,
    );
    query.push(" (");
    for (index, (account_id, start_bucket_epoch, end_bucket_epoch_exclusive)) in
        account_ranges.iter().enumerate()
    {
        if index > 0 {
            query.push(" OR ");
        }
        query
            .push("(upstream_account_id = ")
            .push_bind(account_id)
            .push(" AND bucket_start_epoch >= ")
            .push_bind(start_bucket_epoch)
            .push(" AND bucket_start_epoch < ")
            .push_bind(end_bucket_epoch_exclusive)
            .push(")");
    }
    query.push(") ORDER BY upstream_account_id ASC, bucket_start_epoch ASC");

    query
        .build_query_as::<AccountWindowUsageHourlyRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn load_window_actual_usage_minute_rows_from_pool(
    pool: &Pool<Sqlite>,
    account_ranges: &[(i64, i64, i64)],
) -> Result<Vec<AccountWindowUsageMinuteRow>> {
    if account_ranges.is_empty() {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            bucket_start_epoch,
            upstream_account_id,
            SUM(total_count) AS request_count,
            SUM(total_tokens) AS total_tokens,
            SUM(total_cost) AS total_cost,
            SUM(input_tokens) AS input_tokens,
            SUM(output_tokens) AS output_tokens,
            SUM(cache_input_tokens) AS cache_input_tokens
        FROM upstream_account_stats_minute
        WHERE
        "#,
    );
    query.push(" (");
    for (index, (account_id, start_bucket_epoch, end_bucket_epoch_exclusive)) in
        account_ranges.iter().enumerate()
    {
        if index > 0 {
            query.push(" OR ");
        }
        query
            .push("(upstream_account_id = ")
            .push_bind(account_id)
            .push(" AND bucket_start_epoch >= ")
            .push_bind(start_bucket_epoch)
            .push(" AND bucket_start_epoch < ")
            .push_bind(end_bucket_epoch_exclusive)
            .push(")");
    }
    query.push(") GROUP BY bucket_start_epoch, upstream_account_id ORDER BY upstream_account_id ASC, bucket_start_epoch ASC");

    query
        .build_query_as::<AccountWindowUsageMinuteRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn load_window_actual_usage_rows_for_account_bucket_keys_from_archives(
    pool: &Pool<Sqlite>,
    bucket_keys: &HashSet<(i64, i64)>,
    bucket_seconds: i64,
    archive_dir: &Path,
    max_rows: Option<usize>,
) -> Result<AccountWindowArchiveBucketRows> {
    if bucket_keys.is_empty()
        || bucket_seconds <= 0
        || !sqlite_table_exists(pool, "archive_batches").await?
    {
        return Ok(AccountWindowArchiveBucketRows::default());
    }

    let first_bucket_epoch = bucket_keys
        .iter()
        .map(|(_, bucket_epoch)| *bucket_epoch)
        .min()
        .expect("non-empty bucket keys have a first epoch");
    let last_bucket_end_epoch = bucket_keys
        .iter()
        .map(|(_, bucket_epoch)| bucket_epoch.saturating_add(bucket_seconds))
        .max()
        .expect("non-empty bucket keys have a last epoch");
    let start_at = format_naive(
        Utc.timestamp_opt(first_bucket_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid archive usage bucket start: {first_bucket_epoch}"))?
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let end_at = format_naive(
        Utc.timestamp_opt(last_bucket_end_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid archive usage bucket end: {last_bucket_end_epoch}"))?
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let archive_files = sqlx::query_as::<_, ArchiveBatchFileRow>(
        r#"
        SELECT id, file_path, coverage_start_at, coverage_end_at
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
          AND status = ?1
          AND (coverage_end_at IS NULL OR coverage_end_at >= ?2)
          AND (coverage_start_at IS NULL OR coverage_start_at <= ?3)
        ORDER BY month_key DESC, day_key DESC, part_key DESC, created_at DESC, id DESC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&start_at)
    .bind(&end_at)
    .fetch_all(pool)
    .await?;
    let archive_files = archive_files
        .into_iter()
        .filter(|archive_file| {
            archive_file_intersects_usage_bucket_keys(archive_file, bucket_keys, bucket_seconds)
        })
        .collect::<Vec<_>>();

    let mut rows = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut covered_bucket_keys = HashSet::new();
    let mut scanned_row_count = 0_usize;
    for archive_file in archive_files {
        let archive_path = resolve_archive_batch_path(archive_dir, &archive_file.file_path);
        if !archive_path.exists() {
            warn!(
                file_path = %archive_path.display(),
                "skipping missing invocation archive batch while repairing account window usage"
            );
            continue;
        }
        let temp_path = PathBuf::from(format!(
            "{}.{}.sqlite",
            archive_path.display(),
            retention_temp_suffix()
        ));
        if temp_path.exists() {
            let _ = fs::remove_file(&temp_path);
        }
        let temp_cleanup = TempSqliteCleanup(temp_path.clone());
        inflate_gzip_sqlite_file(&archive_path, &temp_path)?;
        let archive_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&sqlite_url_for_path(&temp_path))
            .await
            .with_context(|| format!("failed to open archive batch {}", archive_path.display()))?;
        let archive_rows = load_window_actual_usage_rows_for_detail_exact_bucket_keys_from_pool(
            &archive_pool,
            bucket_keys,
            bucket_seconds,
            max_rows.map(|max_rows| max_rows.saturating_sub(scanned_row_count).saturating_add(1)),
        )
        .await?;
        if max_rows
            .is_some_and(|max_rows| scanned_row_count.saturating_add(archive_rows.len()) > max_rows)
        {
            archive_pool.close().await;
            drop(temp_cleanup);
            return Ok(AccountWindowArchiveBucketRows {
                rows,
                covered_bucket_count: covered_bucket_keys.len(),
                row_limit_exceeded: true,
            });
        }
        scanned_row_count = scanned_row_count.saturating_add(archive_rows.len());
        // A manifest is only coverage evidence after the file has inflated and its bounded
        // query completed. This also makes an empty account bucket an explicit, verified zero.
        covered_bucket_keys.extend(archive_file_covered_usage_bucket_keys(
            &archive_file,
            bucket_keys,
            bucket_seconds,
        ));
        for row in archive_rows {
            if seen_ids.insert(row.id) {
                rows.push(row);
            }
        }
        archive_pool.close().await;
        drop(temp_cleanup);
    }
    Ok(AccountWindowArchiveBucketRows {
        rows,
        covered_bucket_count: covered_bucket_keys.len(),
        row_limit_exceeded: false,
    })
}

fn archive_file_covered_usage_bucket_keys(
    archive_file: &ArchiveBatchFileRow,
    bucket_keys: &HashSet<(i64, i64)>,
    bucket_seconds: i64,
) -> HashSet<(i64, i64)> {
    bucket_keys
        .iter()
        .filter(|(_, bucket_epoch)| {
            let bucket_start = Utc
                .timestamp_opt(*bucket_epoch, 0)
                .single()
                .map(|value| format_naive(value.with_timezone(&Shanghai).naive_local()));
            let bucket_end = Utc
                .timestamp_opt(bucket_epoch.saturating_add(bucket_seconds), 0)
                .single()
                .map(|value| format_naive(value.with_timezone(&Shanghai).naive_local()));
            bucket_start
                .zip(bucket_end)
                .is_some_and(|(bucket_start, bucket_end)| {
                    archive_file
                        .coverage_start_at
                        .as_deref()
                        .is_some_and(|start| start <= bucket_start.as_str())
                        && archive_file
                            .coverage_end_at
                            .as_deref()
                            .is_some_and(|end| end >= bucket_end.as_str())
                })
        })
        .copied()
        .collect()
}

fn archive_file_intersects_usage_bucket_keys(
    archive_file: &ArchiveBatchFileRow,
    bucket_keys: &HashSet<(i64, i64)>,
    bucket_seconds: i64,
) -> bool {
    bucket_keys.iter().any(|(_, bucket_epoch)| {
        let Some(bucket_start) = Utc
            .timestamp_opt(*bucket_epoch, 0)
            .single()
            .map(|value| format_naive(value.with_timezone(&Shanghai).naive_local()))
        else {
            return false;
        };
        let Some(bucket_end) = Utc
            .timestamp_opt(bucket_epoch.saturating_add(bucket_seconds), 0)
            .single()
            .map(|value| format_naive(value.with_timezone(&Shanghai).naive_local()))
        else {
            return false;
        };
        archive_file
            .coverage_end_at
            .as_deref()
            .is_none_or(|end| end >= bucket_start.as_str())
            && archive_file
                .coverage_start_at
                .as_deref()
                .is_none_or(|start| start <= bucket_end.as_str())
    })
}

pub(crate) async fn load_replayed_invocation_archive_covered_hour_keys(
    pool: &Pool<Sqlite>,
    bucket_keys: &HashSet<(i64, i64)>,
    archive_dir: &Path,
) -> Result<HashSet<(i64, i64)>> {
    if bucket_keys.is_empty()
        || !sqlite_table_exists(pool, "archive_batches").await?
        || !sqlite_table_exists(pool, "hourly_rollup_archive_replay").await?
    {
        return Ok(HashSet::new());
    }
    let first_bucket_epoch = bucket_keys
        .iter()
        .map(|(_, bucket_epoch)| *bucket_epoch)
        .min()
        .expect("non-empty bucket keys have a first epoch");
    let last_bucket_end_epoch = bucket_keys
        .iter()
        .map(|(_, bucket_epoch)| bucket_epoch.saturating_add(3_600))
        .max()
        .expect("non-empty bucket keys have a last epoch");
    let start_at = format_naive(
        Utc.timestamp_opt(first_bucket_epoch, 0)
            .single()
            .ok_or_else(|| {
                anyhow!("invalid replayed archive coverage start: {first_bucket_epoch}")
            })?
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let end_at = format_naive(
        Utc.timestamp_opt(last_bucket_end_epoch, 0)
            .single()
            .ok_or_else(|| {
                anyhow!("invalid replayed archive coverage end: {last_bucket_end_epoch}")
            })?
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    #[derive(Debug, FromRow)]
    struct ReplayedArchiveFile {
        id: i64,
        file_path: String,
        coverage_start_at: Option<String>,
        coverage_end_at: Option<String>,
        archive_sha256: String,
    }

    let archive_files = sqlx::query_as::<_, ReplayedArchiveFile>(
        r#"
        SELECT batches.id, batches.file_path, batches.coverage_start_at, batches.coverage_end_at,
               batches.sha256 AS archive_sha256
        FROM archive_batches AS batches
        INNER JOIN hourly_rollup_archive_replay AS replay
          ON replay.dataset = batches.dataset
         AND replay.file_path = batches.file_path
         AND replay.archive_sha256 = batches.sha256
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status = ?1
          AND batches.coverage_start_at IS NOT NULL
          AND batches.coverage_end_at IS NOT NULL
          AND batches.coverage_end_at >= ?2
          AND batches.coverage_start_at <= ?3
          AND replay.target = ?4
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(&start_at)
    .bind(&end_at)
    .bind(crate::maintenance::HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE)
    .fetch_all(pool)
    .await?;
    let mut verified_archive_files = Vec::with_capacity(archive_files.len());
    for archive_file in archive_files {
        let archive_path = resolve_archive_batch_path(archive_dir, &archive_file.file_path);
        let expected_sha256 = archive_file.archive_sha256.clone();
        let actual_sha256 =
            tokio::task::spawn_blocking(move || crate::maintenance::sha256_hex_file(&archive_path))
                .await;
        match actual_sha256 {
            Ok(Ok(actual_sha256)) if actual_sha256 == expected_sha256 => {
                verified_archive_files.push(ArchiveBatchFileRow {
                    id: archive_file.id,
                    file_path: archive_file.file_path,
                    coverage_start_at: archive_file.coverage_start_at,
                    coverage_end_at: archive_file.coverage_end_at,
                });
            }
            Ok(Ok(_)) | Ok(Err(_)) | Err(_) => {
                warn!(
                    file_path = %archive_file.file_path,
                    "archive replay marker cannot prove account window coverage because the current file hash is unavailable or changed"
                );
            }
        }
    }
    Ok(bucket_keys
        .iter()
        .filter(|(_, bucket_epoch)| {
            let bucket_start = Utc
                .timestamp_opt(*bucket_epoch, 0)
                .single()
                .map(|value| format_naive(value.with_timezone(&Shanghai).naive_local()));
            let bucket_end = Utc
                .timestamp_opt(bucket_epoch.saturating_add(3_600), 0)
                .single()
                .map(|value| format_naive(value.with_timezone(&Shanghai).naive_local()));
            bucket_start
                .zip(bucket_end)
                .is_some_and(|(bucket_start, bucket_end)| {
                    verified_archive_files.iter().any(|archive_file| {
                        archive_file
                            .coverage_start_at
                            .as_deref()
                            .is_some_and(|start| start <= bucket_start.as_str())
                            && archive_file
                                .coverage_end_at
                                .as_deref()
                                .is_some_and(|end| end >= bucket_end.as_str())
                    })
                })
        })
        .copied()
        .collect())
}

#[derive(Debug, Default)]
pub(crate) struct AccountWindowArchiveBucketRows {
    pub(crate) rows: Vec<AccountWindowUsageRow>,
    pub(crate) covered_bucket_count: usize,
    pub(crate) row_limit_exceeded: bool,
}

async fn load_window_actual_usage_rows_for_detail_exact_bucket_keys_from_pool(
    pool: &Pool<Sqlite>,
    bucket_keys: &HashSet<(i64, i64)>,
    bucket_seconds: i64,
    limit: Option<usize>,
) -> Result<Vec<AccountWindowUsageRow>> {
    if bucket_keys.is_empty() || bucket_seconds <= 0 {
        return Ok(Vec::new());
    }
    let upstream_account_id_sql = detail_exact_upstream_account_id_sql(pool).await?;
    let mut sorted_bucket_keys = bucket_keys.iter().copied().collect::<Vec<_>>();
    sorted_bucket_keys.sort_unstable();
    let mut query = QueryBuilder::<Sqlite>::new("SELECT id, occurred_at, ");
    query
        .push(upstream_account_id_sql)
        .push(
            " AS upstream_account_id, input_tokens, output_tokens, cache_input_tokens, total_tokens, cost \
             FROM codex_invocations WHERE ",
        );
    query.push("(");
    for (index, (account_id, bucket_epoch)) in sorted_bucket_keys.iter().enumerate() {
        if index > 0 {
            query.push(" OR ");
        }
        let bucket_start = Utc
            .timestamp_opt(*bucket_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid archive usage bucket epoch: {bucket_epoch}"))?;
        let bucket_end = Utc
            .timestamp_opt(bucket_epoch.saturating_add(bucket_seconds), 0)
            .single()
            .ok_or_else(|| anyhow!("invalid archive usage bucket end: {bucket_epoch}"))?;
        query
            .push("(")
            .push(upstream_account_id_sql)
            .push(" = ")
            .push_bind(account_id)
            .push(" AND occurred_at >= ")
            .push_bind(format_naive(
                bucket_start.with_timezone(&Shanghai).naive_local(),
            ))
            .push(" AND occurred_at < ")
            .push_bind(format_naive(
                bucket_end.with_timezone(&Shanghai).naive_local(),
            ))
            .push(")");
    }
    query.push(") ORDER BY occurred_at ASC, id ASC");
    if let Some(limit) = limit {
        query.push(" LIMIT ").push_bind(limit as i64);
    }
    query
        .build_query_as::<AccountWindowUsageRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) fn resolve_archive_batch_path(archive_dir: &Path, file_path: &str) -> PathBuf {
    let path = PathBuf::from(file_path);
    if path.is_absolute() || path.starts_with(archive_dir) {
        path
    } else {
        archive_dir.join(path)
    }
}

pub(crate) fn collect_account_window_partial_bucket_epochs(
    plans: &HashMap<i64, AccountWindowUsagePlan>,
) -> Result<HashSet<i64>> {
    let mut bucket_epochs = HashSet::new();
    for range in plans
        .values()
        .flat_map(|plan| [plan.primary.as_ref(), plan.secondary.as_ref()])
        .flatten()
    {
        let start_bucket_epoch = invocation_bucket_start_epoch(&range.start_at)?;
        if range.full_hour_start_epoch != Some(start_bucket_epoch) {
            bucket_epochs.insert(start_bucket_epoch);
        }
        bucket_epochs.insert(invocation_bucket_start_epoch(&range.end_at)?);
    }
    Ok(bucket_epochs)
}

/// A bucket can be complete for one account window and partial for another.
/// Keep the coverage identity account-scoped so a rollup never suppresses the
/// exact boundary tail required by a sibling range.
pub(crate) fn collect_account_window_partial_hour_keys(
    plans: &HashMap<i64, AccountWindowUsagePlan>,
) -> Result<HashSet<(i64, i64)>> {
    let mut keys = HashSet::new();
    for (account_id, plan) in plans {
        for range in [plan.primary.as_ref(), plan.secondary.as_ref()]
            .into_iter()
            .flatten()
        {
            let start_bucket_epoch = invocation_bucket_start_epoch(&range.start_at)?;
            if range.full_hour_start_epoch != Some(start_bucket_epoch) {
                keys.insert((*account_id, start_bucket_epoch));
            }

            let end_bucket_epoch = invocation_bucket_start_epoch(&range.end_at)?;
            if range.end_at_epoch.rem_euclid(3_600) != 0 {
                keys.insert((*account_id, end_bucket_epoch));
            }
        }
    }
    Ok(keys)
}

pub(crate) fn collect_account_window_partial_minute_bucket_epochs(
    plans: &HashMap<i64, AccountWindowUsagePlan>,
) -> Result<HashSet<i64>> {
    let mut bucket_epochs = HashSet::new();
    for range in plans
        .values()
        .flat_map(|plan| [plan.primary.as_ref(), plan.secondary.as_ref()])
        .flatten()
    {
        let start_bucket_epoch = invocation_bucket_start_epoch_for_seconds(&range.start_at, 60)?;
        let end_bucket_epoch = invocation_bucket_start_epoch_for_seconds(&range.end_at, 60)?;
        if start_bucket_epoch < end_bucket_epoch && start_bucket_epoch != range.start_at_epoch {
            bucket_epochs.insert(start_bucket_epoch);
        }
        if range.end_at_epoch.rem_euclid(60) != 0 {
            bucket_epochs.insert(end_bucket_epoch);
        }
    }
    Ok(bucket_epochs)
}

pub(crate) fn collect_account_window_partial_minute_keys(
    plans: &HashMap<i64, AccountWindowUsagePlan>,
) -> Result<HashSet<(i64, i64)>> {
    let mut keys = HashSet::new();
    for (account_id, plan) in plans {
        for range in [plan.primary.as_ref(), plan.secondary.as_ref()]
            .into_iter()
            .flatten()
        {
            let start_bucket_epoch =
                invocation_bucket_start_epoch_for_seconds(&range.start_at, 60)?;
            let end_bucket_epoch = invocation_bucket_start_epoch_for_seconds(&range.end_at, 60)?;
            if start_bucket_epoch < end_bucket_epoch && start_bucket_epoch != range.start_at_epoch {
                keys.insert((*account_id, start_bucket_epoch));
            }
            if range.end_at_epoch.rem_euclid(60) != 0 {
                keys.insert((*account_id, end_bucket_epoch));
            }
        }
    }
    Ok(keys)
}

pub(crate) fn collect_account_window_full_hour_ranges(
    plans: &HashMap<i64, AccountWindowUsagePlan>,
) -> Vec<(i64, i64, i64)> {
    collect_account_window_full_bucket_ranges(plans, |range| {
        range.full_hour_start_epoch.zip(range.full_hour_end_epoch)
    })
}

pub(crate) fn collect_account_window_exact_ranges(
    plans: &HashMap<i64, AccountWindowUsagePlan>,
) -> Vec<(i64, String, String)> {
    let mut ranges = plans
        .iter()
        .flat_map(|(account_id, plan)| {
            [plan.primary.as_ref(), plan.secondary.as_ref()]
                .into_iter()
                .flatten()
                .map(move |range| (*account_id, range.start_at.clone(), range.end_at.clone()))
        })
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    ranges.dedup();
    ranges
}

pub(crate) fn collect_account_window_full_minute_ranges(
    plans: &HashMap<i64, AccountWindowUsagePlan>,
) -> Vec<(i64, i64, i64)> {
    collect_account_window_full_bucket_ranges(plans, |range| {
        range
            .full_minute_start_epoch
            .zip(range.full_minute_end_epoch)
    })
}

fn collect_account_window_full_bucket_ranges<F>(
    plans: &HashMap<i64, AccountWindowUsagePlan>,
    bounds: F,
) -> Vec<(i64, i64, i64)>
where
    F: Fn(&AccountWindowUsageRange) -> Option<(i64, i64)>,
{
    let mut ranges = plans
        .iter()
        .flat_map(|(account_id, plan)| {
            [plan.primary.as_ref(), plan.secondary.as_ref()]
                .into_iter()
                .flatten()
                .filter_map(|range| {
                    bounds(range).and_then(|(start_epoch, end_epoch)| {
                        (start_epoch < end_epoch).then_some((*account_id, start_epoch, end_epoch))
                    })
                })
        })
        .collect::<Vec<_>>();
    ranges.sort_unstable();
    ranges.dedup();
    ranges
}

pub(crate) fn collect_account_window_full_hour_bounds(
    plans: &HashMap<i64, AccountWindowUsagePlan>,
) -> Option<(i64, i64)> {
    let mut minimum_start_epoch: Option<i64> = None;
    let mut maximum_end_epoch: Option<i64> = None;

    for range in plans
        .values()
        .flat_map(|plan| [plan.primary.as_ref(), plan.secondary.as_ref()])
        .flatten()
    {
        let (Some(full_hour_start_epoch), Some(full_hour_end_epoch)) =
            (range.full_hour_start_epoch, range.full_hour_end_epoch)
        else {
            continue;
        };
        minimum_start_epoch = Some(
            minimum_start_epoch
                .map(|value| value.min(full_hour_start_epoch))
                .unwrap_or(full_hour_start_epoch),
        );
        maximum_end_epoch = Some(
            maximum_end_epoch
                .map(|value| value.max(full_hour_end_epoch))
                .unwrap_or(full_hour_end_epoch),
        );
    }

    Some((minimum_start_epoch?, maximum_end_epoch?))
}

pub(crate) fn collect_account_window_full_minute_bounds(
    plans: &HashMap<i64, AccountWindowUsagePlan>,
) -> Option<(i64, i64)> {
    let mut minimum_start_epoch: Option<i64> = None;
    let mut maximum_end_epoch: Option<i64> = None;

    for plan in plans.values() {
        for range in [plan.primary.as_ref(), plan.secondary.as_ref()]
            .into_iter()
            .flatten()
        {
            let Some(start_epoch) = range.full_minute_start_epoch else {
                continue;
            };
            let Some(end_epoch) = range.full_minute_end_epoch else {
                continue;
            };
            minimum_start_epoch = Some(
                minimum_start_epoch
                    .map(|value| value.min(start_epoch))
                    .unwrap_or(start_epoch),
            );
            maximum_end_epoch = Some(
                maximum_end_epoch
                    .map(|value| value.max(end_epoch))
                    .unwrap_or(end_epoch),
            );
        }
    }

    Some((minimum_start_epoch?, maximum_end_epoch?))
}

pub(crate) fn collect_account_window_missing_full_hour_bucket_epochs(
    plans: &HashMap<i64, AccountWindowUsagePlan>,
    covered_hourly_keys: &HashSet<(i64, i64)>,
) -> HashSet<i64> {
    let mut missing_bucket_epochs = HashSet::new();

    for (account_id, plan) in plans {
        for range in [plan.primary.as_ref(), plan.secondary.as_ref()]
            .into_iter()
            .flatten()
        {
            let (Some(start_epoch), Some(end_epoch)) =
                (range.full_hour_start_epoch, range.full_hour_end_epoch)
            else {
                continue;
            };
            let mut bucket_epoch = start_epoch;
            while bucket_epoch < end_epoch {
                if !covered_hourly_keys.contains(&(*account_id, bucket_epoch)) {
                    missing_bucket_epochs.insert(bucket_epoch);
                }
                bucket_epoch = bucket_epoch.saturating_add(3_600);
            }
        }
    }

    missing_bucket_epochs
}

pub(crate) fn collect_account_window_missing_full_hour_keys(
    plans: &HashMap<i64, AccountWindowUsagePlan>,
    covered_hourly_keys: &HashSet<(i64, i64)>,
) -> HashSet<(i64, i64)> {
    let mut missing_keys = HashSet::new();
    for (account_id, plan) in plans {
        for range in [plan.primary.as_ref(), plan.secondary.as_ref()]
            .into_iter()
            .flatten()
        {
            let (Some(start_epoch), Some(end_epoch)) =
                (range.full_hour_start_epoch, range.full_hour_end_epoch)
            else {
                continue;
            };
            let mut bucket_epoch = start_epoch;
            while bucket_epoch < end_epoch {
                let key = (*account_id, bucket_epoch);
                if !covered_hourly_keys.contains(&key) {
                    missing_keys.insert(key);
                }
                bucket_epoch = bucket_epoch.saturating_add(3_600);
            }
        }
    }
    missing_keys
}

pub(crate) fn fold_account_window_usage_rows(
    rows: Vec<AccountWindowUsageRow>,
    plans: &HashMap<i64, AccountWindowUsagePlan>,
) -> HashMap<i64, AccountWindowUsageSummary> {
    let mut usage = plans
        .keys()
        .copied()
        .map(|account_id| (account_id, AccountWindowUsageSummary::default()))
        .collect::<HashMap<_, _>>();

    for row in rows {
        let Some(plan) = plans.get(&row.upstream_account_id) else {
            continue;
        };
        let entry = usage.entry(row.upstream_account_id).or_default();
        if plan.primary.as_ref().is_some_and(|range| {
            row.occurred_at.as_str() >= range.start_at.as_str()
                && row.occurred_at.as_str() < range.end_at.as_str()
        }) {
            entry.primary.add_row(&row);
        }
        if plan.secondary.as_ref().is_some_and(|range| {
            row.occurred_at.as_str() >= range.start_at.as_str()
                && row.occurred_at.as_str() < range.end_at.as_str()
        }) {
            entry.secondary.add_row(&row);
        }
    }

    usage
}

pub(crate) fn fold_account_window_usage_hourly_rows(
    usage: &mut HashMap<i64, AccountWindowUsageSummary>,
    rows: &[AccountWindowUsageHourlyRow],
    plans: &HashMap<i64, AccountWindowUsagePlan>,
) {
    for row in rows {
        let Some(plan) = plans.get(&row.upstream_account_id) else {
            continue;
        };
        let entry = usage.entry(row.upstream_account_id).or_default();
        if plan.primary.as_ref().is_some_and(|range| {
            range
                .full_hour_start_epoch
                .zip(range.full_hour_end_epoch)
                .is_some_and(|(start_epoch, end_epoch)| {
                    row.bucket_start_epoch >= start_epoch && row.bucket_start_epoch < end_epoch
                })
        }) {
            entry.primary.add_hourly_row(row);
        }
        if plan.secondary.as_ref().is_some_and(|range| {
            range
                .full_hour_start_epoch
                .zip(range.full_hour_end_epoch)
                .is_some_and(|(start_epoch, end_epoch)| {
                    row.bucket_start_epoch >= start_epoch && row.bucket_start_epoch < end_epoch
                })
        }) {
            entry.secondary.add_hourly_row(row);
        }
    }
}

pub(crate) fn fold_account_window_usage_minute_rows(
    usage: &mut HashMap<i64, AccountWindowUsageSummary>,
    rows: &[AccountWindowUsageMinuteRow],
    plans: &HashMap<i64, AccountWindowUsagePlan>,
) {
    for row in rows {
        let Some(plan) = plans.get(&row.upstream_account_id) else {
            continue;
        };
        let row_start_at = Utc
            .timestamp_opt(row.bucket_start_epoch, 0)
            .single()
            .map(|value| format_naive(value.with_timezone(&Shanghai).naive_local()));
        let Some(row_start_at) = row_start_at else {
            continue;
        };
        let entry = usage.entry(row.upstream_account_id).or_default();
        if plan.primary.as_ref().is_some_and(|range| {
            row_start_at.as_str() >= range.start_at.as_str()
                && row_start_at.as_str() < range.end_at.as_str()
        }) {
            entry.primary.add_minute_row(row);
        }
        if plan.secondary.as_ref().is_some_and(|range| {
            row_start_at.as_str() >= range.start_at.as_str()
                && row_start_at.as_str() < range.end_at.as_str()
        }) {
            entry.secondary.add_minute_row(row);
        }
    }
}

pub(crate) fn collect_account_window_hourly_coverage_keys(
    rows: &[AccountWindowUsageHourlyRow],
) -> HashSet<(i64, i64)> {
    rows.iter()
        .map(|row| (row.upstream_account_id, row.bucket_start_epoch))
        .collect()
}

pub(crate) fn filter_account_window_usage_rows_for_exact_fallback(
    rows: Vec<AccountWindowUsageRow>,
    partial_minute_bucket_epochs: &HashSet<i64>,
    include_partial_minute_rows: bool,
    partial_bucket_epochs: &HashSet<i64>,
    missing_full_hour_bucket_epochs: &HashSet<i64>,
    covered_hourly_keys: &HashSet<(i64, i64)>,
    covered_minute_keys: &HashSet<(i64, i64)>,
) -> Result<Vec<AccountWindowUsageRow>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let mut filtered_rows = Vec::with_capacity(rows.len());
    for row in rows {
        let minute_bucket_epoch = invocation_bucket_start_epoch_for_seconds(&row.occurred_at, 60)?;
        if covered_minute_keys.contains(&(row.upstream_account_id, minute_bucket_epoch)) {
            continue;
        }
        if partial_minute_bucket_epochs.contains(&minute_bucket_epoch)
            && !include_partial_minute_rows
        {
            continue;
        }
        let bucket_epoch = invocation_bucket_start_epoch(&row.occurred_at)?;
        let include = if partial_minute_bucket_epochs.contains(&minute_bucket_epoch) {
            true
        } else if partial_bucket_epochs.contains(&bucket_epoch) {
            !covered_hourly_keys.contains(&(row.upstream_account_id, bucket_epoch))
        } else {
            missing_full_hour_bucket_epochs.contains(&bucket_epoch)
        };
        if include {
            filtered_rows.push(row);
        }
    }
    Ok(filtered_rows)
}

pub(crate) fn apply_window_actual_usage_to_summaries(
    items: &mut [UpstreamAccountSummary],
    usage: &HashMap<i64, AccountWindowUsageSummary>,
) {
    for item in items {
        let account_usage = usage.get(&item.id).copied().unwrap_or_default();
        if let Some(window) = item.primary_window.as_mut() {
            window.actual_usage = Some(account_usage.primary.into_snapshot());
        }
        if let Some(window) = item.secondary_window.as_mut() {
            window.actual_usage = Some(account_usage.secondary.into_snapshot());
        }
    }
}

pub(crate) async fn load_account_active_conversation_count_map(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
    now: DateTime<Utc>,
) -> Result<HashMap<i64, i64>> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let active_cutoff =
        format_utc_iso(now - ChronoDuration::minutes(POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES));
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            account_id,
            COUNT(*) AS active_conversation_count
        FROM (
            SELECT DISTINCT account_id, sticky_key
            FROM (
                SELECT account_id, sticky_key
                FROM pool_sticky_routes
                WHERE last_seen_at >=
        "#,
    );
    query.push_bind(&active_cutoff).push(
        r#"
                UNION ALL
                SELECT account_id, sticky_key
                FROM pool_sticky_model_routes
                WHERE last_seen_at >=
        "#,
    );
    query.push_bind(&active_cutoff).push(
        r#"
            )
        )
        WHERE account_id IN (
        "#,
    );
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    let rows = query
        .push(") GROUP BY account_id")
        .build_query_as::<AccountActiveConversationCountRow>()
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| (row.account_id, row.active_conversation_count))
        .collect())
}

pub(crate) fn build_compact_support_state(row: &UpstreamAccountRow) -> CompactSupportState {
    let status = row
        .compact_support_status
        .as_deref()
        .map(str::trim)
        .filter(|value| {
            matches!(
                *value,
                COMPACT_SUPPORT_STATUS_UNKNOWN
                    | COMPACT_SUPPORT_STATUS_SUPPORTED
                    | COMPACT_SUPPORT_STATUS_UNSUPPORTED
            )
        })
        .unwrap_or(COMPACT_SUPPORT_STATUS_UNKNOWN)
        .to_string();
    CompactSupportState {
        status,
        observed_at: row.compact_support_observed_at.clone(),
        reason: row.compact_support_reason.clone(),
    }
}

pub(crate) fn build_capability_state(
    observed_value: Option<&str>,
    observed_at: Option<&String>,
    reason: Option<&String>,
    override_value: Option<&str>,
) -> UpstreamCapabilityState {
    let observed = decode_capability_support(observed_value);
    let override_value = decode_capability_override(override_value);
    UpstreamCapabilityState {
        observed,
        override_value,
        effective: effective_capability_support(observed, override_value),
        observed_at: observed_at.cloned(),
        reason: reason.cloned(),
    }
}

pub(crate) fn build_action_event_from_row(
    row: &UpstreamAccountActionEventRow,
) -> UpstreamAccountActionEvent {
    UpstreamAccountActionEvent {
        id: row.id,
        occurred_at: row.occurred_at.clone(),
        action: row.action.clone(),
        source: row.source.clone(),
        account_display_name: row.account_display_name.clone(),
        account_group_name: row.account_group_name.clone(),
        forward_proxy_key: row.forward_proxy_key.clone(),
        forward_proxy_display_name: row.forward_proxy_display_name.clone(),
        forward_proxy_egress_ip: row.forward_proxy_egress_ip.clone(),
        result: row.result.clone(),
        result_description: row.result_description.clone(),
        reason_code: row.reason_code.clone(),
        reason_message: row.reason_message.clone(),
        http_status: row.http_status.and_then(|value| u16::try_from(value).ok()),
        failure_kind: row.failure_kind.clone(),
        invoke_id: row.invoke_id.clone(),
        attempt_id: row.attempt_public_id.clone(),
        sticky_key: row.sticky_key.clone(),
        model: row.model.clone(),
        model_route_state_before: row.model_route_state_before.clone(),
        model_route_state_after: row.model_route_state_after.clone(),
        model_route_priority_before: row.model_route_priority_before.clone(),
        model_route_priority_after: row.model_route_priority_after.clone(),
        model_route_failure_count: row.model_route_failure_count,
        model_route_cooldown_until: row.model_route_cooldown_until.clone(),
        blocked_binding: parse_blocked_binding_json(row.blocked_binding_json.as_deref()),
        created_at: row.created_at.clone(),
    }
}

pub(crate) async fn load_account_last_activity_map(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
) -> Result<HashMap<i64, String>> {
    if account_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id AS account_id, last_activity_at FROM pool_upstream_accounts WHERE last_activity_at IS NOT NULL AND id IN (",
    );
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    query.push(")");

    let rows = query
        .build_query_as::<AccountLastActivityRow>()
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| (row.account_id, row.last_activity_at))
        .collect())
}

pub(crate) async fn backfill_upstream_account_last_activity_from_live_invocations(
    pool: &Pool<Sqlite>,
) -> Result<u64> {
    if !sqlite_table_exists(pool, "codex_invocations")
        .await
        .context("failed to inspect codex_invocations existence")?
    {
        return Ok(0);
    }

    let updated = sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET last_activity_at = (
                SELECT MAX(occurred_at)
                FROM codex_invocations
                WHERE CASE
                    WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER)
                END = pool_upstream_accounts.id
            ),
            last_activity_live_backfill_completed = 1
        WHERE COALESCE(deleted_at, '') = ''
          AND last_activity_at IS NULL
          AND last_activity_live_backfill_completed = 0
        "#,
    )
    .execute(pool)
    .await
    .context("failed to backfill pool_upstream_accounts.last_activity_at from live invocations")?;
    Ok(updated.rows_affected())
}

pub(crate) async fn group_has_accounts(pool: &Pool<Sqlite>, group_name: &str) -> Result<bool> {
    let mut conn = pool.acquire().await?;
    group_has_accounts_conn(&mut conn, group_name).await
}

pub(crate) async fn group_account_count_conn(
    conn: &mut SqliteConnection,
    group_name: &str,
) -> Result<i64> {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_accounts
        WHERE COALESCE(deleted_at, '') = '' AND group_name = ?1
        "#,
    )
    .bind(group_name)
    .fetch_one(conn)
    .await
    .map_err(Into::into)
}

pub(crate) async fn group_has_accounts_conn(
    conn: &mut SqliteConnection,
    group_name: &str,
) -> Result<bool> {
    Ok(group_account_count_conn(conn, group_name).await? > 0)
}

pub(crate) fn normalize_bound_proxy_keys(bound_proxy_keys: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    bound_proxy_keys
        .into_iter()
        .filter_map(|value| normalize_optional_text(Some(value)))
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

pub(crate) fn decode_group_bound_proxy_keys_json(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .map(normalize_bound_proxy_keys)
        .unwrap_or_default()
}

pub(crate) fn decode_group_node_shunt_enabled(raw: i64) -> bool {
    raw != 0
}

pub(crate) fn decode_group_single_account_rotation_enabled(raw: i64) -> bool {
    raw != 0
}

pub(crate) fn decode_group_requested_flag(raw: i64) -> bool {
    raw != 0
}

pub(crate) fn decode_group_upstream_429_retry_enabled(raw: i64) -> bool {
    raw != 0
}

pub(crate) fn normalize_group_upstream_429_max_retries(value: u8) -> u8 {
    value.min(MAX_PROXY_UPSTREAM_429_MAX_RETRIES)
}

pub(crate) fn normalize_enabled_group_upstream_429_max_retries(value: u8) -> u8 {
    normalize_group_upstream_429_max_retries(value).max(1)
}

pub(crate) fn normalize_group_upstream_429_retry_metadata(
    upstream_429_retry_enabled: bool,
    upstream_429_max_retries: u8,
) -> u8 {
    if upstream_429_retry_enabled {
        normalize_enabled_group_upstream_429_max_retries(upstream_429_max_retries)
    } else {
        0
    }
}

pub(crate) fn decode_group_upstream_429_max_retries(raw: i64) -> u8 {
    normalize_group_upstream_429_max_retries(raw.max(0) as u8)
}

pub(crate) fn encode_group_bound_proxy_keys_json(bound_proxy_keys: &[String]) -> Result<String> {
    serde_json::to_string(bound_proxy_keys).context("failed to encode group bound proxy keys")
}

pub(crate) fn group_node_shunt_unassigned_error_message() -> &'static str {
    UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED_MESSAGE
}

pub(crate) fn group_node_shunt_unassigned_error() -> anyhow::Error {
    anyhow!(group_node_shunt_unassigned_error_message())
}

pub(crate) fn is_group_node_shunt_unassigned_message(message: &str) -> bool {
    message
        .trim()
        .contains(UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED_MESSAGE)
}

pub(crate) fn missing_request_group_error_message() -> String {
    "groupName is required for upstream accounts".to_string()
}

pub(crate) fn missing_account_group_error_message() -> String {
    "upstream account is not assigned to a group; assign it to a group with at least one bound forward proxy node".to_string()
}

pub(crate) fn missing_group_bound_proxy_error_message(group_name: &str) -> String {
    format!(
        "upstream account group \"{group_name}\" has no bound forward proxy nodes; bind at least one proxy node to the group"
    )
}

pub(crate) fn missing_selectable_group_bound_proxy_error_message(group_name: &str) -> String {
    format!("upstream account group \"{group_name}\" has no selectable bound forward proxy nodes")
}

pub(crate) fn build_requested_group_metadata_changes(
    note: Option<String>,
    note_was_requested: bool,
    bound_proxy_keys: Option<Vec<String>>,
    bound_proxy_keys_was_requested: bool,
    concurrency_limit: i64,
    concurrency_limit_was_requested: bool,
    node_shunt_enabled: Option<bool>,
    node_shunt_enabled_was_requested: bool,
    single_account_rotation_enabled: Option<bool>,
    single_account_rotation_enabled_was_requested: bool,
) -> RequestedGroupMetadataChanges {
    RequestedGroupMetadataChanges {
        note: normalize_optional_text(note),
        note_was_requested,
        bound_proxy_keys: if bound_proxy_keys_was_requested {
            normalize_bound_proxy_keys(bound_proxy_keys.unwrap_or_default())
        } else {
            Vec::new()
        },
        bound_proxy_keys_was_requested,
        concurrency_limit,
        concurrency_limit_was_requested,
        node_shunt_enabled: node_shunt_enabled.unwrap_or(false),
        node_shunt_enabled_was_requested,
        single_account_rotation_enabled: single_account_rotation_enabled.unwrap_or(false),
        single_account_rotation_enabled_was_requested,
    }
}

pub(crate) fn required_account_forward_proxy_scope(
    group_name: Option<&str>,
    bound_proxy_keys: Vec<String>,
) -> Result<ForwardProxyRouteScope> {
    let normalized_group_name = group_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!(missing_account_group_error_message()))?;
    let normalized_bound_proxy_keys = normalize_bound_proxy_keys(bound_proxy_keys);
    if normalized_bound_proxy_keys.is_empty() {
        bail!(missing_group_bound_proxy_error_message(
            &normalized_group_name
        ));
    }
    Ok(ForwardProxyRouteScope::BoundGroup {
        group_name: normalized_group_name,
        bound_proxy_keys: normalized_bound_proxy_keys,
    })
}

pub(crate) fn map_required_group_proxy_selection_error(
    scope: &ForwardProxyRouteScope,
    err: anyhow::Error,
) -> anyhow::Error {
    match scope {
        ForwardProxyRouteScope::BoundGroup { group_name, .. }
            if err
                .to_string()
                .contains("bound forward proxy group has no selectable nodes") =>
        {
            anyhow!(missing_selectable_group_bound_proxy_error_message(
                group_name
            ))
        }
        _ => err,
    }
}

pub(crate) async fn ensure_required_group_proxy_scope_selectable(
    state: &AppState,
    scope: &ForwardProxyRouteScope,
) -> Result<()> {
    select_forward_proxy_for_scope(state, scope)
        .await
        .map(|_| ())
        .map_err(|err| map_required_group_proxy_selection_error(scope, err))
}

pub(crate) async fn resolve_required_group_proxy_binding_for_write(
    state: &AppState,
    group_name: Option<String>,
    requested_bound_proxy_keys: Option<Vec<String>>,
    requested_node_shunt_enabled: Option<bool>,
) -> Result<ResolvedRequiredGroupProxyBinding, (StatusCode, String)> {
    let group_name = normalize_optional_text(group_name).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            missing_request_group_error_message(),
        )
    })?;
    let existing_metadata = load_group_metadata(&state.pool, Some(&group_name))
        .await
        .map_err(internal_error_tuple)?;
    let bound_proxy_keys = if let Some(requested_bound_proxy_keys) = requested_bound_proxy_keys {
        normalize_bound_proxy_keys(requested_bound_proxy_keys)
    } else {
        existing_metadata.bound_proxy_keys.clone()
    };
    let bound_proxy_keys = canonicalize_forward_proxy_bound_keys(state, &bound_proxy_keys)
        .await
        .map_err(internal_error_tuple)?;
    let node_shunt_enabled =
        requested_node_shunt_enabled.unwrap_or(existing_metadata.node_shunt_enabled);
    if bound_proxy_keys.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            missing_group_bound_proxy_error_message(&group_name),
        ));
    }
    if !node_shunt_enabled {
        let scope =
            required_account_forward_proxy_scope(Some(&group_name), bound_proxy_keys.clone())
                .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
        ensure_required_group_proxy_scope_selectable(state, &scope)
            .await
            .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    }
    Ok(ResolvedRequiredGroupProxyBinding {
        group_name,
        bound_proxy_keys,
        node_shunt_enabled,
    })
}

pub(crate) async fn load_group_metadata_conn(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    group_name: &str,
) -> Result<Option<UpstreamAccountGroupMetadata>> {
    sqlx::query_as::<_, (String, Option<String>, i64, i64, i64, i64, i64)>(
        r#"
        SELECT
            note,
            bound_proxy_keys_json,
            node_shunt_enabled,
            single_account_rotation_enabled,
            upstream_429_retry_enabled,
            upstream_429_max_retries,
            concurrency_limit
        FROM pool_upstream_account_group_notes
        WHERE group_name = ?1
        "#,
    )
    .bind(group_name)
    .fetch_optional(executor)
    .await
    .map(|row| {
        row.map(
            |(
                note,
                bound_proxy_keys_json,
                node_shunt_enabled,
                single_account_rotation_enabled,
                upstream_429_retry_enabled,
                upstream_429_max_retries,
                concurrency_limit,
            )| {
                let node_shunt_enabled = decode_group_node_shunt_enabled(node_shunt_enabled);
                let single_account_rotation_enabled =
                    decode_group_single_account_rotation_enabled(single_account_rotation_enabled);
                let upstream_429_retry_enabled =
                    decode_group_upstream_429_retry_enabled(upstream_429_retry_enabled);
                let upstream_429_max_retries = normalize_group_upstream_429_retry_metadata(
                    upstream_429_retry_enabled,
                    decode_group_upstream_429_max_retries(upstream_429_max_retries),
                );
                UpstreamAccountGroupMetadata {
                    note: normalize_optional_text(Some(note)),
                    bound_proxy_keys: decode_group_bound_proxy_keys_json(
                        bound_proxy_keys_json.as_deref(),
                    ),
                    node_shunt_enabled,
                    single_account_rotation_enabled,
                    upstream_429_retry_enabled,
                    upstream_429_max_retries,
                    concurrency_limit,
                }
            },
        )
    })
    .map_err(Into::into)
}

pub(crate) async fn load_group_metadata(
    pool: &Pool<Sqlite>,
    group_name: Option<&str>,
) -> Result<UpstreamAccountGroupMetadata> {
    let Some(group_name) = group_name else {
        return Ok(UpstreamAccountGroupMetadata::default());
    };
    let mut conn = pool.acquire().await?;
    Ok(load_group_metadata_conn(&mut *conn, group_name)
        .await?
        .unwrap_or_default())
}

pub(crate) async fn load_required_account_forward_proxy_scope_from_group_metadata(
    state: &AppState,
    group_name: Option<&str>,
) -> Result<ForwardProxyRouteScope> {
    let normalized_group_name = group_name.map(str::trim).filter(|value| !value.is_empty());
    let Some(group_name) = normalized_group_name else {
        return Ok(ForwardProxyRouteScope::Automatic);
    };
    let bound_proxy_keys = load_group_metadata(&state.pool, Some(group_name))
        .await?
        .bound_proxy_keys;
    let bound_proxy_keys = canonicalize_forward_proxy_bound_keys(state, &bound_proxy_keys).await?;
    required_account_forward_proxy_scope(Some(group_name), bound_proxy_keys)
}

pub(crate) async fn save_group_metadata_record_conn(
    conn: &mut SqliteConnection,
    group_name: &str,
    metadata: UpstreamAccountGroupMetadata,
) -> Result<()> {
    let normalized_note = normalize_optional_text(metadata.note);
    let normalized_bound_proxy_keys = normalize_bound_proxy_keys(metadata.bound_proxy_keys);
    let normalized_node_shunt_enabled = metadata.node_shunt_enabled;
    let normalized_single_account_rotation_enabled = metadata.single_account_rotation_enabled;
    let normalized_upstream_429_retry_enabled = metadata.upstream_429_retry_enabled;
    let normalized_upstream_429_max_retries = normalize_group_upstream_429_retry_metadata(
        normalized_upstream_429_retry_enabled,
        metadata.upstream_429_max_retries,
    );
    let normalized_concurrency_limit =
        normalize_concurrency_limit(Some(metadata.concurrency_limit), "concurrencyLimit")
            .map_err(|(status, message)| anyhow!("{status}: {message}"))?;
    let now_iso = format_utc_iso(Utc::now());
    let bound_proxy_keys_json = encode_group_bound_proxy_keys_json(&normalized_bound_proxy_keys)?;
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_group_notes (
            group_name,
            note,
            bound_proxy_keys_json,
            node_shunt_enabled,
            single_account_rotation_enabled,
            upstream_429_retry_enabled,
            upstream_429_max_retries,
            concurrency_limit,
            created_at,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
        ON CONFLICT(group_name) DO UPDATE SET
            note = excluded.note,
            bound_proxy_keys_json = excluded.bound_proxy_keys_json,
            node_shunt_enabled = excluded.node_shunt_enabled,
            single_account_rotation_enabled = excluded.single_account_rotation_enabled,
            upstream_429_retry_enabled = excluded.upstream_429_retry_enabled,
            upstream_429_max_retries = excluded.upstream_429_max_retries,
            concurrency_limit = excluded.concurrency_limit,
            policy_allow_cut_out = pool_upstream_account_group_notes.policy_allow_cut_out,
            policy_allow_cut_in = pool_upstream_account_group_notes.policy_allow_cut_in,
            policy_priority_tier = pool_upstream_account_group_notes.policy_priority_tier,
            policy_fast_mode_rewrite_mode = pool_upstream_account_group_notes.policy_fast_mode_rewrite_mode,
            policy_image_tool_rewrite_mode = pool_upstream_account_group_notes.policy_image_tool_rewrite_mode,
            policy_codex_imagegen_rewrite_mode = pool_upstream_account_group_notes.policy_codex_imagegen_rewrite_mode,
            policy_concurrency_limit = pool_upstream_account_group_notes.policy_concurrency_limit,
            policy_upstream_429_retry_enabled = pool_upstream_account_group_notes.policy_upstream_429_retry_enabled,
            policy_upstream_429_max_retries = pool_upstream_account_group_notes.policy_upstream_429_max_retries,
            policy_available_models_json = pool_upstream_account_group_notes.policy_available_models_json,
            policy_status_change_upstream_http_401 = pool_upstream_account_group_notes.policy_status_change_upstream_http_401,
            policy_status_change_upstream_http_402 = pool_upstream_account_group_notes.policy_status_change_upstream_http_402,
            policy_status_change_upstream_http_403 = pool_upstream_account_group_notes.policy_status_change_upstream_http_403,
            policy_status_change_reauth_required = pool_upstream_account_group_notes.policy_status_change_reauth_required,
            policy_status_change_upstream_http_429_rate_limit = pool_upstream_account_group_notes.policy_status_change_upstream_http_429_rate_limit,
            policy_status_change_upstream_http_429_quota_exhausted = pool_upstream_account_group_notes.policy_status_change_upstream_http_429_quota_exhausted,
            policy_status_change_usage_snapshot_exhausted = pool_upstream_account_group_notes.policy_status_change_usage_snapshot_exhausted,
            policy_status_change_quota_still_exhausted = pool_upstream_account_group_notes.policy_status_change_quota_still_exhausted,
            policy_status_change_transport_failure = pool_upstream_account_group_notes.policy_status_change_transport_failure,
            policy_status_change_upstream_server_overloaded = pool_upstream_account_group_notes.policy_status_change_upstream_server_overloaded,
            policy_status_change_upstream_http_5xx = pool_upstream_account_group_notes.policy_status_change_upstream_http_5xx,
            policy_responses_first_byte_timeout_secs = pool_upstream_account_group_notes.policy_responses_first_byte_timeout_secs,
            policy_compact_first_byte_timeout_secs = pool_upstream_account_group_notes.policy_compact_first_byte_timeout_secs,
            policy_image_first_byte_timeout_secs = pool_upstream_account_group_notes.policy_image_first_byte_timeout_secs,
            policy_responses_stream_timeout_secs = pool_upstream_account_group_notes.policy_responses_stream_timeout_secs,
            policy_compact_stream_timeout_secs = pool_upstream_account_group_notes.policy_compact_stream_timeout_secs,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(group_name)
    .bind(normalized_note.unwrap_or_default())
    .bind(bound_proxy_keys_json)
    .bind(if normalized_node_shunt_enabled {
        1_i64
    } else {
        0_i64
    })
    .bind(if normalized_single_account_rotation_enabled {
        1_i64
    } else {
        0_i64
    })
    .bind(if normalized_upstream_429_retry_enabled {
        1_i64
    } else {
        0_i64
    })
    .bind(i64::from(normalized_upstream_429_max_retries))
    .bind(normalized_concurrency_limit)
    .bind(now_iso)
    .execute(conn)
    .await?;
    Ok(())
}

#[allow(dead_code)]
pub(crate) async fn save_group_note_record(
    pool: &Pool<Sqlite>,
    group_name: &str,
    note: Option<String>,
) -> Result<()> {
    let mut conn = pool.acquire().await?;
    save_group_note_record_conn(&mut conn, group_name, note).await
}

pub(crate) async fn save_group_note_record_conn(
    conn: &mut SqliteConnection,
    group_name: &str,
    note: Option<String>,
) -> Result<()> {
    let mut metadata = load_group_metadata_conn(&mut *conn, group_name)
        .await?
        .unwrap_or_default();
    metadata.note = note;
    save_group_metadata_record_conn(conn, group_name, metadata).await
}

pub(crate) async fn save_requested_group_metadata_changes(
    conn: &mut SqliteConnection,
    group_name: Option<&str>,
    changes: &RequestedGroupMetadataChanges,
) -> Result<()> {
    if !changes.was_requested() {
        return Ok(());
    }
    let Some(group_name) = group_name else {
        return Ok(());
    };
    let mut metadata = load_group_metadata_conn(&mut *conn, group_name)
        .await?
        .unwrap_or_default();
    if changes.note_was_requested {
        metadata.note = changes.note.clone();
    }
    if changes.bound_proxy_keys_was_requested {
        metadata.bound_proxy_keys = changes.bound_proxy_keys.clone();
    }
    if changes.concurrency_limit_was_requested {
        metadata.concurrency_limit = changes.concurrency_limit;
    }
    if changes.node_shunt_enabled_was_requested {
        metadata.node_shunt_enabled = changes.node_shunt_enabled;
    }
    if changes.single_account_rotation_enabled_was_requested {
        metadata.single_account_rotation_enabled = changes.single_account_rotation_enabled;
    }
    save_group_metadata_record_conn(conn, group_name, metadata).await
}

pub(crate) async fn save_group_metadata_after_account_write(
    conn: &mut SqliteConnection,
    group_name: Option<&str>,
    changes: &RequestedGroupMetadataChanges,
    _target_group_already_had_current_account: bool,
) -> Result<()> {
    save_requested_group_metadata_changes(conn, group_name, changes).await
}

pub(crate) async fn cleanup_orphaned_group_metadata(
    conn: &mut SqliteConnection,
    group_name: Option<&str>,
) -> Result<()> {
    let _ = conn;
    let _ = group_name;
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub(crate) struct GroupNodeShuntSlots {
    pub(crate) valid_proxy_keys: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct UpstreamAccountNodeShuntAssignments {
    pub(crate) account_proxy_keys: HashMap<i64, String>,
    pub(crate) group_slots: HashMap<String, GroupNodeShuntSlots>,
    pub(crate) group_assigned_proxy_keys: HashMap<String, HashSet<String>>,
    pub(crate) eligible_account_ids: HashSet<i64>,
}

pub(crate) fn compare_node_shunt_reserved_candidates(
    lhs: &AccountRoutingCandidateRow,
    rhs: &AccountRoutingCandidateRow,
) -> std::cmp::Ordering {
    rhs.in_flight_reservations
        .cmp(&lhs.in_flight_reservations)
        .then_with(|| compare_routing_candidates(lhs, rhs))
}

pub(crate) async fn load_node_shunt_enabled_group_metadata_map(
    pool: &Pool<Sqlite>,
) -> Result<HashMap<String, UpstreamAccountGroupMetadata>> {
    let rows = sqlx::query_as::<_, (String, String, Option<String>, i64, i64, i64, i64, i64)>(
        r#"
        SELECT
            group_name,
            note,
            bound_proxy_keys_json,
            node_shunt_enabled,
            single_account_rotation_enabled,
            upstream_429_retry_enabled,
            upstream_429_max_retries,
            concurrency_limit
        FROM pool_upstream_account_group_notes
        WHERE node_shunt_enabled != 0
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut groups = HashMap::with_capacity(rows.len());
    for (
        group_name,
        note,
        bound_proxy_keys_json,
        node_shunt_enabled,
        single_account_rotation_enabled,
        upstream_429_retry_enabled,
        upstream_429_max_retries,
        concurrency_limit,
    ) in rows
    {
        let node_shunt_enabled = decode_group_node_shunt_enabled(node_shunt_enabled);
        let single_account_rotation_enabled =
            decode_group_single_account_rotation_enabled(single_account_rotation_enabled);
        let upstream_429_retry_enabled =
            decode_group_upstream_429_retry_enabled(upstream_429_retry_enabled);
        let upstream_429_max_retries = normalize_group_upstream_429_retry_metadata(
            upstream_429_retry_enabled,
            decode_group_upstream_429_max_retries(upstream_429_max_retries),
        );
        groups.insert(
            group_name,
            UpstreamAccountGroupMetadata {
                note: normalize_optional_text(Some(note)),
                bound_proxy_keys: decode_group_bound_proxy_keys_json(
                    bound_proxy_keys_json.as_deref(),
                ),
                node_shunt_enabled,
                single_account_rotation_enabled,
                upstream_429_retry_enabled,
                upstream_429_max_retries,
                concurrency_limit,
            },
        );
    }
    Ok(groups)
}

pub(crate) async fn load_upstream_account_rows_for_groups(
    pool: &Pool<Sqlite>,
    group_names: &[String],
) -> Result<Vec<UpstreamAccountRow>> {
    if group_names.is_empty() {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(format!(
        r#"
        SELECT {UPSTREAM_ACCOUNT_ROW_SELECT_COLUMNS}
        FROM pool_upstream_accounts
        WHERE COALESCE(deleted_at, '') = '' AND group_name IN (
        "#
    ));
    {
        let mut separated = query.separated(", ");
        for group_name in group_names {
            separated.push_bind(group_name);
        }
    }
    query.push(") ORDER BY id ASC");

    query
        .build_query_as::<UpstreamAccountRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) fn account_is_node_shunt_slot_eligible(
    row: &UpstreamAccountRow,
    snapshot_exhausted: bool,
    now: DateTime<Utc>,
) -> bool {
    if !is_routing_eligible_account(row) {
        return false;
    }
    let health_status = derive_upstream_account_health_status(
        &row.kind,
        row.enabled != 0,
        &row.status,
        row.last_error.as_deref(),
        row.last_error_at.as_deref(),
        row.last_route_failure_at.as_deref(),
        row.last_route_failure_kind.as_deref(),
        row.last_action_reason_code.as_deref(),
    );
    if health_status != UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL {
        return false;
    }
    let sync_state = derive_upstream_account_sync_state(row.enabled != 0, &row.status);
    if sync_state != UPSTREAM_ACCOUNT_SYNC_STATE_IDLE {
        return false;
    }
    let work_status = derive_upstream_account_work_status(
        row.enabled != 0,
        &row.status,
        health_status,
        sync_state,
        snapshot_exhausted,
        row.cooldown_until.as_deref(),
        row.last_error_at.as_deref(),
        row.last_route_failure_at.as_deref(),
        row.last_route_failure_kind.as_deref(),
        row.last_action_reason_code.as_deref(),
        row.temporary_route_failure_streak_started_at.as_deref(),
        row.last_selected_at.as_deref(),
        now,
    );
    matches!(
        work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_WORKING
            | UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED
            | UPSTREAM_ACCOUNT_WORK_STATUS_IDLE
    )
}

pub(crate) async fn build_upstream_account_node_shunt_assignments(
    state: &AppState,
) -> Result<UpstreamAccountNodeShuntAssignments> {
    let group_metadata_map = load_node_shunt_enabled_group_metadata_map(&state.pool).await?;
    if group_metadata_map.is_empty() {
        return Ok(UpstreamAccountNodeShuntAssignments::default());
    }

    let group_names = group_metadata_map.keys().cloned().collect::<Vec<_>>();
    let rows = load_upstream_account_rows_for_groups(&state.pool, &group_names).await?;
    let rows_by_id = rows
        .into_iter()
        .map(|row| (row.id, row))
        .collect::<HashMap<_, _>>();

    let mut assignments = UpstreamAccountNodeShuntAssignments::default();
    {
        let manager = state.forward_proxy.lock().await;
        for (group_name, metadata) in &group_metadata_map {
            assignments.group_slots.insert(
                group_name.clone(),
                GroupNodeShuntSlots {
                    valid_proxy_keys: manager
                        .selectable_bound_proxy_keys_in_order(&metadata.bound_proxy_keys),
                },
            );
        }
    }

    let now = Utc::now();
    let mut group_candidates = HashMap::<String, Vec<AccountRoutingCandidateRow>>::new();
    let reservation_snapshot = pool_routing_reservation_snapshot(state);
    let mut candidates = load_account_routing_candidates(&state.pool, &HashSet::new()).await?;
    for candidate in &mut candidates {
        candidate.in_flight_reservations = reservation_snapshot.count_for_account(candidate.id);
    }
    let candidate_effective_rules = load_effective_routing_rules_for_accounts(
        &state.pool,
        &candidates
            .iter()
            .map(|candidate| candidate.id)
            .collect::<Vec<_>>(),
    )
    .await?;
    for candidate in candidates {
        let Some(row) = rows_by_id.get(&candidate.id) else {
            continue;
        };
        let Some(group_name) = normalize_optional_text(row.group_name.clone()) else {
            continue;
        };
        if !group_metadata_map
            .get(&group_name)
            .is_some_and(|metadata| metadata.node_shunt_enabled)
        {
            continue;
        }
        let snapshot_exhausted = routing_candidate_snapshot_is_exhausted(&candidate);
        if !account_is_node_shunt_slot_eligible(row, snapshot_exhausted, now) {
            continue;
        }
        assignments.eligible_account_ids.insert(row.id);
        group_candidates
            .entry(group_name)
            .or_default()
            .push(candidate);
    }

    let mut reserved_candidates = group_candidates
        .iter()
        .flat_map(|(group_name, candidates)| {
            candidates
                .iter()
                .filter(|candidate| candidate.in_flight_reservations > 0)
                .cloned()
                .map(|candidate| (group_name.clone(), candidate))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    reserved_candidates.sort_by(|(lhs_group, lhs), (rhs_group, rhs)| {
        routing_priority_rank(candidate_effective_rules.get(&lhs.id))
            .cmp(&routing_priority_rank(
                candidate_effective_rules.get(&rhs.id),
            ))
            .then_with(|| compare_node_shunt_reserved_candidates(lhs, rhs))
            .then_with(|| lhs_group.cmp(rhs_group))
            .then_with(|| lhs.id.cmp(&rhs.id))
    });

    let mut fresh_candidates = group_candidates
        .iter()
        .flat_map(|(group_name, candidates)| {
            candidates
                .iter()
                .cloned()
                .map(|candidate| (group_name.clone(), candidate))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    fresh_candidates.sort_by(|(lhs_group, lhs), (rhs_group, rhs)| {
        routing_priority_rank(candidate_effective_rules.get(&lhs.id))
            .cmp(&routing_priority_rank(
                candidate_effective_rules.get(&rhs.id),
            ))
            .then_with(|| compare_routing_candidates(lhs, rhs))
            .then_with(|| lhs_group.cmp(rhs_group))
            .then_with(|| lhs.id.cmp(&rhs.id))
    });

    let mut globally_occupied_proxy_keys = HashSet::new();
    let mut assigned_account_ids = HashSet::new();
    for (group_name, candidate) in &reserved_candidates {
        let Some(valid_proxy_keys) = assignments
            .group_slots
            .get(group_name)
            .map(|slots| slots.valid_proxy_keys.clone())
        else {
            continue;
        };
        let reserved_proxy_keys = reservation_snapshot.pinned_proxy_keys_for_account(
            candidate.id,
            &valid_proxy_keys,
            &globally_occupied_proxy_keys,
        );
        let Some(proxy_key) = reserved_proxy_keys.first().cloned() else {
            continue;
        };
        if !assigned_account_ids.insert(candidate.id) {
            continue;
        }
        assignments
            .account_proxy_keys
            .insert(candidate.id, proxy_key.clone());
        for reserved_proxy_key in reserved_proxy_keys {
            globally_occupied_proxy_keys.insert(reserved_proxy_key.clone());
            assignments
                .group_assigned_proxy_keys
                .entry(group_name.clone())
                .or_default()
                .insert(reserved_proxy_key);
        }
    }
    for (group_name, slots) in &assignments.group_slots {
        let globally_reserved_proxy_keys =
            reservation_snapshot.reserved_proxy_keys_for_group(&slots.valid_proxy_keys);
        for proxy_key in globally_reserved_proxy_keys {
            globally_occupied_proxy_keys.insert(proxy_key.clone());
            assignments
                .group_assigned_proxy_keys
                .entry(group_name.clone())
                .or_default()
                .insert(proxy_key);
        }
    }
    for (group_name, candidate) in fresh_candidates {
        if !assigned_account_ids.insert(candidate.id) {
            continue;
        }
        let Some(valid_proxy_keys) = assignments
            .group_slots
            .get(&group_name)
            .map(|slots| slots.valid_proxy_keys.clone())
        else {
            continue;
        };
        let Some(proxy_key) = valid_proxy_keys
            .into_iter()
            .find(|proxy_key| !globally_occupied_proxy_keys.contains(proxy_key.as_str()))
        else {
            continue;
        };
        globally_occupied_proxy_keys.insert(proxy_key.clone());
        assignments
            .account_proxy_keys
            .insert(candidate.id, proxy_key.clone());
        assignments
            .group_assigned_proxy_keys
            .entry(group_name)
            .or_default()
            .insert(proxy_key);
    }

    Ok(assignments)
}

pub(crate) async fn prepare_pool_account_with_node_shunt_refresh(
    state: &AppState,
    row: &UpstreamAccountRow,
    effective_rule: &EffectiveRoutingRule,
    group_metadata: &UpstreamAccountGroupMetadata,
    node_shunt_assignments: &mut UpstreamAccountNodeShuntAssignments,
) -> Result<Option<PoolResolvedAccount>> {
    let mut prepared_account = prepare_pool_account(
        state,
        row,
        effective_rule,
        group_metadata.clone(),
        node_shunt_assignments,
        None,
    )
    .await;
    if group_metadata.node_shunt_enabled
        && prepared_account
            .as_ref()
            .err()
            .is_some_and(|err| is_group_node_shunt_unassigned_message(&err.to_string()))
    {
        *node_shunt_assignments = build_upstream_account_node_shunt_assignments(state).await?;
        prepared_account = prepare_pool_account(
            state,
            row,
            effective_rule,
            group_metadata.clone(),
            node_shunt_assignments,
            None,
        )
        .await;
    }
    if group_metadata.node_shunt_enabled && matches!(prepared_account, Ok(None)) {
        *node_shunt_assignments = build_upstream_account_node_shunt_assignments(state).await?;
    }
    prepared_account
}

pub(crate) fn resolve_account_forward_proxy_scope_from_assignments(
    account_id: i64,
    group_name: Option<&str>,
    group_metadata: &UpstreamAccountGroupMetadata,
    assignments: &UpstreamAccountNodeShuntAssignments,
) -> Result<ForwardProxyRouteScope> {
    if !group_metadata.node_shunt_enabled {
        return required_account_forward_proxy_scope(
            group_name,
            group_metadata.bound_proxy_keys.clone(),
        );
    }

    let normalized_group_name = group_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!(missing_account_group_error_message()))?;
    let Some(proxy_key) = assignments.account_proxy_keys.get(&account_id) else {
        bail!(group_node_shunt_unassigned_error_message());
    };
    if !assignments.group_slots.contains_key(normalized_group_name) {
        return Err(group_node_shunt_unassigned_error());
    }
    Ok(ForwardProxyRouteScope::pinned(proxy_key.clone()))
}

pub(crate) fn account_bound_forward_proxy_scope(
    row: &UpstreamAccountRow,
) -> Option<ForwardProxyRouteScope> {
    let bound_proxy_keys = row.bound_proxy_keys();
    (!bound_proxy_keys.is_empty()).then(|| {
        ForwardProxyRouteScope::bound_scope(format!("account:{}", row.id()), bound_proxy_keys)
    })
}

pub(crate) async fn resolve_account_forward_proxy_scope(
    state: &AppState,
    row: &UpstreamAccountRow,
    group_metadata: Option<UpstreamAccountGroupMetadata>,
) -> Result<ForwardProxyRouteScope> {
    let group_metadata = match group_metadata {
        Some(metadata) => metadata,
        None => load_group_metadata(&state.pool, row.group_name.as_deref()).await?,
    };
    if let Some(scope) = account_bound_forward_proxy_scope(row) {
        return Ok(scope);
    }
    if !group_metadata.node_shunt_enabled {
        return required_account_forward_proxy_scope(
            row.group_name.as_deref(),
            group_metadata.bound_proxy_keys,
        );
    }
    let assignments = build_upstream_account_node_shunt_assignments(state).await?;
    resolve_account_forward_proxy_scope_from_assignments(
        row.id,
        row.group_name.as_deref(),
        &group_metadata,
        &assignments,
    )
}

pub(crate) async fn resolve_account_forward_proxy_scope_for_sync(
    state: &AppState,
    row: &UpstreamAccountRow,
    group_metadata: Option<UpstreamAccountGroupMetadata>,
) -> Result<ForwardProxyRouteScope> {
    let group_metadata = match group_metadata {
        Some(metadata) => metadata,
        None => load_group_metadata(&state.pool, row.group_name.as_deref()).await?,
    };
    if let Some(scope) = account_bound_forward_proxy_scope(row) {
        return Ok(scope);
    }
    if !group_metadata.node_shunt_enabled {
        return required_account_forward_proxy_scope(
            row.group_name.as_deref(),
            group_metadata.bound_proxy_keys,
        );
    }

    let normalized_group_name = row
        .group_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!(missing_account_group_error_message()))?;
    let normalized_bound_proxy_keys =
        normalize_bound_proxy_keys(group_metadata.bound_proxy_keys.clone());
    if normalized_bound_proxy_keys.is_empty() {
        bail!(missing_group_bound_proxy_error_message(
            normalized_group_name
        ));
    }
    let valid_proxy_keys = {
        let manager = state.forward_proxy.lock().await;
        manager.selectable_bound_proxy_keys_in_order(&normalized_bound_proxy_keys)
    };
    if valid_proxy_keys.is_empty() {
        bail!(missing_selectable_group_bound_proxy_error_message(
            normalized_group_name
        ));
    }

    let reservation_snapshot = pool_routing_reservation_snapshot(state);
    if let Some(proxy_key) = reservation_snapshot
        .pinned_proxy_keys_for_account(row.id, &valid_proxy_keys, &HashSet::new())
        .into_iter()
        .next()
    {
        return Ok(ForwardProxyRouteScope::pinned(proxy_key));
    }

    let assignments = build_upstream_account_node_shunt_assignments(state).await?;
    if let Some(proxy_key) = assignments.account_proxy_keys.get(&row.id) {
        return Ok(ForwardProxyRouteScope::pinned(proxy_key.clone()));
    }

    required_account_forward_proxy_scope(Some(normalized_group_name), valid_proxy_keys)
}

pub(crate) async fn resolve_group_forward_proxy_scope_for_provisioning(
    state: &AppState,
    binding: &ResolvedRequiredGroupProxyBinding,
    assignments: Option<&UpstreamAccountNodeShuntAssignments>,
    provisioning_account: Option<&UpstreamAccountRow>,
    consumed_proxy_keys: &HashSet<String>,
) -> Result<ForwardProxyRouteScope> {
    if !binding.node_shunt_enabled {
        return required_account_forward_proxy_scope(
            Some(&binding.group_name),
            binding.bound_proxy_keys.clone(),
        );
    }

    let valid_proxy_keys = {
        let manager = state.forward_proxy.lock().await;
        manager.selectable_bound_proxy_keys_in_order(&binding.bound_proxy_keys)
    };
    let globally_occupied_proxy_keys = assignments
        .map(|value| {
            value
                .group_assigned_proxy_keys
                .values()
                .flat_map(|proxy_keys| proxy_keys.iter().cloned())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let reservation_snapshot = pool_routing_reservation_snapshot(state);

    if let (Some(value), Some(account)) = (assignments, provisioning_account)
        && normalize_optional_text(account.group_name.clone()).as_deref()
            == Some(binding.group_name.as_str())
    {
        if let Some(proxy_key) = value.account_proxy_keys.get(&account.id) {
            return Ok(ForwardProxyRouteScope::pinned(proxy_key.clone()));
        }
        if let Some(proxy_key) = reservation_snapshot
            .pinned_proxy_keys_for_account(
                account.id,
                &valid_proxy_keys,
                &globally_occupied_proxy_keys,
            )
            .into_iter()
            .find(|proxy_key| !consumed_proxy_keys.contains(proxy_key))
        {
            return Ok(ForwardProxyRouteScope::pinned(proxy_key));
        }
    }

    let reserved_proxy_keys = reservation_snapshot.reserved_proxy_keys_for_group(&valid_proxy_keys);
    for proxy_key in valid_proxy_keys {
        if globally_occupied_proxy_keys.contains(&proxy_key)
            || reserved_proxy_keys.contains(&proxy_key)
            || consumed_proxy_keys.contains(&proxy_key)
        {
            continue;
        }
        return Ok(ForwardProxyRouteScope::pinned(proxy_key));
    }

    Err(group_node_shunt_unassigned_error())
}

pub(crate) fn reserve_imported_oauth_node_shunt_scope(
    state: &AppState,
    source_id: &str,
    account_id: Option<i64>,
    scope: &ForwardProxyRouteScope,
) -> Result<Option<String>> {
    let ForwardProxyRouteScope::PinnedProxyKey(proxy_key) = scope else {
        return Ok(None);
    };
    let reservation_key = format!(
        "imported-oauth:{source_id}:{}",
        random_hex(8).map_err(|(_, message)| anyhow!(message))?
    );
    state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned")
        .insert(
            reservation_key.clone(),
            crate::PoolRoutingReservation {
                account_id: account_id.unwrap_or_default(),
                model: None,
                proxy_key: Some(proxy_key.clone()),
                created_at: Instant::now(),
            },
        );
    Ok(Some(reservation_key))
}

pub(crate) fn release_imported_oauth_node_shunt_scope(
    state: &AppState,
    reservation_key: Option<String>,
) {
    if let Some(reservation_key) = reservation_key {
        crate::release_pool_routing_reservation(state, &reservation_key);
    }
}

pub(crate) async fn load_login_session_by_login_id_with_executor(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    login_id: &str,
) -> Result<Option<OauthLoginSessionRow>> {
    sqlx::query_as::<_, OauthLoginSessionRow>(
        r#"
        SELECT
            login_id, account_id, display_name, email, group_name, group_bound_proxy_keys_json, group_node_shunt_enabled,
            group_node_shunt_enabled_requested, group_single_account_rotation_enabled,
            group_single_account_rotation_enabled_requested, is_mother, note, tag_ids_json, group_note,
            group_concurrency_limit,
            mailbox_session_id, generated_mailbox_address AS mailbox_address, state, pkce_verifier, redirect_uri, status, auth_url,
            error_message, pending_encrypted_credentials, pending_token_expires_at, pending_verified_email,
            pending_chatgpt_account_id, pending_chatgpt_user_id, pending_plan_type, pending_has_refresh_token,
            expires_at, consumed_at, created_at, updated_at
        FROM pool_oauth_login_sessions
        WHERE login_id = ?1
        LIMIT 1
        "#,
    )
    .bind(login_id)
    .fetch_optional(executor)
    .await
    .map_err(Into::into)
}

pub(crate) async fn load_login_session_by_login_id(
    pool: &Pool<Sqlite>,
    login_id: &str,
) -> Result<Option<OauthLoginSessionRow>> {
    load_login_session_by_login_id_with_executor(pool, login_id).await
}

pub(crate) async fn load_login_session_by_state(
    pool: &Pool<Sqlite>,
    state_value: &str,
) -> Result<Option<OauthLoginSessionRow>> {
    sqlx::query_as::<_, OauthLoginSessionRow>(
        r#"
        SELECT
            login_id, account_id, display_name, email, group_name, group_bound_proxy_keys_json, group_node_shunt_enabled,
            group_node_shunt_enabled_requested, group_single_account_rotation_enabled,
            group_single_account_rotation_enabled_requested, is_mother, note, tag_ids_json, group_note,
            group_concurrency_limit,
            mailbox_session_id, generated_mailbox_address AS mailbox_address, state, pkce_verifier, redirect_uri, status, auth_url,
            error_message, pending_encrypted_credentials, pending_token_expires_at, pending_verified_email,
            pending_chatgpt_account_id, pending_chatgpt_user_id, pending_plan_type, pending_has_refresh_token,
            expires_at, consumed_at, created_at, updated_at
        FROM pool_oauth_login_sessions
        WHERE state = ?1
        LIMIT 1
        "#,
    )
    .bind(state_value)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub(crate) async fn expire_pending_login_sessions(pool: &Pool<Sqlite>) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET status = ?1,
            updated_at = ?2,
            pending_encrypted_credentials = NULL,
            pending_token_expires_at = NULL,
            pending_verified_email = NULL,
            pending_chatgpt_account_id = NULL,
            pending_chatgpt_user_id = NULL,
            pending_plan_type = NULL,
            pending_has_refresh_token = NULL
        WHERE status IN (?3, ?4) AND expires_at < ?2
        "#,
    )
    .bind(LOGIN_SESSION_STATUS_EXPIRED)
    .bind(&now_iso)
    .bind(LOGIN_SESSION_STATUS_PENDING)
    .bind(LOGIN_SESSION_STATUS_NEEDS_IDENTITY_CONFIRMATION)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn load_oauth_mailbox_session(
    pool: &Pool<Sqlite>,
    session_id: &str,
) -> Result<Option<OauthMailboxSessionRow>> {
    sqlx::query_as::<_, OauthMailboxSessionRow>(
        r#"
        SELECT
            session_id, remote_email_id, email_address, email_domain, mailbox_source, latest_code_value,
            latest_code_source, latest_code_updated_at, invite_subject, invite_copy_value,
            invite_copy_label, invite_updated_at, invited, last_message_id, created_at, updated_at,
            expires_at
        FROM pool_oauth_mailbox_sessions
        WHERE session_id = ?1
        LIMIT 1
        "#,
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

pub(crate) async fn load_oauth_mailbox_sessions(
    pool: &Pool<Sqlite>,
    session_ids: &[String],
) -> Result<Vec<OauthMailboxSessionRow>> {
    if session_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            session_id, remote_email_id, email_address, email_domain, mailbox_source, latest_code_value,
            latest_code_source, latest_code_updated_at, invite_subject, invite_copy_value,
            invite_copy_label, invite_updated_at, invited, last_message_id, created_at, updated_at,
            expires_at
        FROM pool_oauth_mailbox_sessions
        WHERE session_id IN (
        "#,
    );
    let mut separated = builder.separated(", ");
    for session_id in session_ids {
        separated.push_bind(session_id);
    }
    separated.push_unseparated(")");
    builder.push(" ORDER BY created_at ASC");
    builder
        .build_query_as::<OauthMailboxSessionRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn delete_oauth_mailbox_session_with_executor(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    session_id: &str,
) -> Result<u64> {
    let affected = sqlx::query(
        r#"
        DELETE FROM pool_oauth_mailbox_sessions
        WHERE session_id = ?1
        "#,
    )
    .bind(session_id)
    .execute(executor)
    .await?
    .rows_affected();
    Ok(affected)
}

pub(crate) async fn cleanup_expired_oauth_mailbox_sessions(state: &AppState) -> Result<()> {
    let kaisoumail_meta = state.config.upstream_accounts_kaisoumail.as_ref();
    let now_iso = format_utc_iso(Utc::now());
    let expired_rows = sqlx::query_as::<_, OauthMailboxSessionRow>(
        r#"
        SELECT
            session_id, remote_email_id, email_address, email_domain, mailbox_source, latest_code_value,
            latest_code_source, latest_code_updated_at, invite_subject, invite_copy_value,
            invite_copy_label, invite_updated_at, invited, last_message_id, created_at, updated_at,
            expires_at
        FROM pool_oauth_mailbox_sessions
        WHERE expires_at <= ?1
        ORDER BY expires_at ASC
        "#,
    )
    .bind(&now_iso)
    .fetch_all(&state.pool)
    .await?;

    for row in expired_rows {
        if expired_mailbox_session_requires_remote_delete(&row)
            && let Some(config) = kaisoumail_meta
            && let Err(err) =
                kaisoumail_delete_mailbox(&state.http_clients.shared, config, &row.remote_email_id)
                    .await
        {
            debug!(
                mailbox_session_id = %row.session_id,
                remote_email_id = %row.remote_email_id,
                error = %err,
                "failed to delete expired kaisoumail mailbox"
            );
        }
        delete_oauth_mailbox_session_with_executor(&state.pool, &row.session_id).await?;
    }
    Ok(())
}

pub(crate) async fn complete_login_session_with_executor(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    login_id: &str,
    account_id: i64,
    group_note_snapshot: Option<String>,
    group_concurrency_limit_snapshot: i64,
    previous_updated_at: &str,
    preserve_pending_updated_at: bool,
) -> Result<()> {
    let consumed_at = next_login_session_updated_at(Some(previous_updated_at));
    let completed_updated_at = if preserve_pending_updated_at {
        previous_updated_at.to_string()
    } else {
        consumed_at.clone()
    };
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET status = ?2,
            account_id = ?3,
            group_note = ?4,
            group_concurrency_limit = ?5,
            updated_at = ?6,
            consumed_at = ?7,
            pending_encrypted_credentials = NULL,
            pending_token_expires_at = NULL,
            pending_verified_email = NULL,
            pending_chatgpt_account_id = NULL,
            pending_chatgpt_user_id = NULL,
            pending_plan_type = NULL,
            pending_has_refresh_token = NULL
        WHERE login_id = ?1
        "#,
    )
    .bind(login_id)
    .bind(LOGIN_SESSION_STATUS_COMPLETED)
    .bind(account_id)
    .bind(group_note_snapshot)
    .bind(group_concurrency_limit_snapshot)
    .bind(&completed_updated_at)
    .bind(&consumed_at)
    .execute(executor)
    .await?;
    Ok(())
}

pub(crate) async fn load_group_metadata_snapshot_conn(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    group_name: Option<&str>,
    fallback_note: Option<&str>,
) -> Result<UpstreamAccountGroupMetadata> {
    load_group_metadata_snapshot_conn_with_limit(executor, group_name, fallback_note, 0).await
}

pub(crate) async fn load_group_metadata_snapshot_conn_with_limit(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    group_name: Option<&str>,
    fallback_note: Option<&str>,
    fallback_concurrency_limit: i64,
) -> Result<UpstreamAccountGroupMetadata> {
    let normalized_fallback_concurrency_limit =
        normalize_concurrency_limit(Some(fallback_concurrency_limit), "concurrencyLimit")
            .map_err(|(_, message)| anyhow!(message))?;
    let Some(group_name) = group_name else {
        return Ok(UpstreamAccountGroupMetadata {
            note: fallback_note.map(str::to_string),
            bound_proxy_keys: Vec::new(),
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: normalized_fallback_concurrency_limit,
        });
    };
    let metadata = load_group_metadata_conn(executor, group_name)
        .await?
        .unwrap_or_default();
    Ok(UpstreamAccountGroupMetadata {
        note: metadata
            .note
            .or_else(|| normalize_optional_text(fallback_note.map(str::to_string))),
        bound_proxy_keys: metadata.bound_proxy_keys,
        node_shunt_enabled: metadata.node_shunt_enabled,
        single_account_rotation_enabled: metadata.single_account_rotation_enabled,
        upstream_429_retry_enabled: metadata.upstream_429_retry_enabled,
        upstream_429_max_retries: metadata.upstream_429_max_retries,
        concurrency_limit: metadata.concurrency_limit,
    })
}

pub(crate) fn next_login_session_updated_at(previous_updated_at: Option<&str>) -> String {
    let mut next_updated_at =
        parse_rfc3339_utc(&Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
            .unwrap_or_else(Utc::now);
    if let Some(previous_updated_at) = previous_updated_at
        && let Some(previous_updated_at) = parse_rfc3339_utc(previous_updated_at)
        && next_updated_at <= previous_updated_at
    {
        next_updated_at = previous_updated_at + ChronoDuration::milliseconds(1);
    }
    next_updated_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(crate) async fn fail_login_session_with_executor(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    login_id: &str,
    error_message: &str,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET status = ?2,
            error_message = ?3,
            consumed_at = ?4,
            updated_at = ?4,
            pending_encrypted_credentials = NULL,
            pending_token_expires_at = NULL,
            pending_verified_email = NULL,
            pending_chatgpt_account_id = NULL,
            pending_chatgpt_user_id = NULL,
            pending_plan_type = NULL,
            pending_has_refresh_token = NULL
        WHERE login_id = ?1
        "#,
    )
    .bind(login_id)
    .bind(LOGIN_SESSION_STATUS_FAILED)
    .bind(error_message)
    .bind(&now_iso)
    .execute(executor)
    .await?;
    Ok(())
}

pub(crate) async fn fail_login_session(
    pool: &Pool<Sqlite>,
    login_id: &str,
    error_message: &str,
) -> Result<()> {
    fail_login_session_with_executor(pool, login_id, error_message).await
}

pub(crate) async fn mark_login_session_expired(pool: &Pool<Sqlite>, login_id: &str) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET status = ?2,
            updated_at = ?3
        WHERE login_id = ?1
        "#,
    )
    .bind(login_id)
    .bind(LOGIN_SESSION_STATUS_EXPIRED)
    .bind(&now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) fn login_session_to_response(row: &OauthLoginSessionRow) -> LoginSessionStatusResponse {
    LoginSessionStatusResponse {
        login_id: row.login_id.clone(),
        status: row.status.clone(),
        auth_url: if row.status == LOGIN_SESSION_STATUS_PENDING {
            Some(row.auth_url.clone())
        } else {
            None
        },
        redirect_uri: if row.status == LOGIN_SESSION_STATUS_PENDING {
            Some(row.redirect_uri.clone())
        } else {
            None
        },
        expires_at: row.expires_at.clone(),
        updated_at: row.updated_at.clone(),
        account_id: row.account_id,
        email: row.email.clone(),
        error: row.error_message.clone(),
        sync_applied: None,
        identity_confirmation: login_session_identity_confirmation_response(row),
    }
}

pub(crate) fn login_session_identity_confirmation_response(
    row: &OauthLoginSessionRow,
) -> Option<OauthIdentityConfirmationResponse> {
    if row.status != LOGIN_SESSION_STATUS_NEEDS_IDENTITY_CONFIRMATION {
        return None;
    }
    Some(OauthIdentityConfirmationResponse {
        current: OauthIdentitySummaryResponse {
            account_id: row.account_id,
            display_name: row.display_name.clone(),
            email: row.email.clone(),
            verified_email: None,
            chatgpt_account_id: None,
            chatgpt_user_id: None,
            plan_type: None,
        },
        incoming: OauthIdentitySummaryResponse {
            account_id: row.account_id,
            display_name: None,
            email: row.pending_verified_email.clone(),
            verified_email: row.pending_verified_email.clone(),
            chatgpt_account_id: row.pending_chatgpt_account_id.clone(),
            chatgpt_user_id: row.pending_chatgpt_user_id.clone(),
            plan_type: row.pending_plan_type.clone(),
        },
    })
}

pub(crate) fn login_session_to_response_with_sync_applied(
    row: &OauthLoginSessionRow,
    sync_applied: bool,
) -> LoginSessionStatusResponse {
    let mut response = login_session_to_response(row);
    response.sync_applied = Some(sync_applied);
    response
}

pub(crate) fn login_session_required_forward_proxy_scope(
    row: &OauthLoginSessionRow,
) -> Result<ForwardProxyRouteScope> {
    required_account_forward_proxy_scope(
        row.group_name.as_deref(),
        decode_group_bound_proxy_keys_json(row.group_bound_proxy_keys_json.as_deref()),
    )
}
