struct RollupProjectionExpressions {
    input_tokens: &'static str,
    output_tokens: &'static str,
    cache_input_tokens: &'static str,
    reasoning_tokens: &'static str,
    non_success_cost: &'static str,
    total_latency_sample_count: &'static str,
    total_latency_sum_ms: &'static str,
    first_token_sample_count: &'static str,
    first_token_sum_ms: &'static str,
    first_token_max_ms: &'static str,
    first_token_histogram: &'static str,
}

async fn load_rollup_projection_expressions(
    tx: &mut SqliteConnection,
    table_name: &str,
) -> Result<RollupProjectionExpressions, ApiError> {
    let input_tokens = if sqlite_table_has_column_tx(tx, table_name, "input_tokens").await? {
        "COALESCE(input_tokens, 0) AS input_tokens"
    } else {
        "0 AS input_tokens"
    };
    let output_tokens = if sqlite_table_has_column_tx(tx, table_name, "output_tokens").await? {
        "COALESCE(output_tokens, 0) AS output_tokens"
    } else {
        "0 AS output_tokens"
    };
    let cache_input_tokens =
        if sqlite_table_has_column_tx(tx, table_name, "cache_input_tokens").await? {
            "COALESCE(cache_input_tokens, 0) AS cache_input_tokens"
        } else {
            "0 AS cache_input_tokens"
        };
    let reasoning_tokens = if sqlite_table_has_column_tx(tx, table_name, "reasoning_tokens").await?
    {
        "COALESCE(reasoning_tokens, 0) AS reasoning_tokens"
    } else {
        "0 AS reasoning_tokens"
    };
    let non_success_cost = if sqlite_table_has_column_tx(tx, table_name, "non_success_cost").await?
    {
        "COALESCE(non_success_cost, 0.0) AS non_success_cost"
    } else {
        "0.0 AS non_success_cost"
    };
    let total_latency_sample_count =
        if sqlite_table_has_column_tx(tx, table_name, "total_latency_sample_count").await? {
            "COALESCE(total_latency_sample_count, 0) AS total_latency_sample_count"
        } else {
            "0 AS total_latency_sample_count"
        };
    let total_latency_sum_ms =
        if sqlite_table_has_column_tx(tx, table_name, "total_latency_sum_ms").await? {
            "COALESCE(total_latency_sum_ms, 0.0) AS total_latency_sum_ms"
        } else {
            "0.0 AS total_latency_sum_ms"
        };
    let has_first_token_rollup =
        sqlite_table_has_column_tx(tx, table_name, "first_token_sample_count").await?;
    let first_token_sample_count = if has_first_token_rollup {
        "COALESCE(first_token_sample_count, 0) AS first_token_sample_count"
    } else {
        "0 AS first_token_sample_count"
    };
    let first_token_sum_ms = if has_first_token_rollup {
        "COALESCE(first_token_sum_ms, 0.0) AS first_token_sum_ms"
    } else {
        "0.0 AS first_token_sum_ms"
    };
    let first_token_max_ms = if has_first_token_rollup {
        "COALESCE(first_token_max_ms, 0.0) AS first_token_max_ms"
    } else {
        "0.0 AS first_token_max_ms"
    };
    let first_token_histogram = if has_first_token_rollup {
        "COALESCE(first_token_histogram, '[]') AS first_token_histogram"
    } else {
        "'[]' AS first_token_histogram"
    };
    Ok(RollupProjectionExpressions {
        input_tokens,
        output_tokens,
        cache_input_tokens,
        reasoning_tokens,
        non_success_cost,
        total_latency_sample_count,
        total_latency_sum_ms,
        first_token_sample_count,
        first_token_sum_ms,
        first_token_max_ms,
        first_token_histogram,
    })
}

