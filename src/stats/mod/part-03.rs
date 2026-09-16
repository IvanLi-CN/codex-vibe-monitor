pub(crate) fn merge_invocation_hourly_rollup_delta(
    target: &mut InvocationHourlyRollupDelta,
    delta: &InvocationHourlyRollupDelta,
) -> Result<()> {
    target.total_count += delta.total_count;
    target.success_count += delta.success_count;
    target.failure_count += delta.failure_count;
    target.terminal_count += delta.terminal_count;
    target.terminal_tokens += delta.terminal_tokens;
    target.terminal_cost += delta.terminal_cost;
    target.total_tokens += delta.total_tokens;
    target.input_tokens += delta.input_tokens;
    target.output_tokens += delta.output_tokens;
    target.cache_input_tokens += delta.cache_input_tokens;
    target.reasoning_tokens += delta.reasoning_tokens;
    target.total_cost += delta.total_cost;
    target.non_success_cost += delta.non_success_cost;
    target.total_latency_sample_count += delta.total_latency_sample_count;
    target.total_latency_sum_ms += delta.total_latency_sum_ms;
    target.first_byte_sample_count += delta.first_byte_sample_count;
    target.first_byte_sum_ms += delta.first_byte_sum_ms;
    target.first_byte_max_ms = target.first_byte_max_ms.max(delta.first_byte_max_ms);
    if target.first_byte_histogram.is_empty() {
        target.first_byte_histogram = delta.first_byte_histogram.clone();
    } else if !delta.first_byte_histogram.is_empty() {
        merge_approx_histogram_into(
            &mut target.first_byte_histogram,
            &delta.first_byte_histogram,
        )?;
    }
    target.first_response_byte_total_sample_count += delta.first_response_byte_total_sample_count;
    target.first_response_byte_total_sum_ms += delta.first_response_byte_total_sum_ms;
    target.first_response_byte_total_max_ms = target
        .first_response_byte_total_max_ms
        .max(delta.first_response_byte_total_max_ms);
    if target.first_response_byte_total_histogram.is_empty() {
        target.first_response_byte_total_histogram =
            delta.first_response_byte_total_histogram.clone();
    } else if !delta.first_response_byte_total_histogram.is_empty() {
        merge_approx_histogram_into(
            &mut target.first_response_byte_total_histogram,
            &delta.first_response_byte_total_histogram,
        )?;
    }
    target.first_token_sample_count += delta.first_token_sample_count;
    target.first_token_sum_ms += delta.first_token_sum_ms;
    target.first_token_max_ms = target.first_token_max_ms.max(delta.first_token_max_ms);
    if target.first_token_histogram.is_empty() {
        target.first_token_histogram = delta.first_token_histogram.clone();
    } else if !delta.first_token_histogram.is_empty() {
        merge_approx_histogram_into(
            &mut target.first_token_histogram,
            &delta.first_token_histogram,
        )?;
    }
    Ok(())
}

pub(crate) fn merge_invocation_hourly_rollup_delta_map(
    target: &mut BTreeMap<(i64, String), InvocationHourlyRollupDelta>,
    source: &BTreeMap<(i64, String), InvocationHourlyRollupDelta>,
) -> Result<()> {
    for (key, delta) in source {
        let entry = target
            .entry(key.clone())
            .or_insert_with(|| InvocationHourlyRollupDelta {
                first_byte_histogram: empty_approx_histogram(),
                first_response_byte_total_histogram: empty_approx_histogram(),
                first_token_histogram: empty_approx_histogram(),
                ..InvocationHourlyRollupDelta::default()
            });
        merge_invocation_hourly_rollup_delta(entry, delta)?;
    }
    Ok(())
}

pub(crate) fn merge_proxy_perf_stage_hourly_delta(
    target: &mut ProxyPerfStageHourlyDelta,
    delta: &ProxyPerfStageHourlyDelta,
) -> Result<()> {
    target.sample_count += delta.sample_count;
    target.sum_ms += delta.sum_ms;
    target.max_ms = target.max_ms.max(delta.max_ms);
    if target.histogram.is_empty() {
        target.histogram = delta.histogram.clone();
    } else if !delta.histogram.is_empty() {
        merge_approx_histogram_into(&mut target.histogram, &delta.histogram)?;
    }
    Ok(())
}

