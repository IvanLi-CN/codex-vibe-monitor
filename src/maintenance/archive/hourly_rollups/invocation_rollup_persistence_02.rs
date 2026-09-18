async fn persist_invocation_hourly_prompt_cache_tx(
    tx: &mut SqliteConnection,
    prompt_cache: InvocationHourlyPromptCacheMap,
) -> Result<()> {
    for ((bucket_start_epoch, source, prompt_cache_key), delta) in prompt_cache {
        sqlx::query(
                r#"
                INSERT INTO prompt_cache_rollup_hourly (
                    bucket_start_epoch,
                    source,
                    prompt_cache_key,
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
                ON CONFLICT(bucket_start_epoch, source, prompt_cache_key) DO UPDATE SET
                    request_count = prompt_cache_rollup_hourly.request_count + excluded.request_count,
                    success_count = prompt_cache_rollup_hourly.success_count + excluded.success_count,
                    failure_count = prompt_cache_rollup_hourly.failure_count + excluded.failure_count,
                    total_tokens = prompt_cache_rollup_hourly.total_tokens + excluded.total_tokens,
                    total_cost = prompt_cache_rollup_hourly.total_cost + excluded.total_cost,
                    first_seen_at = MIN(prompt_cache_rollup_hourly.first_seen_at, excluded.first_seen_at),
                    last_seen_at = MAX(prompt_cache_rollup_hourly.last_seen_at, excluded.last_seen_at),
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(&prompt_cache_key)
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
async fn persist_invocation_hourly_prompt_cache_accounts_tx(
    tx: &mut SqliteConnection,
    prompt_cache_upstream_accounts: InvocationHourlyPromptCacheAccountMap,
) -> Result<()> {
    for (
        (
            bucket_start_epoch,
            source,
            prompt_cache_key,
            upstream_account_key,
            upstream_account_id,
            upstream_account_name,
        ),
        delta,
    ) in prompt_cache_upstream_accounts
    {
        sqlx::query(
                r#"
                INSERT INTO prompt_cache_upstream_account_hourly (
                    bucket_start_epoch,
                    source,
                    prompt_cache_key,
                    upstream_account_key,
                    upstream_account_id,
                    upstream_account_name,
                    request_count,
                    success_count,
                    failure_count,
                    total_tokens,
                    total_cost,
                    first_seen_at,
                    last_seen_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, datetime('now'))
                ON CONFLICT(bucket_start_epoch, source, prompt_cache_key, upstream_account_key) DO UPDATE SET
                    request_count = prompt_cache_upstream_account_hourly.request_count + excluded.request_count,
                    success_count = prompt_cache_upstream_account_hourly.success_count + excluded.success_count,
                    failure_count = prompt_cache_upstream_account_hourly.failure_count + excluded.failure_count,
                    total_tokens = prompt_cache_upstream_account_hourly.total_tokens + excluded.total_tokens,
                    total_cost = prompt_cache_upstream_account_hourly.total_cost + excluded.total_cost,
                    first_seen_at = MIN(prompt_cache_upstream_account_hourly.first_seen_at, excluded.first_seen_at),
                    last_seen_at = MAX(prompt_cache_upstream_account_hourly.last_seen_at, excluded.last_seen_at),
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(&source)
            .bind(&prompt_cache_key)
            .bind(&upstream_account_key)
            .bind(upstream_account_id)
            .bind(upstream_account_name.as_deref())
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
async fn persist_invocation_hourly_account_usage_tx(
    tx: &mut SqliteConnection,
    upstream_account_usage: InvocationHourlyAccountUsageMap,
) -> Result<()> {
    for ((bucket_start_epoch, upstream_account_id), delta) in upstream_account_usage {
        sqlx::query(
                r#"
                INSERT INTO upstream_account_usage_hourly (
                    bucket_start_epoch,
                    upstream_account_id,
                    request_count,
                    success_count,
                    failure_count,
                    total_tokens,
                    total_cost,
                    non_success_cost,
                    input_tokens,
                    output_tokens,
                    cache_input_tokens,
                    reasoning_tokens,
                    first_seen_at,
                    last_seen_at,
                    updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, datetime('now'))
                ON CONFLICT(bucket_start_epoch, upstream_account_id) DO UPDATE SET
                    request_count = upstream_account_usage_hourly.request_count + excluded.request_count,
                    success_count = upstream_account_usage_hourly.success_count + excluded.success_count,
                    failure_count = upstream_account_usage_hourly.failure_count + excluded.failure_count,
                    total_tokens = upstream_account_usage_hourly.total_tokens + excluded.total_tokens,
                    total_cost = upstream_account_usage_hourly.total_cost + excluded.total_cost,
                    non_success_cost = upstream_account_usage_hourly.non_success_cost + excluded.non_success_cost,
                    input_tokens = upstream_account_usage_hourly.input_tokens + excluded.input_tokens,
                    output_tokens = upstream_account_usage_hourly.output_tokens + excluded.output_tokens,
                    cache_input_tokens = upstream_account_usage_hourly.cache_input_tokens + excluded.cache_input_tokens,
                    reasoning_tokens = upstream_account_usage_hourly.reasoning_tokens + excluded.reasoning_tokens,
                    first_seen_at = MIN(upstream_account_usage_hourly.first_seen_at, excluded.first_seen_at),
                    last_seen_at = MAX(upstream_account_usage_hourly.last_seen_at, excluded.last_seen_at),
                    updated_at = datetime('now')
                "#,
            )
            .bind(bucket_start_epoch)
            .bind(upstream_account_id)
            .bind(delta.request_count)
            .bind(delta.success_count)
            .bind(delta.failure_count)
            .bind(delta.total_tokens)
            .bind(delta.total_cost)
            .bind(delta.non_success_cost)
            .bind(delta.input_tokens)
            .bind(delta.output_tokens)
            .bind(delta.cache_input_tokens)
            .bind(delta.reasoning_tokens)
            .bind(&delta.first_seen_at)
            .bind(&delta.last_seen_at)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}
async fn persist_invocation_hourly_breakdown_tx(
    tx: &mut SqliteConnection,
    upstream_account_usage_breakdown: InvocationHourlyBreakdownMap,
) -> Result<()> {
    for (
        (
            bucket_start_epoch,
            source,
            upstream_account_key,
            upstream_account_id,
            normalized_model,
            normalized_reasoning_effort,
        ),
        delta,
    ) in upstream_account_usage_breakdown
    {
        persist_invocation_hourly_breakdown_row(
            tx,
            (
                bucket_start_epoch,
                source,
                upstream_account_key,
                upstream_account_id,
                normalized_model,
                normalized_reasoning_effort,
            ),
            delta,
        )
        .await?;
    }
    Ok(())
}

async fn persist_invocation_hourly_breakdown_row(
    tx: &mut SqliteConnection,
    key: (i64, String, String, Option<i64>, String, String),
    delta: UpstreamAccountUsageBreakdownHourlyDelta,
) -> Result<()> {
    let (
        bucket_start_epoch,
        source,
        upstream_account_key,
        upstream_account_id,
        normalized_model,
        normalized_reasoning_effort,
    ) = key;
    sqlx::query(
        r#"
        INSERT INTO upstream_account_usage_breakdown_hourly (
            bucket_start_epoch, source, upstream_account_key, upstream_account_id,
            normalized_model, normalized_reasoning_effort, request_count, success_count,
            failure_count, cache_write_tokens, cache_read_tokens, output_tokens, cost_input,
            cost_cache_write, cost_cache_read, cost_output, cost_reasoning, cost_unknown, has_cost,
            performance_total_tokens, performance_stream_output_tokens, performance_stream_duration_ms,
            performance_response_sample_count, performance_response_sum_ms,
            performance_first_byte_sample_count, performance_first_byte_sum_ms,
            performance_first_token_sample_count, performance_first_token_sum_ms,
            performance_usage_duration_sample_count, performance_usage_duration_sum_ms, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, datetime('now'))
        ON CONFLICT(bucket_start_epoch, source, upstream_account_key, normalized_model, normalized_reasoning_effort) DO UPDATE SET
            request_count = upstream_account_usage_breakdown_hourly.request_count + excluded.request_count,
            success_count = upstream_account_usage_breakdown_hourly.success_count + excluded.success_count,
            failure_count = upstream_account_usage_breakdown_hourly.failure_count + excluded.failure_count,
            cache_write_tokens = upstream_account_usage_breakdown_hourly.cache_write_tokens + excluded.cache_write_tokens,
            cache_read_tokens = upstream_account_usage_breakdown_hourly.cache_read_tokens + excluded.cache_read_tokens,
            output_tokens = upstream_account_usage_breakdown_hourly.output_tokens + excluded.output_tokens,
            cost_input = upstream_account_usage_breakdown_hourly.cost_input + excluded.cost_input,
            cost_cache_write = upstream_account_usage_breakdown_hourly.cost_cache_write + excluded.cost_cache_write,
            cost_cache_read = upstream_account_usage_breakdown_hourly.cost_cache_read + excluded.cost_cache_read,
            cost_output = upstream_account_usage_breakdown_hourly.cost_output + excluded.cost_output,
            cost_reasoning = upstream_account_usage_breakdown_hourly.cost_reasoning + excluded.cost_reasoning,
            cost_unknown = upstream_account_usage_breakdown_hourly.cost_unknown + excluded.cost_unknown,
            has_cost = upstream_account_usage_breakdown_hourly.has_cost + excluded.has_cost,
            performance_total_tokens = upstream_account_usage_breakdown_hourly.performance_total_tokens + excluded.performance_total_tokens,
            performance_stream_output_tokens = upstream_account_usage_breakdown_hourly.performance_stream_output_tokens + excluded.performance_stream_output_tokens,
            performance_stream_duration_ms = upstream_account_usage_breakdown_hourly.performance_stream_duration_ms + excluded.performance_stream_duration_ms,
            performance_response_sample_count = upstream_account_usage_breakdown_hourly.performance_response_sample_count + excluded.performance_response_sample_count,
            performance_response_sum_ms = upstream_account_usage_breakdown_hourly.performance_response_sum_ms + excluded.performance_response_sum_ms,
            performance_first_byte_sample_count = upstream_account_usage_breakdown_hourly.performance_first_byte_sample_count + excluded.performance_first_byte_sample_count,
            performance_first_byte_sum_ms = upstream_account_usage_breakdown_hourly.performance_first_byte_sum_ms + excluded.performance_first_byte_sum_ms,
            performance_first_token_sample_count = upstream_account_usage_breakdown_hourly.performance_first_token_sample_count + excluded.performance_first_token_sample_count,
            performance_first_token_sum_ms = upstream_account_usage_breakdown_hourly.performance_first_token_sum_ms + excluded.performance_first_token_sum_ms,
            performance_usage_duration_sample_count = upstream_account_usage_breakdown_hourly.performance_usage_duration_sample_count + excluded.performance_usage_duration_sample_count,
            performance_usage_duration_sum_ms = upstream_account_usage_breakdown_hourly.performance_usage_duration_sum_ms + excluded.performance_usage_duration_sum_ms,
            updated_at = datetime('now')
        "#,
    )
    .bind(bucket_start_epoch)
    .bind(&source)
    .bind(&upstream_account_key)
    .bind(upstream_account_id)
    .bind(&normalized_model)
    .bind(&normalized_reasoning_effort)
    .bind(delta.request_count)
    .bind(delta.success_count)
    .bind(delta.failure_count)
    .bind(delta.cache_write_tokens)
    .bind(delta.cache_read_tokens)
    .bind(delta.output_tokens)
    .bind(delta.cost_input)
    .bind(delta.cost_cache_write)
    .bind(delta.cost_cache_read)
    .bind(delta.cost_output)
    .bind(delta.cost_reasoning)
    .bind(delta.cost_unknown)
    .bind(delta.has_cost)
    .bind(delta.performance_total_tokens)
    .bind(delta.performance_stream_output_tokens)
    .bind(delta.performance_stream_duration_ms)
    .bind(delta.performance_response_sample_count)
    .bind(delta.performance_response_sum_ms)
    .bind(delta.performance_first_byte_sample_count)
    .bind(delta.performance_first_byte_sum_ms)
    .bind(delta.performance_first_token_sample_count)
    .bind(delta.performance_first_token_sum_ms)
    .bind(delta.performance_usage_duration_sample_count)
    .bind(delta.performance_usage_duration_sum_ms)
    .execute(&mut *tx)
    .await?;
    Ok(())
}
