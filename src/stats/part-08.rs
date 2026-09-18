pub(crate) struct AllTimeRollupTotals {
    totals: StatsTotals,
    live_tail_ids: HashSet<i64>,
}

pub(crate) async fn query_invocation_all_time_rollup_totals(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
) -> Result<AllTimeRollupTotals> {
    let non_success_cost_expr =
        if sqlite_table_has_column(pool, "invocation_rollup_hourly", "non_success_cost").await? {
            "COALESCE(SUM(non_success_cost), 0.0) AS non_success_cost"
        } else {
            "0.0 AS non_success_cost"
        };
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        r#"
        SELECT
            COALESCE(SUM(total_count), 0) AS total_count,
            COALESCE(SUM(success_count), 0) AS success_count,
            COALESCE(SUM(failure_count), 0) AS failure_count,
            COALESCE(SUM(total_cost), 0.0) AS total_cost,
            COALESCE(SUM(total_tokens), 0) AS total_tokens,
            {non_success_cost_expr}
        FROM invocation_rollup_hourly
        WHERE 1 = 1
        "#,
    ));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    let mut totals = StatsTotals::from(query.build_query_as::<StatsRow>().fetch_one(pool).await?);
    let live_progress_cursor =
        load_hourly_rollup_live_progress(pool, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let repair_live_cursor = load_hourly_rollup_live_progress(
        pool,
        INVOCATION_SUMMARY_ROLLUP_REPAIR_MARKER_LIVE_CURSOR_DATASET,
    )
    .await?;
    let tail_cursor = live_progress_cursor.max(repair_live_cursor);
    // Without a durable live cursor, the rollup may already include an unknown
    // prefix. Do not add a raw tail and risk double-counting; retention advances
    // this cursor atomically only after it materializes a contiguous prefix.
    if tail_cursor <= 0 {
        return Ok(AllTimeRollupTotals {
            totals,
            live_tail_ids: HashSet::new(),
        });
    }

    let tail_query = match source_scope {
        InvocationSourceScope::ProxyOnly => format!(
            "SELECT {} FROM codex_invocations WHERE id > ?1 AND source = ?2",
            stats_success_failure_select_sql()
        ),
        InvocationSourceScope::All => format!(
            "SELECT {} FROM codex_invocations WHERE id > ?1",
            stats_success_failure_select_sql()
        ),
    };
    let tail = match source_scope {
        InvocationSourceScope::ProxyOnly => {
            sqlx::query_as::<_, StatsRow>(&tail_query)
                .bind(tail_cursor)
                .bind(SOURCE_PROXY)
                .fetch_one(pool)
                .await?
        }
        InvocationSourceScope::All => {
            sqlx::query_as::<_, StatsRow>(&tail_query)
                .bind(tail_cursor)
                .fetch_one(pool)
                .await?
        }
    };
    totals = totals.add(StatsTotals::from(tail));
    let live_tail_ids = load_live_invocation_ids_after_id(pool, source_scope, tail_cursor).await?;
    Ok(AllTimeRollupTotals {
        totals,
        live_tail_ids,
    })
}

