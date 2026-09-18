#[derive(sqlx::FromRow)]
struct AccountMinuteStatsHistogramRow {
    first_byte_histogram: String,
    first_response_byte_total_histogram: String,
    first_token_histogram: String,
}

struct MergedAccountMinuteStatsHistograms {
    first_byte_histogram: String,
    first_response_byte_total_histogram: String,
    first_token_histogram: String,
}

const ACCOUNT_MINUTE_STATS_UPSERT_SQL: &str = r#"
INSERT INTO upstream_account_stats_minute (
    bucket_start_epoch, source, upstream_account_id, total_count, success_count, failure_count,
    in_flight_count, total_tokens, input_tokens, output_tokens, cache_input_tokens, total_cost,
    non_success_cost, total_latency_sample_count, total_latency_sum_ms, first_byte_sample_count,
    first_byte_sum_ms, first_byte_max_ms, first_byte_histogram,
    first_response_byte_total_sample_count, first_response_byte_total_sum_ms,
    first_response_byte_total_max_ms, first_response_byte_total_histogram,
    first_token_sample_count, first_token_sum_ms, first_token_max_ms, first_token_histogram,
    reasoning_tokens, updated_at
)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, datetime('now'))
ON CONFLICT(bucket_start_epoch, source, upstream_account_id) DO UPDATE SET
    total_count = upstream_account_stats_minute.total_count + excluded.total_count,
    success_count = upstream_account_stats_minute.success_count + excluded.success_count,
    failure_count = upstream_account_stats_minute.failure_count + excluded.failure_count,
    in_flight_count = upstream_account_stats_minute.in_flight_count + excluded.in_flight_count,
    total_tokens = upstream_account_stats_minute.total_tokens + excluded.total_tokens,
    input_tokens = upstream_account_stats_minute.input_tokens + excluded.input_tokens,
    output_tokens = upstream_account_stats_minute.output_tokens + excluded.output_tokens,
    cache_input_tokens = upstream_account_stats_minute.cache_input_tokens + excluded.cache_input_tokens,
    reasoning_tokens = upstream_account_stats_minute.reasoning_tokens + excluded.reasoning_tokens,
    total_cost = upstream_account_stats_minute.total_cost + excluded.total_cost,
    non_success_cost = upstream_account_stats_minute.non_success_cost + excluded.non_success_cost,
    total_latency_sample_count = upstream_account_stats_minute.total_latency_sample_count + excluded.total_latency_sample_count,
    total_latency_sum_ms = upstream_account_stats_minute.total_latency_sum_ms + excluded.total_latency_sum_ms,
    first_byte_sample_count = upstream_account_stats_minute.first_byte_sample_count + excluded.first_byte_sample_count,
    first_byte_sum_ms = upstream_account_stats_minute.first_byte_sum_ms + excluded.first_byte_sum_ms,
    first_byte_max_ms = MAX(upstream_account_stats_minute.first_byte_max_ms, excluded.first_byte_max_ms),
    first_byte_histogram = excluded.first_byte_histogram,
    first_response_byte_total_sample_count = upstream_account_stats_minute.first_response_byte_total_sample_count + excluded.first_response_byte_total_sample_count,
    first_response_byte_total_sum_ms = upstream_account_stats_minute.first_response_byte_total_sum_ms + excluded.first_response_byte_total_sum_ms,
    first_response_byte_total_max_ms = MAX(upstream_account_stats_minute.first_response_byte_total_max_ms, excluded.first_response_byte_total_max_ms),
    first_response_byte_total_histogram = excluded.first_response_byte_total_histogram,
    first_token_sample_count = upstream_account_stats_minute.first_token_sample_count + excluded.first_token_sample_count,
    first_token_sum_ms = upstream_account_stats_minute.first_token_sum_ms + excluded.first_token_sum_ms,
    first_token_max_ms = MAX(upstream_account_stats_minute.first_token_max_ms, excluded.first_token_max_ms),
    first_token_histogram = excluded.first_token_histogram,
    updated_at = datetime('now')
"#;

