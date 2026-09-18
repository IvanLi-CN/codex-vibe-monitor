async fn persist_invocation_hourly_account_stats_tx(
    tx: &mut SqliteConnection,
    upstream_account_stats_hourly: InvocationHourlyStatsMap,
) -> Result<()> {
    for ((bucket_start_epoch, source, upstream_account_id), delta) in upstream_account_stats_hourly
    {
        persist_invocation_hourly_account_stats_row(
            tx,
            bucket_start_epoch,
            &source,
            upstream_account_id,
            delta,
        )
        .await?;
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct AccountStatsHistogramRow {
    first_byte_histogram: String,
    first_response_byte_total_histogram: String,
    first_token_histogram: String,
}

const ACCOUNT_STATS_HOURLY_UPSERT_SQL: &str = r#"
INSERT INTO upstream_account_stats_hourly (
    bucket_start_epoch, source, upstream_account_id, total_count, success_count, failure_count,
    in_flight_count, total_tokens, input_tokens, output_tokens, cache_input_tokens, total_cost,
    non_success_cost, total_latency_sample_count, total_latency_sum_ms, first_byte_sample_count,
    first_byte_sum_ms, first_byte_max_ms, first_byte_histogram, first_response_byte_total_sample_count,
    first_response_byte_total_sum_ms, first_response_byte_total_max_ms,
    first_response_byte_total_histogram, first_token_sample_count, first_token_sum_ms,
    first_token_max_ms, first_token_histogram, reasoning_tokens, updated_at
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, datetime('now'))
ON CONFLICT(bucket_start_epoch, source, upstream_account_id) DO UPDATE SET
    total_count = upstream_account_stats_hourly.total_count + excluded.total_count,
    success_count = upstream_account_stats_hourly.success_count + excluded.success_count,
    failure_count = upstream_account_stats_hourly.failure_count + excluded.failure_count,
    in_flight_count = upstream_account_stats_hourly.in_flight_count + excluded.in_flight_count,
    total_tokens = upstream_account_stats_hourly.total_tokens + excluded.total_tokens,
    input_tokens = upstream_account_stats_hourly.input_tokens + excluded.input_tokens,
    output_tokens = upstream_account_stats_hourly.output_tokens + excluded.output_tokens,
    cache_input_tokens = upstream_account_stats_hourly.cache_input_tokens + excluded.cache_input_tokens,
    reasoning_tokens = upstream_account_stats_hourly.reasoning_tokens + excluded.reasoning_tokens,
    total_cost = upstream_account_stats_hourly.total_cost + excluded.total_cost,
    non_success_cost = upstream_account_stats_hourly.non_success_cost + excluded.non_success_cost,
    total_latency_sample_count = upstream_account_stats_hourly.total_latency_sample_count + excluded.total_latency_sample_count,
    total_latency_sum_ms = upstream_account_stats_hourly.total_latency_sum_ms + excluded.total_latency_sum_ms,
    first_byte_sample_count = upstream_account_stats_hourly.first_byte_sample_count + excluded.first_byte_sample_count,
    first_byte_sum_ms = upstream_account_stats_hourly.first_byte_sum_ms + excluded.first_byte_sum_ms,
    first_byte_max_ms = MAX(upstream_account_stats_hourly.first_byte_max_ms, excluded.first_byte_max_ms),
    first_byte_histogram = excluded.first_byte_histogram,
    first_response_byte_total_sample_count = upstream_account_stats_hourly.first_response_byte_total_sample_count + excluded.first_response_byte_total_sample_count,
    first_response_byte_total_sum_ms = upstream_account_stats_hourly.first_response_byte_total_sum_ms + excluded.first_response_byte_total_sum_ms,
    first_response_byte_total_max_ms = MAX(upstream_account_stats_hourly.first_response_byte_total_max_ms, excluded.first_response_byte_total_max_ms),
    first_response_byte_total_histogram = excluded.first_response_byte_total_histogram,
    first_token_sample_count = upstream_account_stats_hourly.first_token_sample_count + excluded.first_token_sample_count,
    first_token_sum_ms = upstream_account_stats_hourly.first_token_sum_ms + excluded.first_token_sum_ms,
    first_token_max_ms = MAX(upstream_account_stats_hourly.first_token_max_ms, excluded.first_token_max_ms),
    first_token_histogram = excluded.first_token_histogram,
    updated_at = datetime('now')
"#;

async fn persist_invocation_hourly_account_stats_row(
    tx: &mut SqliteConnection,
    bucket_start_epoch: i64,
    source: &str,
    upstream_account_id: i64,
    delta: UpstreamAccountStatsDelta,
) -> Result<()> {
    let current_histograms = sqlx::query_as::<_, AccountStatsHistogramRow>(
        "SELECT first_byte_histogram, first_response_byte_total_histogram, first_token_histogram
         FROM upstream_account_stats_hourly
         WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = ?3",
    )
    .bind(bucket_start_epoch)
    .bind(source)
    .bind(upstream_account_id)
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
    sqlx::query(ACCOUNT_STATS_HOURLY_UPSERT_SQL)
        .bind(bucket_start_epoch)
        .bind(source)
        .bind(upstream_account_id)
        .bind(delta.total_count)
        .bind(delta.success_count)
        .bind(delta.failure_count)
        .bind(delta.in_flight_count)
        .bind(delta.total_tokens)
        .bind(delta.input_tokens)
        .bind(delta.output_tokens)
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
        .bind(delta.reasoning_tokens)
        .execute(&mut *tx)
        .await?;
    Ok(())
}
async fn persist_invocation_hourly_activity_tx(
    tx: &mut SqliteConnection,
    upstream_account_activity_v2: InvocationHourlyStatsMap,
) -> Result<()> {
    for ((bucket_start_epoch, source, upstream_account_id), delta) in upstream_account_activity_v2 {
        persist_invocation_hourly_activity_row(
            tx,
            bucket_start_epoch,
            &source,
            upstream_account_id,
            delta,
        )
        .await?;
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct ActivityV2StatsHistogramRow {
    activity_v2_first_token_histogram: String,
}

const ACTIVITY_V2_UPSERT_SQL: &str = r#"
INSERT INTO upstream_account_stats_hourly (
    bucket_start_epoch, source, upstream_account_id, activity_v2_request_count,
    activity_v2_success_count, activity_v2_failure_count, activity_v2_non_success_count,
    activity_v2_total_tokens, activity_v2_success_tokens, activity_v2_non_success_tokens,
    activity_v2_failure_tokens, activity_v2_failure_cost, activity_v2_non_success_cost,
    activity_v2_cache_input_tokens, activity_v2_total_cost, activity_v2_first_response_sample_count,
    activity_v2_first_response_sum_ms, activity_v2_total_latency_sample_count,
    activity_v2_total_latency_sum_ms, activity_v2_last_invocation_at,
    activity_v2_latest_unkeyed_conversation_at, activity_v2_latest_first_response_at,
    activity_v2_latest_first_response_ms, activity_v2_latest_total_latency_at,
    activity_v2_latest_total_latency_ms, activity_v2_first_token_sample_count,
    activity_v2_first_token_sum_ms, activity_v2_first_token_max_ms,
    activity_v2_first_token_histogram, updated_at
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, datetime('now'))
ON CONFLICT(bucket_start_epoch, source, upstream_account_id) DO UPDATE SET
    activity_v2_request_count = upstream_account_stats_hourly.activity_v2_request_count + excluded.activity_v2_request_count,
    activity_v2_success_count = upstream_account_stats_hourly.activity_v2_success_count + excluded.activity_v2_success_count,
    activity_v2_failure_count = upstream_account_stats_hourly.activity_v2_failure_count + excluded.activity_v2_failure_count,
    activity_v2_non_success_count = upstream_account_stats_hourly.activity_v2_non_success_count + excluded.activity_v2_non_success_count,
    activity_v2_total_tokens = upstream_account_stats_hourly.activity_v2_total_tokens + excluded.activity_v2_total_tokens,
    activity_v2_success_tokens = upstream_account_stats_hourly.activity_v2_success_tokens + excluded.activity_v2_success_tokens,
    activity_v2_non_success_tokens = upstream_account_stats_hourly.activity_v2_non_success_tokens + excluded.activity_v2_non_success_tokens,
    activity_v2_failure_tokens = upstream_account_stats_hourly.activity_v2_failure_tokens + excluded.activity_v2_failure_tokens,
    activity_v2_failure_cost = upstream_account_stats_hourly.activity_v2_failure_cost + excluded.activity_v2_failure_cost,
    activity_v2_non_success_cost = upstream_account_stats_hourly.activity_v2_non_success_cost + excluded.activity_v2_non_success_cost,
    activity_v2_cache_input_tokens = upstream_account_stats_hourly.activity_v2_cache_input_tokens + excluded.activity_v2_cache_input_tokens,
    activity_v2_total_cost = upstream_account_stats_hourly.activity_v2_total_cost + excluded.activity_v2_total_cost,
    activity_v2_first_response_sample_count = upstream_account_stats_hourly.activity_v2_first_response_sample_count + excluded.activity_v2_first_response_sample_count,
    activity_v2_first_response_sum_ms = upstream_account_stats_hourly.activity_v2_first_response_sum_ms + excluded.activity_v2_first_response_sum_ms,
    activity_v2_total_latency_sample_count = upstream_account_stats_hourly.activity_v2_total_latency_sample_count + excluded.activity_v2_total_latency_sample_count,
    activity_v2_total_latency_sum_ms = upstream_account_stats_hourly.activity_v2_total_latency_sum_ms + excluded.activity_v2_total_latency_sum_ms,
    activity_v2_last_invocation_at = CASE WHEN upstream_account_stats_hourly.activity_v2_last_invocation_at IS NULL OR excluded.activity_v2_last_invocation_at > upstream_account_stats_hourly.activity_v2_last_invocation_at THEN excluded.activity_v2_last_invocation_at ELSE upstream_account_stats_hourly.activity_v2_last_invocation_at END,
    activity_v2_latest_unkeyed_conversation_at = CASE WHEN upstream_account_stats_hourly.activity_v2_latest_unkeyed_conversation_at IS NULL OR excluded.activity_v2_latest_unkeyed_conversation_at > upstream_account_stats_hourly.activity_v2_latest_unkeyed_conversation_at THEN excluded.activity_v2_latest_unkeyed_conversation_at ELSE upstream_account_stats_hourly.activity_v2_latest_unkeyed_conversation_at END,
    activity_v2_latest_first_response_ms = CASE WHEN upstream_account_stats_hourly.activity_v2_latest_first_response_at IS NULL OR excluded.activity_v2_latest_first_response_at > upstream_account_stats_hourly.activity_v2_latest_first_response_at THEN excluded.activity_v2_latest_first_response_ms ELSE upstream_account_stats_hourly.activity_v2_latest_first_response_ms END,
    activity_v2_latest_first_response_at = CASE WHEN upstream_account_stats_hourly.activity_v2_latest_first_response_at IS NULL OR excluded.activity_v2_latest_first_response_at > upstream_account_stats_hourly.activity_v2_latest_first_response_at THEN excluded.activity_v2_latest_first_response_at ELSE upstream_account_stats_hourly.activity_v2_latest_first_response_at END,
    activity_v2_latest_total_latency_ms = CASE WHEN upstream_account_stats_hourly.activity_v2_latest_total_latency_at IS NULL OR excluded.activity_v2_latest_total_latency_at > upstream_account_stats_hourly.activity_v2_latest_total_latency_at THEN excluded.activity_v2_latest_total_latency_ms ELSE upstream_account_stats_hourly.activity_v2_latest_total_latency_ms END,
    activity_v2_latest_total_latency_at = CASE WHEN upstream_account_stats_hourly.activity_v2_latest_total_latency_at IS NULL OR excluded.activity_v2_latest_total_latency_at > upstream_account_stats_hourly.activity_v2_latest_total_latency_at THEN excluded.activity_v2_latest_total_latency_at ELSE upstream_account_stats_hourly.activity_v2_latest_total_latency_at END,
    activity_v2_first_token_sample_count = upstream_account_stats_hourly.activity_v2_first_token_sample_count + excluded.activity_v2_first_token_sample_count,
    activity_v2_first_token_sum_ms = upstream_account_stats_hourly.activity_v2_first_token_sum_ms + excluded.activity_v2_first_token_sum_ms,
    activity_v2_first_token_max_ms = MAX(upstream_account_stats_hourly.activity_v2_first_token_max_ms, excluded.activity_v2_first_token_max_ms),
    activity_v2_first_token_histogram = excluded.activity_v2_first_token_histogram,
    updated_at = datetime('now')
"#;

async fn persist_invocation_hourly_activity_row(
    tx: &mut SqliteConnection,
    bucket_start_epoch: i64,
    source: &str,
    upstream_account_id: i64,
    delta: UpstreamAccountStatsDelta,
) -> Result<()> {
    let current_histogram = sqlx::query_as::<_, ActivityV2StatsHistogramRow>(
        "SELECT activity_v2_first_token_histogram FROM upstream_account_stats_hourly
         WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = ?3",
    )
    .bind(bucket_start_epoch)
    .bind(source)
    .bind(upstream_account_id)
    .fetch_optional(&mut *tx)
    .await?;
    let mut first_token_histogram = current_histogram
        .as_ref()
        .map(|row| decode_approx_histogram(&row.activity_v2_first_token_histogram))
        .unwrap_or_else(empty_approx_histogram);
    merge_approx_histogram_into(&mut first_token_histogram, &delta.first_token_histogram)?;
    sqlx::query(ACTIVITY_V2_UPSERT_SQL)
        .bind(bucket_start_epoch)
        .bind(source)
        .bind(upstream_account_id)
        .bind(delta.total_count)
        .bind(delta.success_count)
        .bind(delta.failure_count)
        .bind(delta.non_success_count)
        .bind(delta.total_tokens)
        .bind(delta.success_tokens)
        .bind(delta.non_success_tokens)
        .bind(delta.failure_tokens)
        .bind(delta.failure_cost)
        .bind(delta.non_success_cost)
        .bind(delta.cache_input_tokens)
        .bind(delta.total_cost)
        .bind(delta.first_response_byte_total_sample_count)
        .bind(delta.first_response_byte_total_sum_ms)
        .bind(delta.total_latency_sample_count)
        .bind(delta.total_latency_sum_ms)
        .bind(delta.last_invocation_at.as_deref())
        .bind(delta.latest_unkeyed_conversation_at.as_deref())
        .bind(delta.latest_first_response_byte_total_at.as_deref())
        .bind(delta.latest_first_response_byte_total_ms)
        .bind(delta.latest_total_latency_at.as_deref())
        .bind(delta.latest_total_latency_ms)
        .bind(delta.first_token_sample_count)
        .bind(delta.first_token_sum_ms)
        .bind(delta.first_token_max_ms)
        .bind(encode_approx_histogram(&first_token_histogram)?)
        .execute(&mut *tx)
        .await?;
    Ok(())
}
