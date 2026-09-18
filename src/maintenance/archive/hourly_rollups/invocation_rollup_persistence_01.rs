async fn persist_invocation_hourly_overall_rollups_tx(
    tx: &mut SqliteConnection,
    overall: InvocationHourlyOverallMap,
) -> Result<()> {
    for ((bucket_start_epoch, source), delta) in overall {
        persist_invocation_hourly_overall_row(tx, bucket_start_epoch, &source, delta).await?;
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct InvocationRollupHistogramRow {
    first_byte_histogram: String,
    first_response_byte_total_histogram: String,
    first_token_histogram: String,
}

const INVOCATION_HOURLY_OVERALL_UPSERT_SQL: &str = r#"
INSERT INTO invocation_rollup_hourly (
    bucket_start_epoch, source, total_count, success_count, failure_count, terminal_count,
    terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, cache_input_tokens,
    total_cost, non_success_cost, total_latency_sample_count, total_latency_sum_ms,
    first_byte_sample_count, first_byte_sum_ms, first_byte_max_ms, first_byte_histogram,
    first_response_byte_total_sample_count, first_response_byte_total_sum_ms,
    first_response_byte_total_max_ms, first_response_byte_total_histogram,
    first_token_sample_count, first_token_sum_ms, first_token_max_ms, first_token_histogram,
    input_tokens, output_tokens, reasoning_tokens, updated_at
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, datetime('now'))
ON CONFLICT(bucket_start_epoch, source) DO UPDATE SET
    total_count = invocation_rollup_hourly.total_count + excluded.total_count,
    success_count = invocation_rollup_hourly.success_count + excluded.success_count,
    failure_count = invocation_rollup_hourly.failure_count + excluded.failure_count,
    terminal_count = invocation_rollup_hourly.terminal_count + excluded.terminal_count,
    terminal_tokens = invocation_rollup_hourly.terminal_tokens + excluded.terminal_tokens,
    terminal_cost = invocation_rollup_hourly.terminal_cost + excluded.terminal_cost,
    terminal_proof_complete = 0,
    total_tokens = invocation_rollup_hourly.total_tokens + excluded.total_tokens,
    cache_input_tokens = invocation_rollup_hourly.cache_input_tokens + excluded.cache_input_tokens,
    input_tokens = invocation_rollup_hourly.input_tokens + excluded.input_tokens,
    output_tokens = invocation_rollup_hourly.output_tokens + excluded.output_tokens,
    reasoning_tokens = invocation_rollup_hourly.reasoning_tokens + excluded.reasoning_tokens,
    total_cost = invocation_rollup_hourly.total_cost + excluded.total_cost,
    non_success_cost = invocation_rollup_hourly.non_success_cost + excluded.non_success_cost,
    total_latency_sample_count = invocation_rollup_hourly.total_latency_sample_count + excluded.total_latency_sample_count,
    total_latency_sum_ms = invocation_rollup_hourly.total_latency_sum_ms + excluded.total_latency_sum_ms,
    first_byte_sample_count = invocation_rollup_hourly.first_byte_sample_count + excluded.first_byte_sample_count,
    first_byte_sum_ms = invocation_rollup_hourly.first_byte_sum_ms + excluded.first_byte_sum_ms,
    first_byte_max_ms = MAX(invocation_rollup_hourly.first_byte_max_ms, excluded.first_byte_max_ms),
    first_byte_histogram = excluded.first_byte_histogram,
    first_response_byte_total_sample_count = invocation_rollup_hourly.first_response_byte_total_sample_count + excluded.first_response_byte_total_sample_count,
    first_response_byte_total_sum_ms = invocation_rollup_hourly.first_response_byte_total_sum_ms + excluded.first_response_byte_total_sum_ms,
    first_response_byte_total_max_ms = MAX(invocation_rollup_hourly.first_response_byte_total_max_ms, excluded.first_response_byte_total_max_ms),
    first_response_byte_total_histogram = excluded.first_response_byte_total_histogram,
    first_token_sample_count = invocation_rollup_hourly.first_token_sample_count + excluded.first_token_sample_count,
    first_token_sum_ms = invocation_rollup_hourly.first_token_sum_ms + excluded.first_token_sum_ms,
    first_token_max_ms = MAX(invocation_rollup_hourly.first_token_max_ms, excluded.first_token_max_ms),
    first_token_histogram = excluded.first_token_histogram,
    updated_at = datetime('now')
"#;

async fn persist_invocation_hourly_overall_row(
    tx: &mut SqliteConnection,
    bucket_start_epoch: i64,
    source: &str,
    delta: InvocationHourlyRollupDelta,
) -> Result<()> {
    let current_histograms = sqlx::query_as::<_, InvocationRollupHistogramRow>(
        "SELECT first_byte_histogram, first_response_byte_total_histogram, first_token_histogram
         FROM invocation_rollup_hourly WHERE bucket_start_epoch = ?1 AND source = ?2",
    )
    .bind(bucket_start_epoch)
    .bind(source)
    .fetch_optional(&mut *tx)
    .await?;
    let mut first_byte_histogram = current_histograms
        .as_ref()
        .map(|row| decode_approx_histogram(&row.first_byte_histogram))
        .unwrap_or_else(empty_approx_histogram);
    merge_approx_histogram_into(&mut first_byte_histogram, &delta.first_byte_histogram)?;
    let mut first_response_byte_total_histogram = current_histograms
        .as_ref()
        .map(|row| decode_approx_histogram(&row.first_response_byte_total_histogram))
        .unwrap_or_else(empty_approx_histogram);
    merge_approx_histogram_into(
        &mut first_response_byte_total_histogram,
        &delta.first_response_byte_total_histogram,
    )?;
    let mut first_token_histogram = current_histograms
        .as_ref()
        .map(|row| decode_approx_histogram(&row.first_token_histogram))
        .unwrap_or_else(empty_approx_histogram);
    merge_approx_histogram_into(&mut first_token_histogram, &delta.first_token_histogram)?;
    sqlx::query(INVOCATION_HOURLY_OVERALL_UPSERT_SQL)
        .bind(bucket_start_epoch)
        .bind(source)
        .bind(delta.total_count)
        .bind(delta.success_count)
        .bind(delta.failure_count)
        .bind(delta.terminal_count)
        .bind(delta.terminal_tokens)
        .bind(delta.terminal_cost)
        .bind(delta.total_tokens)
        .bind(delta.cache_input_tokens)
        .bind(delta.total_cost)
        .bind(delta.non_success_cost)
        .bind(delta.total_latency_sample_count)
        .bind(delta.total_latency_sum_ms)
        .bind(delta.first_byte_sample_count)
        .bind(delta.first_byte_sum_ms)
        .bind(delta.first_byte_max_ms)
        .bind(encode_approx_histogram(&first_byte_histogram)?)
        .bind(delta.first_response_byte_total_sample_count)
        .bind(delta.first_response_byte_total_sum_ms)
        .bind(delta.first_response_byte_total_max_ms)
        .bind(encode_approx_histogram(
            &first_response_byte_total_histogram,
        )?)
        .bind(delta.first_token_sample_count)
        .bind(delta.first_token_sum_ms)
        .bind(delta.first_token_max_ms)
        .bind(encode_approx_histogram(&first_token_histogram)?)
        .bind(delta.input_tokens)
        .bind(delta.output_tokens)
        .bind(delta.reasoning_tokens)
        .execute(&mut *tx)
        .await?;
    Ok(())
}
async fn persist_invocation_hourly_failures_tx(
    tx: &mut SqliteConnection,
    failures: InvocationHourlyFailureMap,
) -> Result<()> {
    for (
        (bucket_start_epoch, source, failure_class, is_actionable, error_category),
        failure_count,
    ) in failures
    {
        sqlx::query(
                r#"
                INSERT INTO invocation_failure_rollup_hourly (
                    bucket_start_epoch,
                    source,
                    failure_class,
                    is_actionable,
                    error_category,
                    failure_count,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source, failure_class, is_actionable, error_category) DO UPDATE SET
                    failure_count = invocation_failure_rollup_hourly.failure_count + excluded.failure_count,
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(&failure_class)
            .bind(is_actionable)
            .bind(&error_category)
            .bind(failure_count)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}
async fn persist_invocation_hourly_perf_tx(
    tx: &mut SqliteConnection,
    perf: InvocationHourlyPerfMap,
) -> Result<()> {
    for ((bucket_start_epoch, stage), delta) in perf {
        let current_histogram = sqlx::query_scalar::<_, String>(
            r#"
                SELECT histogram
                FROM proxy_perf_stage_hourly
                WHERE bucket_start_epoch = ?1 AND stage = ?2
                "#,
        )
        .bind(bucket_start_epoch)
        .bind(&stage)
        .fetch_optional(&mut *tx)
        .await?;
        let mut merged_histogram = current_histogram
            .as_deref()
            .map(decode_approx_histogram)
            .unwrap_or_else(empty_approx_histogram);
        merge_approx_histogram_into(&mut merged_histogram, &delta.histogram)?;
        sqlx::query(
            r#"
                INSERT INTO proxy_perf_stage_hourly (
                    bucket_start_epoch,
                    stage,
                    sample_count,
                    sum_ms,
                    max_ms,
                    histogram,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
                ON CONFLICT(bucket_start_epoch, stage) DO UPDATE SET
                    sample_count = proxy_perf_stage_hourly.sample_count + excluded.sample_count,
                    sum_ms = proxy_perf_stage_hourly.sum_ms + excluded.sum_ms,
                    max_ms = MAX(proxy_perf_stage_hourly.max_ms, excluded.max_ms),
                    histogram = excluded.histogram,
                    updated_at = datetime('now')
                "#,
        )
        .bind(bucket_start_epoch)
        .bind(&stage)
        .bind(delta.sample_count)
        .bind(delta.sum_ms)
        .bind(delta.max_ms)
        .bind(encode_approx_histogram(&merged_histogram)?)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}