async fn persist_invocation_hourly_account_minute_stats_tx(
    tx: &mut SqliteConnection,
    upstream_account_stats_minute: InvocationHourlyStatsMap,
) -> Result<()> {
    for ((bucket_start_epoch, source, upstream_account_id), delta) in upstream_account_stats_minute
    {
        persist_invocation_hourly_account_minute_stats_row(
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

async fn load_account_minute_stats_histograms(
    tx: &mut SqliteConnection,
    bucket_start_epoch: i64,
    source: &str,
    upstream_account_id: i64,
) -> Result<Option<AccountMinuteStatsHistogramRow>> {
    Ok(sqlx::query_as::<_, AccountMinuteStatsHistogramRow>(
        "SELECT first_byte_histogram, first_response_byte_total_histogram, first_token_histogram
         FROM upstream_account_stats_minute
         WHERE bucket_start_epoch = ?1 AND source = ?2 AND upstream_account_id = ?3",
    )
    .bind(bucket_start_epoch)
    .bind(source)
    .bind(upstream_account_id)
    .fetch_optional(&mut *tx)
    .await?)
}

fn merge_account_minute_stats_histograms(
    current: Option<&AccountMinuteStatsHistogramRow>,
    delta: &UpstreamAccountStatsDelta,
) -> Result<MergedAccountMinuteStatsHistograms> {
    let mut first_byte_histogram = current
        .map(|row| decode_approx_histogram(&row.first_byte_histogram))
        .unwrap_or_else(empty_approx_histogram);
    merge_approx_histogram_into(&mut first_byte_histogram, &delta.first_byte_histogram)?;
    let mut first_response_byte_total_histogram = current
        .map(|row| decode_approx_histogram(&row.first_response_byte_total_histogram))
        .unwrap_or_else(empty_approx_histogram);
    merge_approx_histogram_into(
        &mut first_response_byte_total_histogram,
        &delta.first_response_byte_total_histogram,
    )?;
    let mut first_token_histogram = current
        .map(|row| decode_approx_histogram(&row.first_token_histogram))
        .unwrap_or_else(empty_approx_histogram);
    merge_approx_histogram_into(&mut first_token_histogram, &delta.first_token_histogram)?;
    Ok(MergedAccountMinuteStatsHistograms {
        first_byte_histogram: encode_approx_histogram(&first_byte_histogram)?,
        first_response_byte_total_histogram: encode_approx_histogram(
            &first_response_byte_total_histogram,
        )?,
        first_token_histogram: encode_approx_histogram(&first_token_histogram)?,
    })
}

async fn persist_invocation_hourly_account_minute_stats_row(
    tx: &mut SqliteConnection,
    bucket_start_epoch: i64,
    source: &str,
    upstream_account_id: i64,
    delta: UpstreamAccountStatsDelta,
) -> Result<()> {
    let current =
        load_account_minute_stats_histograms(tx, bucket_start_epoch, source, upstream_account_id)
            .await?;
    let histograms = merge_account_minute_stats_histograms(current.as_ref(), &delta)?;
    sqlx::query(ACCOUNT_MINUTE_STATS_UPSERT_SQL)
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
        .bind(histograms.first_byte_histogram)
        .bind(delta.first_response_byte_total_sample_count)
        .bind(delta.first_response_byte_total_sum_ms)
        .bind(delta.first_response_byte_total_max_ms)
        .bind(histograms.first_response_byte_total_histogram)
        .bind(delta.first_token_sample_count)
        .bind(delta.first_token_sum_ms)
        .bind(delta.first_token_max_ms)
        .bind(histograms.first_token_histogram)
        .bind(delta.reasoning_tokens)
        .execute(&mut *tx)
        .await?;
    Ok(())
}
async fn persist_invocation_hourly_sticky_keys_tx(
    tx: &mut SqliteConnection,
    sticky_keys: InvocationHourlyStickyMap,
) -> Result<()> {
    for ((bucket_start_epoch, upstream_account_id, sticky_key), delta) in sticky_keys {
        sqlx::query(
                r#"
                INSERT INTO upstream_sticky_key_hourly (
                    bucket_start_epoch,
                    upstream_account_id,
                    sticky_key,
                    request_count,
                    success_count,
                    failure_count,
                    total_tokens,
                    total_cost,
                    first_seen_at,
                    last_seen_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'))
                ON CONFLICT(bucket_start_epoch, upstream_account_id, sticky_key) DO UPDATE SET
                    request_count = upstream_sticky_key_hourly.request_count + excluded.request_count,
                    success_count = upstream_sticky_key_hourly.success_count + excluded.success_count,
                    failure_count = upstream_sticky_key_hourly.failure_count + excluded.failure_count,
                    total_tokens = upstream_sticky_key_hourly.total_tokens + excluded.total_tokens,
                    total_cost = upstream_sticky_key_hourly.total_cost + excluded.total_cost,
                    first_seen_at = MIN(upstream_sticky_key_hourly.first_seen_at, excluded.first_seen_at),
                    last_seen_at = MAX(upstream_sticky_key_hourly.last_seen_at, excluded.last_seen_at),
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(upstream_account_id)
            .bind(&sticky_key)
            .bind(delta.request_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.total_tokens)
            .bind(delta.total_cost)
            .bind(&delta.first_seen_at)
            .bind(&delta.last_seen_at)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}