pub(crate) async fn query_invocation_totals(
    pool: &Pool<Sqlite>,
    filter: StatsFilter,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals> {
    if matches!(filter, StatsFilter::All) {
        if load_completed_invocation_archive_paths(pool)
            .await?
            .is_empty()
        {
            return Ok(StatsTotals::from(
                query_stats_row(pool, StatsFilter::All, source_scope).await?,
            ));
        }

        // Read paths must stay query-only even when historical summary repair is still pending.
        // Background startup / follow-up maintenance is responsible for rebuilding stale archived
        // hourly rollups and summary replay markers; requests reuse the current materialized
        // rollups plus any still-unmaterialized archive batches instead of writing through here.
        let all_time = query_invocation_all_time_rollup_totals(pool, source_scope).await?;
        return Ok(all_time.totals.add(
            query_unmaterialized_invocation_archive_totals(
                pool,
                source_scope,
                None,
                Some(&all_time.live_tail_ids),
            )
            .await?,
        ));
    }

    Ok(StatsTotals::from(
        query_stats_row(pool, filter, source_scope).await?,
    ))
}

/// Aggregate the bounded live tail which follows the durable hourly-rollup cursor. This keeps
/// all-time projection hydration exact when those rows are older than the retained exact horizon,
/// without materializing the rows themselves into the canonical projection.
pub(crate) async fn query_live_invocation_totals_after_id(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    start_after_id: i64,
    upper_bound_id: i64,
) -> Result<StatsTotals> {
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    query.push(stats_success_failure_select_sql());
    query
        .push(" FROM codex_invocations WHERE id > ")
        .push_bind(start_after_id)
        .push(" AND id <= ")
        .push_bind(upper_bound_id);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    let row = query.build_query_as::<StatsRow>().fetch_one(pool).await?;
    Ok(StatsTotals::from(row))
}

pub(crate) async fn query_invocation_hourly_rollup_range(
    pool: &Pool<Sqlite>,
    range_start_epoch: i64,
    range_end_epoch: i64,
    source_scope: InvocationSourceScope,
) -> Result<Vec<InvocationHourlyRollupRecord>> {
    let columns = invocation_hourly_rollup_column_sql(pool).await?;
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
        input_tokens = columns.input_tokens,
        output_tokens = columns.output_tokens,
        cache_input_tokens = columns.cache_input_tokens,
        reasoning_tokens = columns.reasoning_tokens,
        non_success_cost = columns.non_success_cost,
        total_latency_sample_count = columns.total_latency_sample_count,
        total_latency_sum_ms = columns.total_latency_sum_ms,
        first_token_sample_count = columns.first_token_sample_count,
        first_token_sum_ms = columns.first_token_sum_ms,
        first_token_max_ms = columns.first_token_max_ms,
        first_token_histogram = columns.first_token_histogram,
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
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn sqlite_table_has_column(
    pool: &Pool<Sqlite>,
    table_name: &str,
    column_name: &str,
) -> Result<bool> {
    let escaped_table_name = table_name.replace('\'', "''");
    let pragma = format!("PRAGMA table_info('{escaped_table_name}')");
    let rows = sqlx::query(&pragma).fetch_all(pool).await?;
    Ok(rows.into_iter().any(|row| {
        row.try_get::<String, _>("name")
            .is_ok_and(|name| name == column_name)
    }))
}

pub(crate) async fn query_invocation_failure_hourly_rollup_range(
    pool: &Pool<Sqlite>,
    range_start_epoch: i64,
    range_end_epoch: i64,
    source_scope: InvocationSourceScope,
) -> Result<Vec<InvocationFailureHourlyRollupRecord>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            failure_class,
            is_actionable,
            error_category,
            SUM(failure_count) AS failure_count
        FROM invocation_failure_rollup_hourly
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
    query.push(" GROUP BY failure_class, is_actionable, error_category");

    query
        .build_query_as::<InvocationFailureHourlyRollupRecord>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_proxy_perf_stage_hourly_rollup_range(
    pool: &Pool<Sqlite>,
    range_start_epoch: i64,
    range_end_epoch: i64,
) -> Result<Vec<ProxyPerfStageHourlyRollupRecord>> {
    sqlx::query_as::<_, ProxyPerfStageHourlyRollupRecord>(
        r#"
        SELECT
            bucket_start_epoch,
            stage,
            sample_count,
            sum_ms,
            max_ms,
            histogram
        FROM proxy_perf_stage_hourly
        WHERE bucket_start_epoch >= ?1
          AND bucket_start_epoch < ?2
        ORDER BY stage ASC, bucket_start_epoch ASC
        "#,
    )
    .bind(range_start_epoch)
    .bind(range_end_epoch)
    .fetch_all(pool)
    .await
    .map_err(Into::into)
}

pub(crate) async fn query_combined_totals(
    pool: &Pool<Sqlite>,
    filter: StatsFilter,
    source_scope: InvocationSourceScope,
) -> Result<StatsTotals> {
    query_invocation_totals(pool, filter, source_scope).await
}

pub(crate) async fn resolve_default_source_scope(
    _pool: &Pool<Sqlite>,
) -> Result<InvocationSourceScope> {
    Ok(InvocationSourceScope::All)
}

#[derive(Debug)]
pub(crate) enum ApiError {
    BadRequest(anyhow::Error),
    Unavailable(anyhow::Error),
    Internal(anyhow::Error),
}

impl ApiError {
    pub(crate) fn bad_request<E>(err: E) -> Self
    where
        E: Into<anyhow::Error>,
    {
        Self::BadRequest(err.into())
    }

    pub(crate) fn unavailable<E>(err: E) -> Self
    where
        E: Into<anyhow::Error>,
    {
        Self::Unavailable(err.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, err) = match self {
            ApiError::BadRequest(err) => (StatusCode::BAD_REQUEST, err),
            ApiError::Unavailable(err) => (StatusCode::SERVICE_UNAVAILABLE, err),
            ApiError::Internal(err) => (StatusCode::INTERNAL_SERVER_ERROR, err),
        };
        let message = format!("{err}");
        (status, message).into_response()
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self::Internal(err.into())
    }
}

// --- ISO8601 UTC helpers and serializers ---
pub(crate) fn format_utc_iso(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub(crate) fn format_utc_iso_millis(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub(crate) fn format_utc_iso_precise(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

pub(crate) fn parse_to_utc_datetime(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f") {
        if let Some(loc) = Shanghai.from_local_datetime(&naive).single() {
            return Some(loc.with_timezone(&Utc));
        }
        return Some(Utc.from_utc_datetime(&naive));
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        if let Some(loc) = Shanghai.from_local_datetime(&naive).single() {
            return Some(loc.with_timezone(&Utc));
        }
        return Some(Utc.from_utc_datetime(&naive));
    }
    None
}

#[allow(clippy::ptr_arg)]
pub(crate) fn serialize_local_naive_to_utc_iso<S>(
    value: &String,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let iso = parse_to_utc_datetime(value)
        .map(format_utc_iso)
        .unwrap_or_else(|| value.clone());
    serializer.serialize_str(&iso)
}

#[allow(clippy::ptr_arg)]
pub(crate) fn serialize_local_or_utc_to_utc_iso<S>(
    value: &String,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serialize_local_naive_to_utc_iso(value, serializer)
}

#[allow(clippy::ptr_arg)]
pub(crate) fn serialize_opt_local_or_utc_to_utc_iso<S>(
    value: &Option<String>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match value {
        Some(s) => serialize_local_naive_to_utc_iso(s, serializer),
        None => serializer.serialize_none(),
    }
}