pub(crate) fn merge_proxy_perf_stage_hourly_delta_map(
    target: &mut BTreeMap<(i64, String), ProxyPerfStageHourlyDelta>,
    source: &BTreeMap<(i64, String), ProxyPerfStageHourlyDelta>,
) -> Result<()> {
    for (key, delta) in source {
        let entry = target
            .entry(key.clone())
            .or_insert_with(|| ProxyPerfStageHourlyDelta {
                histogram: empty_approx_histogram(),
                ..ProxyPerfStageHourlyDelta::default()
            });
        merge_proxy_perf_stage_hourly_delta(entry, delta)?;
    }
    Ok(())
}

pub(crate) fn accumulate_failure_rollup_row_counts(
    counts: &mut HashMap<(i64, String, String, i64, String), usize>,
    rows: impl IntoIterator<Item = ArchivedInvocationFailureRow>,
) -> Result<()> {
    for row in rows {
        let Some(key) = archived_failure_rollup_key(&row)? else {
            continue;
        };
        *counts.entry(key).or_default() += 1;
    }
    Ok(())
}

pub(crate) async fn load_materialized_invocation_archive_hourly_rollup_deltas_for_bucket_sources(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    bucket_sources: &HashSet<(i64, String)>,
) -> Result<(
    BTreeMap<(i64, String), InvocationHourlyRollupDelta>,
    HashSet<i64>,
)> {
    if bucket_sources.is_empty() {
        return Ok((BTreeMap::new(), HashSet::new()));
    }

    let month_keys = shanghai_month_keys_for_bucket_starts(
        bucket_sources
            .iter()
            .map(|(bucket_start_epoch, _)| *bucket_start_epoch),
    );
    let archive_rows = load_replayed_invocation_archives_for_month_keys(
        pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        &month_keys,
    )
    .await?;
    let mut materialized = BTreeMap::<(i64, String), InvocationHourlyRollupDelta>::new();
    let mut unreadable_bucket_start_epochs = HashSet::new();

    for archive in archive_rows {
        let archive_row = materialized_archive_path_row(
            archive.file_path.clone(),
            archive.coverage_start_at.clone(),
            archive.coverage_end_at.clone(),
        );
        let Some((archive_pool, temp_cleanup)) =
            open_invocation_archive_batch_pool(&archive_row, "stats-summary").await?
        else {
            unreadable_bucket_start_epochs.extend(replayed_archive_bucket_start_epochs(&archive)?);
            continue;
        };
        let mut cursor_id = 0_i64;
        loop {
            let rows = load_invocation_hourly_source_rows_after_id(
                &archive_pool,
                cursor_id,
                source_scope,
                BACKFILL_BATCH_SIZE,
            )
            .await?;
            if rows.is_empty() {
                break;
            }
            cursor_id = rows.last().map(|row| row.id).unwrap_or(cursor_id);
            let filtered_rows = rows
                .into_iter()
                .filter(|row| invocation_hourly_source_record_matches_range(row, range))
                .filter_map(|row| {
                    summary_rollup_bucket_start_epoch(&row.occurred_at)
                        .ok()
                        .map(|bucket_start_epoch| ((bucket_start_epoch, row.source.clone()), row))
                })
                .filter(|(key, _)| bucket_sources.contains(key))
                .map(|(_, row)| row)
                .collect::<Vec<_>>();
            if filtered_rows.is_empty() {
                continue;
            }
            accumulate_invocation_hourly_overall_rollups(&mut materialized, &filtered_rows)?;
        }

        archive_pool.close().await;
        drop(temp_cleanup);
    }

    Ok((materialized, unreadable_bucket_start_epochs))
}

pub(crate) fn subtract_approx_histogram_counts(
    archive_histogram: &[i64],
    materialized_histogram: &[i64],
) -> ApproxHistogramCounts {
    let expected_len = archive_histogram.len().max(materialized_histogram.len());
    let mut delta = archive_histogram.to_vec();
    delta.resize(expected_len, 0);
    for (idx, slot) in delta.iter_mut().enumerate() {
        let materialized = materialized_histogram.get(idx).copied().unwrap_or_default();
        *slot = slot.saturating_sub(materialized).max(0);
    }
    delta
}

pub(crate) fn subtract_nonnegative_i64(archive_value: i64, materialized_value: i64) -> i64 {
    archive_value.saturating_sub(materialized_value).max(0)
}

pub(crate) fn subtract_nonnegative_f64(archive_value: f64, materialized_value: f64) -> f64 {
    (archive_value - materialized_value).max(0.0)
}

