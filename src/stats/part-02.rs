pub(crate) async fn load_completed_invocation_archive_paths(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    load_completed_archive_paths_for_dataset(executor, HOURLY_ROLLUP_DATASET_INVOCATIONS).await
}

pub(crate) async fn load_completed_archive_paths_for_dataset(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    dataset: &str,
) -> Result<Vec<ArchiveBatchPathRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            file_path,
            month_key,
            coverage_start_at,
            coverage_end_at,
            historical_rollups_materialized_at,
            NULL AS needs_overall,
            NULL AS needs_failures
        FROM archive_batches
        WHERE dataset =
        "#,
    );
    query.push_bind(dataset).push(" AND status = ");
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    if dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS {
        query.push(" AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'");
    }
    query.push(" ORDER BY month_key ASC, created_at ASC, id ASC");
    query
        .build_query_as::<ArchiveBatchPathRow>()
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

pub(crate) async fn load_invocation_archives_missing_rollup_target(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    target: &str,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    load_invocation_archives_missing_rollup_target_with_limit(executor, target, range, None).await
}

pub(crate) async fn load_invocation_archives_missing_rollup_target_bounded(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    target: &str,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    limit: usize,
) -> Result<Vec<ArchiveBatchPathRow>> {
    load_invocation_archives_missing_rollup_target_with_limit(executor, target, range, Some(limit))
        .await
}

async fn load_invocation_archives_missing_rollup_target_with_limit(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    target: &str,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    limit: Option<usize>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            batches.file_path,
            batches.month_key,
            batches.coverage_start_at,
            batches.coverage_end_at,
            batches.historical_rollups_materialized_at,
            NULL AS needs_overall,
            NULL AS needs_failures
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status =
        "#,
    );
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    query.push(
        r#"
          AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
          AND NOT EXISTS(
            SELECT 1
            FROM hourly_rollup_archive_replay AS replay
            WHERE replay.target =
        "#,
    );
    query.push_bind(target);
    query.push(
        r#"
              AND replay.dataset = 'codex_invocations'
              AND replay.file_path = batches.file_path
              AND batches.sha256 IS NOT NULL
              AND TRIM(batches.sha256) <> ''
              AND replay.archive_sha256 = batches.sha256
          )
        "#,
    );

    if let Some((start, end)) = range {
        let start_bound = db_occurred_at_upper_bound(start);
        let end_bound = db_occurred_at_lower_bound(end);
        query.push(
            r#"
            AND (
                batches.coverage_start_at IS NULL
                OR batches.coverage_end_at IS NULL
                OR (
                    batches.coverage_end_at >=
            "#,
        );
        query.push_bind(start_bound).push(
            r#"
                    AND batches.coverage_start_at <
            "#,
        );
        query.push_bind(end_bound).push(
            r#"
                )
            )
            "#,
        );
    }

    query.push(" ORDER BY batches.month_key ASC, batches.created_at ASC, batches.id ASC");
    if let Some(limit) = limit {
        query
            .push(" LIMIT ")
            .push_bind((limit.saturating_add(1)) as i64);
    }
    query
        .build_query_as::<ArchiveBatchPathRow>()
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

pub(crate) fn account_archive_target_treats_materialized_batch_as_replayed(target: &str) -> bool {
    matches!(
        target,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE
            | HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY
            | HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_MINUTE
    )
}

pub(crate) async fn load_completed_invocation_archive_paths_in_range(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    load_completed_archive_paths_for_dataset_in_range_with_limit(
        executor,
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        range,
        None,
    )
    .await
}

pub(crate) async fn load_completed_invocation_archive_paths_in_range_bounded(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    limit: usize,
) -> Result<Vec<ArchiveBatchPathRow>> {
    load_completed_archive_paths_for_dataset_in_range_with_limit(
        executor,
        HOURLY_ROLLUP_DATASET_INVOCATIONS,
        range,
        Some(limit),
    )
    .await
}

pub(crate) async fn load_completed_archive_paths_for_dataset_in_range(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    dataset: &str,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    load_completed_archive_paths_for_dataset_in_range_with_limit(executor, dataset, range, None)
        .await
}

