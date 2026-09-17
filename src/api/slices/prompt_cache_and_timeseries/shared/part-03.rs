pub(crate) async fn query_upstream_account_stats_rollup_range_tx(
    tx: &mut SqliteConnection,
    table_name: &str,
    range_start_epoch: i64,
    range_end_epoch: i64,
    source_scope: InvocationSourceScope,
    upstream_account_id: i64,
) -> Result<Vec<UpstreamAccountStatsRollupRecord>, ApiError> {
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
    } = load_rollup_projection_expressions(tx, table_name).await?;
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        r#"
        SELECT
            bucket_start_epoch,
            total_count,
            success_count,
            failure_count,
            in_flight_count,
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
        FROM {table_name}
        WHERE bucket_start_epoch >=
        "#,
    ));
    query.push_bind(range_start_epoch);
    query
        .push(" AND bucket_start_epoch < ")
        .push_bind(range_end_epoch)
        .push(" AND upstream_account_id = ")
        .push_bind(upstream_account_id);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" ORDER BY bucket_start_epoch ASC");

    query
        .build_query_as::<UpstreamAccountStatsRollupRecord>()
        .fetch_all(&mut *tx)
        .await
        .map_err(Into::into)
}

pub(crate) async fn sqlite_table_has_column_tx(
    tx: &mut SqliteConnection,
    table_name: &str,
    column_name: &str,
) -> Result<bool, ApiError> {
    let escaped_table_name = table_name.replace('\'', "''");
    let pragma = format!("PRAGMA table_info('{escaped_table_name}')");
    let rows = sqlx::query(&pragma).fetch_all(&mut *tx).await?;
    Ok(rows.into_iter().any(|row| {
        row.try_get::<String, _>("name")
            .is_ok_and(|name| name == column_name)
    }))
}

pub(super) fn add_invocation_record_to_summary_totals(
    totals: &mut StatsTotals,
    record: &InvocationAggregateRecord,
) {
    totals.total_count += 1;
    let classification = resolve_failure_classification(
        record.status.as_deref(),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        record.failure_class.as_deref(),
        record.is_actionable,
    );
    if prompt_invocation_status_is_success_like(
        record.status.as_deref(),
        record.error_message.as_deref(),
    ) && classification.failure_class == FailureClass::None
    {
        totals.success_count += 1;
    } else if prompt_invocation_status_counts_toward_terminal_totals(record.status.as_deref())
        && classification.failure_class != FailureClass::None
    {
        totals.failure_count += 1;
    }
    totals.total_tokens += record.total_tokens.unwrap_or_default();
    totals.total_cost += record.cost.unwrap_or_default();
    if invocation_counts_toward_non_success_usage(
        record.status.as_deref(),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        record.failure_class.as_deref(),
        record.is_actionable,
    ) {
        totals.non_success_cost += record.cost.unwrap_or_default();
    }
}

pub(crate) fn db_occurred_at_upper_bound(end_utc: DateTime<Utc>) -> String {
    if end_utc.timestamp_subsec_nanos() > 0 {
        return db_occurred_at_lower_bound(end_utc + ChronoDuration::seconds(1));
    }
    db_occurred_at_lower_bound(end_utc)
}

pub(crate) fn record_perf_stage_sample(
    by_stage: &mut BTreeMap<String, (i64, f64, f64, ApproxHistogramCounts)>,
    stage: &str,
    value: Option<f64>,
) {
    let Some(value) = value.filter(|value| is_valid_perf_stage_sample(stage, *value)) else {
        return;
    };
    let entry = by_stage
        .entry(stage.to_string())
        .or_insert_with(|| (0, 0.0, 0.0, empty_approx_histogram()));
    entry.0 += 1;
    entry.1 += value;
    entry.2 = entry.2.max(value);
    add_approx_histogram_sample(&mut entry.3, value);
}

pub(crate) fn is_valid_perf_stage_sample(stage: &str, value: f64) -> bool {
    value.is_finite() && value >= 0.0 && (stage != "upstreamStream" || value > 0.0)
}

