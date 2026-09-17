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

        for range in [primary, secondary].into_iter().flatten() {
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
    let window_anchor = resets_at.and_then(parse_rfc3339_utc).unwrap_or(now);
    Some(AccountWindowUsageRangeBounds {
        start_at: window_anchor - ChronoDuration::minutes(window_duration_mins),
        end_at: window_anchor.min(now),
    })
}

pub(crate) async fn load_window_actual_usage_rows_from_pool(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
    start_at: &str,
    end_at: &str,
    end_before: Option<&str>,
    min_id_exclusive: Option<i64>,
    max_id_inclusive: Option<i64>,
) -> Result<Vec<AccountWindowUsageRow>> {
    if account_ids.is_empty() {
        return Ok(Vec::new());
    }

    let upstream_account_id_sql = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            id,
            occurred_at,
        "#,
    );
    query
        .push(upstream_account_id_sql)
        .push(
            r#"
            AS upstream_account_id,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            total_tokens,
            cost
        FROM codex_invocations
        WHERE occurred_at >=
        "#,
        )
        .push_bind(start_at)
        .push(" AND occurred_at <= ")
        .push_bind(end_at)
        .push(" AND ")
        .push(upstream_account_id_sql)
        .push(" IS NOT NULL");

    if let Some(end_before) = end_before {
        query.push(" AND occurred_at < ").push_bind(end_before);
    }
    if let Some(min_id_exclusive) = min_id_exclusive {
        query.push(" AND id > ").push_bind(min_id_exclusive.max(0));
    }
    if let Some(max_id_inclusive) = max_id_inclusive {
        query.push(" AND id <= ").push_bind(max_id_inclusive.max(0));
    }

    query
        .push(" AND ")
        .push(upstream_account_id_sql)
        .push(" IN (");
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
    let upstream_account_id_sql = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            id,
            occurred_at,
        "#,
    );
    query.push(upstream_account_id_sql).push(
        r#"
            AS upstream_account_id,
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
    query
        .push(") AND ")
        .push(upstream_account_id_sql)
        .push(" IS NOT NULL AND ")
        .push(upstream_account_id_sql)
        .push(" IN (");
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

pub(crate) async fn load_window_actual_usage_hourly_rows_from_pool(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
    start_bucket_epoch: i64,
    end_bucket_epoch_exclusive: i64,
) -> Result<Vec<AccountWindowUsageHourlyRow>> {
    if account_ids.is_empty() || start_bucket_epoch >= end_bucket_epoch_exclusive {
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
        WHERE bucket_start_epoch >=
        "#,
    );
    query
        .push_bind(start_bucket_epoch)
        .push(" AND bucket_start_epoch < ")
        .push_bind(end_bucket_epoch_exclusive)
        .push(" AND upstream_account_id IN (");
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    query.push(") ORDER BY bucket_start_epoch ASC");

    query
        .build_query_as::<AccountWindowUsageHourlyRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn load_window_actual_usage_minute_rows_from_pool(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
    start_bucket_epoch: i64,
    end_bucket_epoch_exclusive: i64,
) -> Result<Vec<AccountWindowUsageMinuteRow>> {
    if account_ids.is_empty() || start_bucket_epoch >= end_bucket_epoch_exclusive {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            bucket_start_epoch,
            upstream_account_id,
            total_count AS request_count,
            total_tokens,
            total_cost,
            input_tokens,
            output_tokens,
            cache_input_tokens
        FROM upstream_account_stats_minute
        WHERE bucket_start_epoch >=
        "#,
    );
    query
        .push_bind(start_bucket_epoch)
        .push(" AND bucket_start_epoch < ")
        .push_bind(end_bucket_epoch_exclusive)
        .push(" AND upstream_account_id IN (");
    {
        let mut separated = query.separated(", ");
        for account_id in account_ids {
            separated.push_bind(account_id);
        }
    }
    query.push(") ORDER BY bucket_start_epoch ASC");

    query
        .build_query_as::<AccountWindowUsageMinuteRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn load_window_actual_usage_rows_from_archives(
    pool: &Pool<Sqlite>,
    account_ids: &[i64],
    start_at: &str,
    end_at: &str,
    archive_dir: &Path,
) -> Result<Vec<AccountWindowUsageRow>> {
    if account_ids.is_empty() || !sqlite_table_exists(pool, "archive_batches").await? {
        return Ok(Vec::new());
    }

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
    .bind(start_at)
    .bind(end_at)
    .fetch_all(pool)
    .await?;

    let mut rows = Vec::new();
    for archive_file in archive_files {
        let archive_path = resolve_archive_batch_path(archive_dir, &archive_file.file_path);
        if !archive_path.exists() {
            warn!(
                file_path = %archive_path.display(),
                "skipping missing invocation archive batch while calculating account window usage"
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
        rows.extend(
            load_window_actual_usage_rows_from_pool(
                &archive_pool,
                account_ids,
                start_at,
                end_at,
                None,
                None,
                None,
            )
            .await?,
        );
        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok(rows)
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
                && row.occurred_at.as_str() <= range.end_at.as_str()
        }) {
            entry.primary.add_row(&row);
        }
        if plan.secondary.as_ref().is_some_and(|range| {
            row.occurred_at.as_str() >= range.start_at.as_str()
                && row.occurred_at.as_str() <= range.end_at.as_str()
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
    partial_bucket_epochs: &HashSet<i64>,
    missing_full_hour_bucket_epochs: &HashSet<i64>,
    covered_hourly_keys: &HashSet<(i64, i64)>,
) -> Result<Vec<AccountWindowUsageRow>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let mut filtered_rows = Vec::with_capacity(rows.len());
    for row in rows {
        let minute_bucket_epoch = invocation_bucket_start_epoch_for_seconds(&row.occurred_at, 60)?;
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