async fn load_completed_archive_paths_for_dataset_in_range_with_limit(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    dataset: &str,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    limit: Option<usize>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            file_path,
            month_key,
            coverage_start_at,
            coverage_end_at,
            historical_rollups_materialized_at,
            NULL AS needs_overall,
            NULL AS needs_failures
        FROM archive_batches
        WHERE dataset =
        "#,
    );
    query.push_bind(dataset).push(
        r#"
          AND status =
        "#,
    );
    query.push_bind(ARCHIVE_STATUS_COMPLETED);
    if dataset == HOURLY_ROLLUP_DATASET_INVOCATIONS {
        // Detail mirrors preserve payload observability while the canonical invocation remains
        // live. They are not Summary sources and must not consume archive admission capacity.
        query.push(" AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'");
    }

    if let Some((start, end)) = range {
        let start_bound = db_occurred_at_upper_bound(start);
        let end_bound = db_occurred_at_lower_bound(end);
        query.push(
            r#"
            AND (
                coverage_start_at IS NULL
                OR coverage_end_at IS NULL
                OR (
                    coverage_end_at >=
            "#,
        );
        query.push_bind(start_bound).push(
            r#"
                    AND coverage_start_at <
            "#,
        );
        query.push_bind(end_bound).push(
            r#"
                )
            )
            "#,
        );
    }

    query.push(" ORDER BY month_key ASC, created_at ASC, id ASC");
    if let Some(limit) = limit {
        query
            .push(" LIMIT ")
            .push_bind((limit.saturating_add(1)) as i64);
    }
    query
        .build_query_as::<ArchiveBatchPathRow>()
        .fetch_all(executor)
        .await
        .map_err(Into::into)
}

pub(crate) async fn load_completed_invocation_archives_in_range(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> Result<Vec<(String, Option<String>, Option<String>, Option<String>)>> {
    Ok(
        load_completed_invocation_archive_paths_in_range(executor, range)
            .await?
            .into_iter()
            .map(|row| {
                (
                    row.file_path,
                    row.coverage_start_at,
                    row.coverage_end_at,
                    row.historical_rollups_materialized_at,
                )
            })
            .collect(),
    )
}

pub(crate) async fn load_invocation_archives_missing_effective_rollup_target(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
    target: &str,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    let archive_rows =
        load_invocation_archives_missing_rollup_target(executor, target, range).await?;
    if !account_archive_target_treats_materialized_batch_as_replayed(target) {
        return Ok(archive_rows);
    }
    Ok(archive_rows
        .into_iter()
        .filter(|archive_row| archive_row.historical_rollups_materialized_at.is_none())
        .collect())
}

pub(crate) async fn load_invocation_archives_missing_summary_rollup_markers(
    executor: impl sqlx::Executor<'_, Database = Sqlite>,
) -> Result<Vec<ArchiveBatchPathRow>> {
    sqlx::query_as::<_, ArchiveBatchPathRow>(
        r#"
        SELECT
            batches.file_path,
            batches.month_key,
            batches.coverage_start_at,
            batches.coverage_end_at,
            batches.historical_rollups_materialized_at,
            CASE
                WHEN EXISTS(
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?2
                      AND replay.dataset = 'codex_invocations'
                      AND replay.file_path = batches.file_path
                ) THEN 0
                ELSE 1
            END AS needs_overall,
            CASE
                WHEN EXISTS(
                    SELECT 1
                    FROM hourly_rollup_archive_replay AS replay
                    WHERE replay.target = ?3
                      AND replay.dataset = 'codex_invocations'
                      AND replay.file_path = batches.file_path
                ) THEN 0
                ELSE 1
            END AS needs_failures
        FROM archive_batches AS batches
        WHERE batches.dataset = 'codex_invocations'
          AND batches.status = ?1
          AND COALESCE(batches.summary_source_kind, 'unknown') <> 'live_mirror'
          AND (
            NOT EXISTS(
                SELECT 1
                FROM hourly_rollup_archive_replay AS replay
                WHERE replay.target = ?2
                  AND replay.dataset = 'codex_invocations'
                  AND replay.file_path = batches.file_path
            )
            OR NOT EXISTS(
                SELECT 1
                FROM hourly_rollup_archive_replay AS replay
                WHERE replay.target = ?3
                  AND replay.dataset = 'codex_invocations'
                  AND replay.file_path = batches.file_path
            )
          )
        ORDER BY batches.month_key ASC, batches.created_at ASC, batches.id ASC
        "#,
    )
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATIONS)
    .bind(HOURLY_ROLLUP_TARGET_INVOCATION_FAILURES)
    .fetch_all(executor)
    .await
    .map_err(Into::into)
}