#[cfg(test)]
mod in_flight_query_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn perf_stage_samples_reject_invalid_values_and_zero_streams() {
        assert!(is_valid_perf_stage_sample("upstreamFirstByte", 0.0));
        assert!(is_valid_perf_stage_sample("upstreamStream", 0.1));
        assert!(!is_valid_perf_stage_sample("upstreamStream", 0.0));
        assert!(!is_valid_perf_stage_sample("total", -0.1));
        assert!(!is_valid_perf_stage_sample("total", f64::INFINITY));
        assert!(!is_valid_perf_stage_sample("total", f64::NAN));
    }

    #[tokio::test]
    async fn in_flight_query_ignores_projection_cursor_and_keeps_account_scope() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, status TEXT, total_tokens INTEGER, cache_input_tokens INTEGER, cost REAL, error_message TEXT, payload TEXT, failure_kind TEXT, failure_class TEXT, source TEXT, t_total_ms REAL, t_req_read_ms REAL, t_req_parse_ms REAL, t_upstream_connect_ms REAL, t_upstream_ttfb_ms REAL, first_token_ms REAL, t_upstream_stream_ms REAL, t_resp_parse_ms REAL, t_persist_ms REAL)",
        )
        .execute(&pool)
        .await
        .expect("create invocations");
        sqlx::query(
            "CREATE TABLE pool_upstream_request_attempts (id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, attempt_index INTEGER NOT NULL, status TEXT, phase TEXT, stream_latency_ms REAL, first_byte_latency_ms REAL)",
        )
        .execute(&pool)
        .await
        .expect("create pool attempts");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, payload, source) VALUES (1, 'running', '2026-08-03 08:00:10', 'running', '{\"upstreamAccountId\":7}', 'proxy'), (2, 'terminal', '2026-08-03 08:00:20', 'success', '{\"upstreamAccountId\":7}', 'proxy')",
        )
        .execute(&pool)
        .await
        .expect("insert in-flight and terminal records");
        let range = ExactUtcRange {
            start: Utc.with_ymd_and_hms(2026, 8, 3, 0, 0, 0).single().unwrap(),
            end: Utc.with_ymd_and_hms(2026, 8, 3, 0, 1, 0).single().unwrap(),
        };

        let global = query_in_flight_invocation_aggregate_records_from_live_range(
            &pool,
            range,
            InvocationSourceScope::All,
            Some(2),
        )
        .await
        .expect("query global in-flight records");
        let account = query_in_flight_invocation_aggregate_records_from_live_range_for_account(
            &pool,
            range,
            InvocationSourceScope::All,
            Some(2),
            7,
        )
        .await
        .expect("query account in-flight records");

        assert_eq!(
            global.iter().map(|record| record.id).collect::<Vec<_>>(),
            [1]
        );
        assert_eq!(
            account.iter().map(|record| record.id).collect::<Vec<_>>(),
            [1]
        );
    }

    #[tokio::test]
    async fn in_flight_query_uses_final_attempt_ttft_and_phase() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, status TEXT, total_tokens INTEGER, cache_input_tokens INTEGER, cost REAL, error_message TEXT, payload TEXT, failure_kind TEXT, failure_class TEXT, source TEXT, t_total_ms REAL, t_req_read_ms REAL, t_req_parse_ms REAL, t_upstream_connect_ms REAL, t_upstream_ttfb_ms REAL, first_token_ms REAL, t_upstream_stream_ms REAL, t_resp_parse_ms REAL, t_persist_ms REAL)",
        )
        .execute(&pool)
        .await
        .expect("create invocations");
        sqlx::query(
            "CREATE TABLE pool_upstream_request_attempts (id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, attempt_index INTEGER NOT NULL, status TEXT, phase TEXT, stream_latency_ms REAL, first_byte_latency_ms REAL)",
        )
        .execute(&pool)
        .await
        .expect("create pool attempts");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, payload, source, first_token_ms, t_upstream_stream_ms) VALUES (1, 'retried', '2026-08-03 08:00:10', 'running', '{\"upstreamAccountId\":7}', 'proxy', 720.0, 400.0)",
        )
        .execute(&pool)
        .await
        .expect("insert retried invocation");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, attempt_index, status, phase, stream_latency_ms, first_byte_latency_ms) VALUES (1, 'retried', '2026-08-03 08:00:10', 1, 'success', 'streaming_response', 400.0, 120.0), (2, 'retried', '2026-08-03 08:00:10', 2, 'running', 'waiting_first_byte', NULL, NULL)",
        )
        .execute(&pool)
        .await
        .expect("insert retried attempts");

        let range = ExactUtcRange {
            start: Utc.with_ymd_and_hms(2026, 8, 3, 0, 0, 0).single().unwrap(),
            end: Utc.with_ymd_and_hms(2026, 8, 3, 0, 1, 0).single().unwrap(),
        };
        let records = query_in_flight_invocation_aggregate_records_from_live_range(
            &pool,
            range,
            InvocationSourceScope::All,
            Some(1),
        )
        .await
        .expect("query retried in-flight record");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].first_token_ms, None);
        assert_eq!(records[0].t_upstream_stream_ms, None);
        assert_eq!(
            records[0].live_phase.as_deref(),
            Some(INVOCATION_LIVE_PHASE_REQUESTING)
        );
    }
}