pub(super) async fn query_invocation_hourly_rollup_range_tx(
    tx: &mut SqliteConnection,
    range_start_epoch: i64,
    range_end_epoch: i64,
    source_scope: InvocationSourceScope,
) -> Result<Vec<InvocationHourlyRollupRecord>, ApiError> {
    let RollupProjectionExpressions {
        input_tokens,
        output_tokens,
        cache_input_tokens,
        reasoning_tokens,
        non_success_cost,
        total_latency_sample_count,
        total_latency_sum_ms,
        first_token_sample_count,
        first_token_sum_ms,
        first_token_max_ms,
        first_token_histogram,
    } = load_rollup_projection_expressions(tx, "invocation_rollup_hourly").await?;
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        r#"
        SELECT
            bucket_start_epoch,
            total_count,
            success_count,
            failure_count,
            total_tokens,
            {input_tokens},
            {output_tokens},
            {cache_input_tokens},
            {reasoning_tokens},
            total_cost,
            {non_success_cost},
            {total_latency_sample_count},
            {total_latency_sum_ms},
            first_byte_sample_count,
            first_byte_sum_ms,
            first_byte_max_ms,
            first_byte_histogram,
            first_response_byte_total_sample_count,
            first_response_byte_total_sum_ms,
            first_response_byte_total_max_ms,
            first_response_byte_total_histogram,
            {first_token_sample_count},
            {first_token_sum_ms},
            {first_token_max_ms},
            {first_token_histogram}
        FROM invocation_rollup_hourly
        WHERE bucket_start_epoch >=
        "#,
    ));
    query.push_bind(range_start_epoch);
    query
        .push(" AND bucket_start_epoch < ")
        .push_bind(range_end_epoch);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" ORDER BY bucket_start_epoch ASC");

    query
        .build_query_as::<InvocationHourlyRollupRecord>()
        .fetch_all(&mut *tx)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_upstream_account_usage_hourly_rollup_range_tx(
    tx: &mut SqliteConnection,
    range_start_epoch: i64,
    range_end_epoch: i64,
    upstream_account_id: i64,
) -> Result<Vec<UpstreamAccountUsageHourlyRollupRecord>, ApiError> {
    let cache_input_tokens_expr =
        if sqlite_table_has_column_tx(tx, "upstream_account_usage_hourly", "cache_input_tokens")
            .await?
        {
            "cache_input_tokens"
        } else {
            "0 AS cache_input_tokens"
        };
    let reasoning_tokens_expr =
        if sqlite_table_has_column_tx(tx, "upstream_account_usage_hourly", "reasoning_tokens")
            .await?
        {
            "COALESCE(reasoning_tokens, 0) AS reasoning_tokens"
        } else {
            "0 AS reasoning_tokens"
        };
    let non_success_cost_expr =
        if sqlite_table_has_column_tx(tx, "upstream_account_usage_hourly", "non_success_cost")
            .await?
        {
            "COALESCE(non_success_cost, 0.0) AS non_success_cost"
        } else {
            "0.0 AS non_success_cost"
        };
    let query = format!(
        r#"
        SELECT
            bucket_start_epoch,
            request_count,
            COALESCE(success_count, 0) AS success_count,
            COALESCE(failure_count, 0) AS failure_count,
            total_tokens,
            {cache_input_tokens_expr},
            {reasoning_tokens_expr},
            total_cost,
            {non_success_cost_expr}
        FROM upstream_account_usage_hourly
        WHERE bucket_start_epoch >= ?1
          AND bucket_start_epoch < ?2
          AND upstream_account_id = ?3
        ORDER BY bucket_start_epoch ASC
        "#,
    );
    sqlx::query_as::<_, UpstreamAccountUsageHourlyRollupRecord>(&query)
        .bind(range_start_epoch)
        .bind(range_end_epoch)
        .bind(upstream_account_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_upstream_account_usage_breakdown_hourly_rollup_range_tx(
    tx: &mut SqliteConnection,
    range_start_epoch: i64,
    range_end_epoch: i64,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<Vec<UpstreamAccountUsageBreakdownHourlyRollupRecord>, ApiError> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            bucket_start_epoch,
            upstream_account_id,
            normalized_model AS model,
            NULLIF(normalized_reasoning_effort, '') AS reasoning_effort,
            request_count,
            success_count,
            failure_count,
            cache_write_tokens,
            cache_read_tokens,
            output_tokens,
            cost_input,
            cost_cache_write,
            cost_cache_read,
            cost_output,
            cost_reasoning,
            cost_unknown,
            has_cost,
            performance_total_tokens,
            performance_stream_output_tokens,
            performance_stream_duration_ms,
            performance_response_sample_count,
            performance_response_sum_ms,
            performance_first_byte_sample_count,
            performance_first_byte_sum_ms,
            performance_first_token_sample_count,
            performance_first_token_sum_ms,
            performance_usage_duration_sample_count,
            performance_usage_duration_sum_ms
        FROM upstream_account_usage_breakdown_hourly
        WHERE bucket_start_epoch >=
        "#,
    );
    query.push_bind(range_start_epoch);
    query
        .push(" AND bucket_start_epoch < ")
        .push_bind(range_end_epoch);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND upstream_account_id = ")
            .push_bind(upstream_account_id);
    }
    query.push(
        " ORDER BY bucket_start_epoch ASC, upstream_account_id ASC, normalized_model ASC, normalized_reasoning_effort ASC",
    );

    query
        .build_query_as::<UpstreamAccountUsageBreakdownHourlyRollupRecord>()
        .fetch_all(&mut *tx)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_upstream_account_usage_breakdown_hourly_rollup_range_bounded_tx(
    tx: &mut SqliteConnection,
    range_start_epoch: i64,
    range_end_epoch: i64,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    max_bytes: usize,
) -> Result<Vec<UpstreamAccountUsageBreakdownHourlyRollupRecord>, ApiError> {
    let max_bytes = i64::try_from(max_bytes)
        .map_err(|_| anyhow!("usage rollup byte budget does not fit SQLite integer"))?;
    let mut budget_query = QueryBuilder::<Sqlite>::new(
        "SELECT COALESCE(SUM(CAST(256 + \
         length(CAST(COALESCE(normalized_model, '') AS BLOB)) + \
         length(CAST(COALESCE(normalized_reasoning_effort, '') AS BLOB)) AS INTEGER)), 0) \
         FROM upstream_account_usage_breakdown_hourly WHERE bucket_start_epoch >= ",
    );
    budget_query
        .push_bind(range_start_epoch)
        .push(" AND bucket_start_epoch < ")
        .push_bind(range_end_epoch);
    if source_scope == InvocationSourceScope::ProxyOnly {
        budget_query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        budget_query
            .push(" AND upstream_account_id = ")
            .push_bind(upstream_account_id);
    }
    let estimated_bytes = budget_query
        .build_query_scalar::<i64>()
        .fetch_one(&mut *tx)
        .await
        .map_err(ApiError::from)?;
    if estimated_bytes > max_bytes {
        return Err(ApiError::from(anyhow!(
            "usage rollup byte budget exceeded ({estimated_bytes} > {max_bytes})"
        )));
    }

    query_upstream_account_usage_breakdown_hourly_rollup_range_tx(
        tx,
        range_start_epoch,
        range_end_epoch,
        source_scope,
        upstream_account_id,
    )
    .await
}