pub(crate) async fn load_materialized_invocation_rollup_record(
    pool: &Pool<Sqlite>,
    bucket_start_epoch: i64,
    source: &str,
) -> Result<Option<InvocationHourlyRollupRecord>> {
    let input_tokens_expr =
        if sqlite_table_has_column(pool, "invocation_rollup_hourly", "input_tokens").await? {
            "COALESCE(input_tokens, 0) AS input_tokens"
        } else {
            "0 AS input_tokens"
        };
    let output_tokens_expr =
        if sqlite_table_has_column(pool, "invocation_rollup_hourly", "output_tokens").await? {
            "COALESCE(output_tokens, 0) AS output_tokens"
        } else {
            "0 AS output_tokens"
        };
    let cache_input_tokens_expr =
        if sqlite_table_has_column(pool, "invocation_rollup_hourly", "cache_input_tokens").await? {
            "COALESCE(cache_input_tokens, 0) AS cache_input_tokens"
        } else {
            "0 AS cache_input_tokens"
        };
    let reasoning_tokens_expr =
        if sqlite_table_has_column(pool, "invocation_rollup_hourly", "reasoning_tokens").await? {
            "COALESCE(reasoning_tokens, 0) AS reasoning_tokens"
        } else {
            "0 AS reasoning_tokens"
        };
    let non_success_cost_expr =
        if sqlite_table_has_column(pool, "invocation_rollup_hourly", "non_success_cost").await? {
            "COALESCE(non_success_cost, 0.0) AS non_success_cost"
        } else {
            "0.0 AS non_success_cost"
        };
    let total_latency_sample_count_expr = if sqlite_table_has_column(
        pool,
        "invocation_rollup_hourly",
        "total_latency_sample_count",
    )
    .await?
    {
        "COALESCE(total_latency_sample_count, 0) AS total_latency_sample_count"
    } else {
        "0 AS total_latency_sample_count"
    };
    let total_latency_sum_ms_expr =
        if sqlite_table_has_column(pool, "invocation_rollup_hourly", "total_latency_sum_ms").await?
        {
            "COALESCE(total_latency_sum_ms, 0.0) AS total_latency_sum_ms"
        } else {
            "0.0 AS total_latency_sum_ms"
        };
    let has_first_token_rollup =
        sqlite_table_has_column(pool, "invocation_rollup_hourly", "first_token_sample_count")
            .await?;
    let first_token_sample_count_expr = if has_first_token_rollup {
        "COALESCE(first_token_sample_count, 0) AS first_token_sample_count"
    } else {
        "0 AS first_token_sample_count"
    };
    let first_token_sum_ms_expr = if has_first_token_rollup {
        "COALESCE(first_token_sum_ms, 0.0) AS first_token_sum_ms"
    } else {
        "0.0 AS first_token_sum_ms"
    };
    let first_token_max_ms_expr = if has_first_token_rollup {
        "COALESCE(first_token_max_ms, 0.0) AS first_token_max_ms"
    } else {
        "0.0 AS first_token_max_ms"
    };
    let first_token_histogram_expr = if has_first_token_rollup {
        "COALESCE(first_token_histogram, '[]') AS first_token_histogram"
    } else {
        "'[]' AS first_token_histogram"
    };
    let query = format!(
        r#"
        SELECT
            bucket_start_epoch,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            {input_tokens_expr},
            {output_tokens_expr},
            {cache_input_tokens_expr},
            {reasoning_tokens_expr},
            total_cost,
            {non_success_cost_expr},
            {total_latency_sample_count_expr},
            {total_latency_sum_ms_expr},
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram,
            first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms,
            first_response_byte_total_max_ms,
            first_response_byte_total_histogram,
            {first_token_sample_count_expr},
            {first_token_sum_ms_expr},
            {first_token_max_ms_expr},
            {first_token_histogram_expr}
        FROM invocation_rollup_hourly
        WHERE bucket_start_epoch = ?1
          AND source = ?2
        LIMIT 1
        "#,
    );
    sqlx::query_as::<_, InvocationHourlyRollupRecord>(&query)
        .bind(bucket_start_epoch)
        .bind(source)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub(crate) fn build_invocation_hourly_rollup_delta_record(
    bucket_start_epoch: i64,
    archive_delta: &InvocationHourlyRollupDelta,
    materialized_row: Option<&InvocationHourlyRollupRecord>,
) -> Result<Option<InvocationHourlyRollupRecord>> {
    let total_count = subtract_nonnegative_i64(
        archive_delta.total_count,
        materialized_row
            .map(|row| row.total_count.max(0))
            .unwrap_or(0),
    );
    let success_count = subtract_nonnegative_i64(
        archive_delta.success_count,
        materialized_row
            .map(|row| row.success_count.max(0))
            .unwrap_or(0),
    );
    let failure_count = subtract_nonnegative_i64(
        archive_delta.failure_count,
        materialized_row
            .map(|row| row.failure_count.max(0))
            .unwrap_or(0),
    );
    let total_tokens = subtract_nonnegative_i64(
        archive_delta.total_tokens,
        materialized_row
            .map(|row| row.total_tokens.max(0))
            .unwrap_or(0),
    );
    let input_tokens = subtract_nonnegative_i64(
        archive_delta.input_tokens,
        materialized_row
            .map(|row| row.input_tokens.max(0))
            .unwrap_or(0),
    );
    let output_tokens = subtract_nonnegative_i64(
        archive_delta.output_tokens,
        materialized_row
            .map(|row| row.output_tokens.max(0))
            .unwrap_or(0),
    );
    let cache_input_tokens = subtract_nonnegative_i64(
        archive_delta.cache_input_tokens,
        materialized_row
            .map(|row| row.cache_input_tokens.max(0))
            .unwrap_or(0),
    );
    let reasoning_tokens = subtract_nonnegative_i64(
        archive_delta.reasoning_tokens,
        materialized_row
            .map(|row| row.reasoning_tokens.max(0))
            .unwrap_or(0),
    );
    let total_cost = subtract_nonnegative_f64(
        archive_delta.total_cost,
        materialized_row.map(|row| row.total_cost).unwrap_or(0.0),
    );
    let non_success_cost = subtract_nonnegative_f64(
        archive_delta.non_success_cost,
        materialized_row
            .map(|row| row.non_success_cost)
            .unwrap_or(0.0),
    );
    let total_latency_sample_count = subtract_nonnegative_i64(
        archive_delta.total_latency_sample_count,
        materialized_row
            .map(|row| row.total_latency_sample_count.max(0))
            .unwrap_or(0),
    );
    let total_latency_sum_ms = subtract_nonnegative_f64(
        archive_delta.total_latency_sum_ms,
        materialized_row
            .map(|row| row.total_latency_sum_ms)
            .unwrap_or(0.0),
    );

    let first_byte_histogram = subtract_approx_histogram_counts(
        &archive_delta.first_byte_histogram,
        &materialized_row
            .map(|row| decode_approx_histogram(&row.first_byte_histogram))
            .unwrap_or_else(empty_approx_histogram),
    );
    let first_byte_sample_count = first_byte_histogram.iter().copied().sum::<i64>();
    let first_byte_sum_ms = (archive_delta.first_byte_sum_ms
        - materialized_row
            .map(|row| row.first_byte_sum_ms)
            .unwrap_or(0.0))
    .max(0.0);

    let first_response_byte_total_histogram = subtract_approx_histogram_counts(
        &archive_delta.first_response_byte_total_histogram,
        &materialized_row
            .map(|row| decode_approx_histogram(&row.first_response_byte_total_histogram))
            .unwrap_or_else(empty_approx_histogram),
    );
    let first_response_byte_total_sample_count = first_response_byte_total_histogram
        .iter()
        .copied()
        .sum::<i64>();
    let first_response_byte_total_sum_ms = (archive_delta.first_response_byte_total_sum_ms
        - materialized_row
            .map(|row| row.first_response_byte_total_sum_ms)
            .unwrap_or(0.0))
    .max(0.0);

    let first_token_histogram = subtract_approx_histogram_counts(
        &archive_delta.first_token_histogram,
        &materialized_row
            .map(|row| decode_approx_histogram(&row.first_token_histogram))
            .unwrap_or_else(empty_approx_histogram),
    );
    let first_token_sample_count = first_token_histogram.iter().copied().sum::<i64>();
    let first_token_sum_ms = (archive_delta.first_token_sum_ms
        - materialized_row
            .map(|row| row.first_token_sum_ms)
            .unwrap_or(0.0))
    .max(0.0);

    if total_count <= 0
        && success_count <= 0
        && failure_count <= 0
        && total_tokens <= 0
        && cache_input_tokens <= 0
        && total_cost <= 0.0
        && non_success_cost <= 0.0
        && total_latency_sample_count <= 0
        && first_byte_sample_count <= 0
        && first_response_byte_total_sample_count <= 0
        && first_token_sample_count <= 0
    {
        return Ok(None);
    }

    Ok(Some(InvocationHourlyRollupRecord {
        bucket_start_epoch,
        total_count,
        success_count,
        failure_count,
        total_tokens,
        input_tokens,
        output_tokens,
        cache_input_tokens,
        reasoning_tokens,
        total_cost,
        non_success_cost,
        total_latency_sample_count,
        total_latency_sum_ms,
        first_byte_sample_count,
        first_byte_sum_ms,
        first_byte_max_ms: if first_byte_sample_count > 0 {
            approx_histogram_percentile_ms(&first_byte_histogram, 1.0).unwrap_or(0.0)
        } else {
            0.0
        },
        first_byte_histogram: encode_approx_histogram(&first_byte_histogram)?,
        first_response_byte_total_sample_count,
        first_response_byte_total_sum_ms,
        first_response_byte_total_max_ms: if first_response_byte_total_sample_count > 0 {
            approx_histogram_percentile_ms(&first_response_byte_total_histogram, 1.0).unwrap_or(0.0)
        } else {
            0.0
        },
        first_response_byte_total_histogram: encode_approx_histogram(
            &first_response_byte_total_histogram,
        )?,
        first_token_sample_count,
        first_token_sum_ms,
        first_token_max_ms: if first_token_sample_count > 0 {
            approx_histogram_percentile_ms(&first_token_histogram, 1.0).unwrap_or(0.0)
        } else {
            0.0
        },
        first_token_histogram: encode_approx_histogram(&first_token_histogram)?,
    }))
}

pub(crate) fn build_materialized_pending_invocation_rollup_overlap_record(
    bucket_start_epoch: i64,
    materialized_row: Option<&InvocationHourlyRollupRecord>,
    completed_archive_delta: Option<&InvocationHourlyRollupDelta>,
) -> Result<Option<InvocationHourlyRollupRecord>> {
    let Some(materialized_row) = materialized_row else {
        return Ok(None);
    };

    let total_count = subtract_nonnegative_i64(
        materialized_row.total_count.max(0),
        completed_archive_delta
            .map(|delta| delta.total_count.max(0))
            .unwrap_or(0),
    );
    let success_count = subtract_nonnegative_i64(
        materialized_row.success_count.max(0),
        completed_archive_delta
            .map(|delta| delta.success_count.max(0))
            .unwrap_or(0),
    );
    let failure_count = subtract_nonnegative_i64(
        materialized_row.failure_count.max(0),
        completed_archive_delta
            .map(|delta| delta.failure_count.max(0))
            .unwrap_or(0),
    );
    let total_tokens = subtract_nonnegative_i64(
        materialized_row.total_tokens.max(0),
        completed_archive_delta
            .map(|delta| delta.total_tokens.max(0))
            .unwrap_or(0),
    );
    let input_tokens = subtract_nonnegative_i64(
        materialized_row.input_tokens.max(0),
        completed_archive_delta
            .map(|delta| delta.input_tokens.max(0))
            .unwrap_or(0),
    );
    let output_tokens = subtract_nonnegative_i64(
        materialized_row.output_tokens.max(0),
        completed_archive_delta
            .map(|delta| delta.output_tokens.max(0))
            .unwrap_or(0),
    );
    let cache_input_tokens = subtract_nonnegative_i64(
        materialized_row.cache_input_tokens.max(0),
        completed_archive_delta
            .map(|delta| delta.cache_input_tokens.max(0))
            .unwrap_or(0),
    );
    let reasoning_tokens = subtract_nonnegative_i64(
        materialized_row.reasoning_tokens.max(0),
        completed_archive_delta
            .map(|delta| delta.reasoning_tokens.max(0))
            .unwrap_or(0),
    );
    let total_cost = subtract_nonnegative_f64(
        materialized_row.total_cost,
        completed_archive_delta
            .map(|delta| delta.total_cost)
            .unwrap_or(0.0),
    );
    let non_success_cost = subtract_nonnegative_f64(
        materialized_row.non_success_cost,
        completed_archive_delta
            .map(|delta| delta.non_success_cost)
            .unwrap_or(0.0),
    );
    let total_latency_sample_count = subtract_nonnegative_i64(
        materialized_row.total_latency_sample_count.max(0),
        completed_archive_delta
            .map(|delta| delta.total_latency_sample_count.max(0))
            .unwrap_or(0),
    );
    let total_latency_sum_ms = subtract_nonnegative_f64(
        materialized_row.total_latency_sum_ms,
        completed_archive_delta
            .map(|delta| delta.total_latency_sum_ms)
            .unwrap_or(0.0),
    );

    let materialized_first_byte_histogram =
        decode_approx_histogram(&materialized_row.first_byte_histogram);
    let empty_first_byte_histogram = empty_approx_histogram();
    let first_byte_histogram = subtract_approx_histogram_counts(
        &materialized_first_byte_histogram,
        completed_archive_delta
            .map(|delta| delta.first_byte_histogram.as_slice())
            .unwrap_or(empty_first_byte_histogram.as_slice()),
    );
    let first_byte_sample_count = first_byte_histogram.iter().copied().sum::<i64>();
    let first_byte_sum_ms = subtract_nonnegative_f64(
        materialized_row.first_byte_sum_ms,
        completed_archive_delta
            .map(|delta| delta.first_byte_sum_ms)
            .unwrap_or(0.0),
    );

    let materialized_first_response_byte_total_histogram =
        decode_approx_histogram(&materialized_row.first_response_byte_total_histogram);
    let empty_first_response_byte_total_histogram = empty_approx_histogram();
    let first_response_byte_total_histogram = subtract_approx_histogram_counts(
        &materialized_first_response_byte_total_histogram,
        completed_archive_delta
            .map(|delta| delta.first_response_byte_total_histogram.as_slice())
            .unwrap_or(empty_first_response_byte_total_histogram.as_slice()),
    );
    let first_response_byte_total_sample_count = first_response_byte_total_histogram
        .iter()
        .copied()
        .sum::<i64>();
    let first_response_byte_total_sum_ms = subtract_nonnegative_f64(
        materialized_row.first_response_byte_total_sum_ms,
        completed_archive_delta
            .map(|delta| delta.first_response_byte_total_sum_ms)
            .unwrap_or(0.0),
    );

    let materialized_first_token_histogram =
        decode_approx_histogram(&materialized_row.first_token_histogram);
    let empty_first_token_histogram = empty_approx_histogram();
    let first_token_histogram = subtract_approx_histogram_counts(
        &materialized_first_token_histogram,
        completed_archive_delta
            .map(|delta| delta.first_token_histogram.as_slice())
            .unwrap_or(empty_first_token_histogram.as_slice()),
    );
    let first_token_sample_count = first_token_histogram.iter().copied().sum::<i64>();
    let first_token_sum_ms = subtract_nonnegative_f64(
        materialized_row.first_token_sum_ms,
        completed_archive_delta
            .map(|delta| delta.first_token_sum_ms)
            .unwrap_or(0.0),
    );

    if total_count <= 0
        && success_count <= 0
        && failure_count <= 0
        && total_tokens <= 0
        && cache_input_tokens <= 0
        && total_cost <= 0.0
        && non_success_cost <= 0.0
        && total_latency_sample_count <= 0
        && first_byte_sample_count <= 0
        && first_response_byte_total_sample_count <= 0
        && first_token_sample_count <= 0
    {
        return Ok(None);
    }

    Ok(Some(InvocationHourlyRollupRecord {
        bucket_start_epoch,
        total_count,
        success_count,
        failure_count,
        total_tokens,
        input_tokens,
        output_tokens,
        cache_input_tokens,
        reasoning_tokens,
        total_cost,
        non_success_cost,
        total_latency_sample_count,
        total_latency_sum_ms,
        first_byte_sample_count,
        first_byte_sum_ms,
        first_byte_max_ms: if first_byte_sample_count > 0 {
            approx_histogram_percentile_ms(&first_byte_histogram, 1.0).unwrap_or(0.0)
        } else {
            0.0
        },
        first_byte_histogram: encode_approx_histogram(&first_byte_histogram)?,
        first_response_byte_total_sample_count,
        first_response_byte_total_sum_ms,
        first_response_byte_total_max_ms: if first_response_byte_total_sample_count > 0 {
            approx_histogram_percentile_ms(&first_response_byte_total_histogram, 1.0).unwrap_or(0.0)
        } else {
            0.0
        },
        first_response_byte_total_histogram: encode_approx_histogram(
            &first_response_byte_total_histogram,
        )?,
        first_token_sample_count,
        first_token_sum_ms,
        first_token_max_ms: if first_token_sample_count > 0 {
            approx_histogram_percentile_ms(&first_token_histogram, 1.0).unwrap_or(0.0)
        } else {
            0.0
        },
        first_token_histogram: encode_approx_histogram(&first_token_histogram)?,
    }))
}

pub(crate) async fn query_unmaterialized_invocation_archive_hourly_rollup_deltas(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
) -> Result<Vec<InvocationHourlyRollupRecord>> {
    query_unmaterialized_invocation_archive_hourly_rollup_deltas_with_budget(
        pool,
        source_scope,
        range,
        exclude_invocation_ids,
        None,
        false,
    )
    .await
}

pub(crate) async fn query_unmaterialized_invocation_archive_hourly_rollup_deltas_bounded(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
    max_rows: usize,
) -> Result<Vec<InvocationHourlyRollupRecord>> {
    query_unmaterialized_invocation_archive_hourly_rollup_deltas_with_budget(
        pool,
        source_scope,
        range,
        exclude_invocation_ids,
        Some(max_rows),
        false,
    )
    .await
}

async fn query_unmaterialized_invocation_archive_hourly_rollup_deltas_bounded_strict(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
    max_rows: usize,
) -> Result<Vec<InvocationHourlyRollupRecord>> {
    query_unmaterialized_invocation_archive_hourly_rollup_deltas_with_budget(
        pool,
        source_scope,
        range,
        exclude_invocation_ids,
        Some(max_rows),
        true,
    )
    .await
}

async fn query_unmaterialized_invocation_archive_hourly_rollup_deltas_with_budget(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    exclude_invocation_ids: Option<&HashSet<i64>>,
    max_rows: Option<usize>,
    fail_on_unreadable_unmaterialized: bool,
) -> Result<Vec<InvocationHourlyRollupRecord>> {
    let pending_state = load_pending_invocation_archive_hourly_rollup_deltas(
        pool,
        source_scope,
        range,
        exclude_invocation_ids,
        max_rows,
    )
    .await?;
    if fail_on_unreadable_unmaterialized
        && let Some(path) = pending_state.unreadable_unmaterialized_paths.first()
    {
        return Err(anyhow!("summary archive is unavailable: {path}"));
    }
    let mut pending_bucket_sources = pending_state
        .unmaterialized
        .keys()
        .cloned()
        .collect::<HashSet<_>>();
    pending_bucket_sources.extend(pending_state.materialized.keys().cloned());
    let materialized_bucket_sources = load_materialized_rollup_bucket_sources(
        pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        &pending_bucket_sources,
    )
    .await?;
    let (materialized_overall, mut unreadable_materialized_bucket_start_epochs) =
        load_materialized_invocation_archive_hourly_rollup_deltas_for_bucket_sources(
            pool,
            source_scope,
            range,
            &materialized_bucket_sources,
        )
        .await?;
    let mut known_materialized_overall = materialized_overall;
    merge_invocation_hourly_rollup_delta_map(
        &mut known_materialized_overall,
        &pending_state.materialized,
    )?;
    unreadable_materialized_bucket_start_epochs
        .extend(pending_state.unreadable_materialized_bucket_start_epochs);
    let mut delta_rows = Vec::new();
    for ((bucket_start_epoch, source), archive_delta) in pending_state.unmaterialized {
        let materialized_overlap = if unreadable_materialized_bucket_start_epochs
            .contains(&bucket_start_epoch)
        {
            None
        } else {
            build_materialized_pending_invocation_rollup_overlap_record(
                bucket_start_epoch,
                load_materialized_invocation_rollup_record(pool, bucket_start_epoch, &source)
                    .await?
                    .as_ref(),
                materialized_bucket_sources
                    .contains(&(bucket_start_epoch, source.clone()))
                    .then(|| known_materialized_overall.get(&(bucket_start_epoch, source.clone())))
                    .flatten(),
            )?
        };
        if let Some(delta_row) = build_invocation_hourly_rollup_delta_record(
            bucket_start_epoch,
            &archive_delta,
            materialized_overlap.as_ref(),
        )? {
            delta_rows.push(delta_row);
        }
    }

    delta_rows.sort_by_key(|row| row.bucket_start_epoch);
    Ok(delta_rows)
}
