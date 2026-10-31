use super::*;

const ANALYTICS_PROJECTION_TIME_ZONE: &str = "Asia/Shanghai";
const ANALYTICS_PROJECTION_MAX_SERIES_KEYS: usize = 100;

#[derive(Debug, Clone)]
pub(crate) struct AnalyticsFactWindow {
    pub(crate) snapshot_id: i64,
    pub(crate) facts: Vec<AnalyticsFact>,
}

#[derive(Debug, FromRow)]
struct AnalyticsFactRow {
    fact_version: i16,
    source: String,
    source_row_id: i64,
    invoke_id: String,
    occurred_at: String,
    occurred_at_epoch_ms: i64,
    stats_date: String,
    model: Option<String>,
    request_model: Option<String>,
    response_model: Option<String>,
    reasoning_effort: Option<String>,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<String>,
    status: Option<String>,
    success_like: bool,
    terminal: bool,
    failure_kind: Option<String>,
    failure_class: Option<String>,
    is_actionable: Option<bool>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    reasoning_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
    cost_input: Option<f64>,
    cost_cache_write: Option<f64>,
    cost_cache_read: Option<f64>,
    cost_output: Option<f64>,
    cost_reasoning: Option<f64>,
    t_total_ms: Option<f64>,
    t_req_read_ms: Option<f64>,
    t_req_parse_ms: Option<f64>,
    t_upstream_connect_ms: Option<f64>,
    t_upstream_ttfb_ms: Option<f64>,
    t_upstream_stream_ms: Option<f64>,
    first_token_ms: Option<f64>,
    t_resp_parse_ms: Option<f64>,
}

impl From<AnalyticsFactRow> for AnalyticsFact {
    fn from(row: AnalyticsFactRow) -> Self {
        Self {
            fact_version: row.fact_version,
            source: row.source,
            source_row_id: row.source_row_id,
            invoke_id: row.invoke_id,
            occurred_at: row.occurred_at,
            occurred_at_epoch_ms: row.occurred_at_epoch_ms,
            stats_date: row.stats_date,
            model: row.model,
            request_model: row.request_model,
            response_model: row.response_model,
            reasoning_effort: row.reasoning_effort,
            upstream_account_id: row.upstream_account_id,
            upstream_account_name: row.upstream_account_name,
            status: row.status,
            success_like: row.success_like,
            terminal: row.terminal,
            failure_kind: row.failure_kind,
            failure_class: row.failure_class,
            is_actionable: row.is_actionable,
            input_tokens: row.input_tokens,
            output_tokens: row.output_tokens,
            cache_input_tokens: row.cache_input_tokens,
            reasoning_tokens: row.reasoning_tokens,
            total_tokens: row.total_tokens,
            cost: row.cost,
            cost_input: row.cost_input,
            cost_cache_write: row.cost_cache_write,
            cost_cache_read: row.cost_cache_read,
            cost_output: row.cost_output,
            cost_reasoning: row.cost_reasoning,
            t_total_ms: row.t_total_ms,
            t_req_read_ms: row.t_req_read_ms,
            t_req_parse_ms: row.t_req_parse_ms,
            t_upstream_connect_ms: row.t_upstream_connect_ms,
            t_upstream_ttfb_ms: row.t_upstream_ttfb_ms,
            t_upstream_stream_ms: row.t_upstream_stream_ms,
            first_token_ms: row.first_token_ms,
            t_resp_parse_ms: row.t_resp_parse_ms,
        }
    }
}

fn normalized_fact_value(value: Option<&str>) -> &str {
    value.map(str::trim).unwrap_or_default()
}

fn failure_class_is_failure(value: Option<&str>) -> bool {
    matches!(
        normalized_fact_value(value).to_ascii_lowercase().as_str(),
        "service_failure" | "client_failure" | "client_abort"
    )
}

fn failure_kind_is_present(value: Option<&str>) -> bool {
    let value = normalized_fact_value(value);
    !value.is_empty() && !value.eq_ignore_ascii_case("none")
}

pub(crate) fn analytics_status_is_success_like(
    status: Option<&str>,
    failure_kind: Option<&str>,
    failure_class: Option<&str>,
) -> bool {
    if failure_class_is_failure(failure_class) {
        return false;
    }
    let status = normalized_fact_value(status).to_ascii_lowercase();
    let failure_kind = normalized_fact_value(failure_kind).to_ascii_lowercase();
    match status.as_str() {
        "success" | "succeeded" | "completed" | "ok" => failure_kind.is_empty(),
        "warning_success" => failure_kind.is_empty() || failure_kind == "downstream_closed",
        "http_200" => failure_kind.is_empty(),
        _ => false,
    }
}

pub(crate) fn analytics_fact_is_success_like(fact: &AnalyticsFact) -> bool {
    if failure_class_is_failure(fact.failure_class.as_deref()) {
        return false;
    }
    if failure_kind_is_present(fact.failure_kind.as_deref())
        && !fact.status.as_deref().is_some_and(|status| {
            normalized_fact_value(Some(status)).eq_ignore_ascii_case("warning_success")
                && normalized_fact_value(fact.failure_kind.as_deref())
                    .eq_ignore_ascii_case("downstream_closed")
        })
    {
        return false;
    }
    fact.success_like
        || analytics_status_is_success_like(
            fact.status.as_deref(),
            fact.failure_kind.as_deref(),
            fact.failure_class.as_deref(),
        )
}

pub(crate) fn analytics_fact_is_failure(fact: &AnalyticsFact) -> bool {
    fact.terminal
        && !analytics_fact_is_success_like(fact)
        && (failure_class_is_failure(fact.failure_class.as_deref())
            || fact
                .status
                .as_deref()
                .is_some_and(|status| !normalized_fact_value(Some(status)).is_empty())
            || failure_kind_is_present(fact.failure_kind.as_deref()))
}

const ANALYTICS_FACT_SELECT: &str = r#"
    SELECT fact_version, source, source_row_id, invoke_id,
           to_char(occurred_at AT TIME ZONE 'UTC',
                   'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') AS occurred_at,
           occurred_at_epoch_ms,
           stats_date::text AS stats_date, model, request_model, response_model,
           reasoning_effort, upstream_account_id, upstream_account_name, status,
           success_like, terminal, failure_kind, failure_class, is_actionable,
           input_tokens, output_tokens, cache_input_tokens, reasoning_tokens,
           total_tokens, cost::double precision AS cost,
           cost_input::double precision AS cost_input,
           cost_cache_write::double precision AS cost_cache_write,
           cost_cache_read::double precision AS cost_cache_read,
           cost_output::double precision AS cost_output,
           cost_reasoning::double precision AS cost_reasoning,
           t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms,
           t_upstream_ttfb_ms, t_upstream_stream_ms, first_token_ms, t_resp_parse_ms
    FROM cvm_analytics.analytics_facts
    WHERE 1 = 1
"#;

pub(crate) async fn query_invocation_facts(
    runtime: &PgAnalyticsRuntime,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    range: Option<(DateTime<Utc>, DateTime<Utc>)>,
    limit: Option<i64>,
    newest_first: bool,
) -> AnalyticsReadResult<AnalyticsFactWindow> {
    let pool = runtime.require_read_pool().await?;
    let mut transaction = pool.begin().await.map_err(|error| {
        AnalyticsReadError::Internal(anyhow!(
            "failed to start PostgreSQL analytics fact read transaction: {error}"
        ))
    })?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *transaction)
        .await
        .map_err(|error| {
            AnalyticsReadError::Internal(anyhow!(
                "failed to establish PostgreSQL analytics fact read snapshot: {error}"
            ))
        })?;
    let snapshot_id = sqlx::query_scalar::<_, i64>(
        "SELECT consumer_checkpoint FROM cvm_analytics.analytics_replica_state WHERE id = 1",
    )
    .fetch_one(&mut *transaction)
    .await
    .map_err(|error| {
        AnalyticsReadError::Internal(anyhow!(
            "failed to load PostgreSQL analytics snapshot checkpoint: {error}"
        ))
    })?;

    let mut query = QueryBuilder::<sqlx::Postgres>::new(ANALYTICS_FACT_SELECT);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND upstream_account_id = ")
            .push_bind(upstream_account_id);
    }
    if let Some((start, end)) = range {
        query
            .push(" AND occurred_at_epoch_ms >= ")
            .push_bind(start.timestamp_millis())
            .push(" AND occurred_at_epoch_ms < ")
            .push_bind(end.timestamp_millis());
    }
    query.push(" ORDER BY occurred_at_epoch_ms ");
    query.push(if newest_first { "DESC" } else { "ASC" });
    query.push(", source_row_id ");
    query.push(if newest_first { "DESC" } else { "ASC" });
    if let Some(limit) = limit.filter(|limit| *limit > 0) {
        query.push(" LIMIT ").push_bind(limit);
    }
    let facts = query
        .build_query_as::<AnalyticsFactRow>()
        .fetch_all(&mut *transaction)
        .await
        .map_err(|error| {
            AnalyticsReadError::Internal(anyhow!(
                "failed to query PostgreSQL analytics invocation facts: {error}"
            ))
        })?
        .into_iter()
        .map(AnalyticsFact::from)
        .collect();
    Ok(AnalyticsFactWindow { snapshot_id, facts })
}

pub(crate) async fn rebuild_analytics_projection(pool: &sqlx::PgPool) -> Result<()> {
    let mut transaction = pool
        .begin()
        .await
        .context("failed to start PostgreSQL analytics projection rebuild")?;
    rebuild_analytics_projection_on_connection(&mut transaction).await?;
    transaction
        .commit()
        .await
        .context("failed to commit PostgreSQL analytics projection rebuild")?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct AnalyticsProjectionBucket {
    pub(super) stats_date: String,
    pub(super) occurred_at_epoch_ms: i64,
}

pub(super) fn projection_bucket_for_fact(fact: &AnalyticsFact) -> AnalyticsProjectionBucket {
    AnalyticsProjectionBucket {
        stats_date: fact.stats_date.clone(),
        occurred_at_epoch_ms: fact.occurred_at_epoch_ms,
    }
}

pub(super) async fn collect_projection_buckets_for_fact(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    source: &str,
    source_row_id: i64,
    buckets: &mut std::collections::HashSet<AnalyticsProjectionBucket>,
) -> Result<()> {
    let rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT stats_date::text, occurred_at_epoch_ms FROM cvm_analytics.analytics_facts WHERE source = $1 AND source_row_id = $2",
    )
    .bind(source)
    .bind(source_row_id)
    .fetch_all(&mut **transaction)
    .await
    .context("failed to load existing PostgreSQL analytics projection bucket")?;
    buckets.extend(rows.into_iter().map(|(stats_date, occurred_at_epoch_ms)| {
        AnalyticsProjectionBucket {
            stats_date,
            occurred_at_epoch_ms,
        }
    }));
    Ok(())
}

pub(super) async fn collect_projection_buckets_for_invoke_id(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    invoke_id: &str,
    buckets: &mut std::collections::HashSet<AnalyticsProjectionBucket>,
) -> Result<()> {
    let rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT stats_date::text, occurred_at_epoch_ms FROM cvm_analytics.analytics_facts WHERE source = 'codex_invocations' AND invoke_id = $1",
    )
    .bind(invoke_id)
    .fetch_all(&mut **transaction)
    .await
    .context("failed to load PostgreSQL analytics buckets for an invocation account refresh")?;
    buckets.extend(rows.into_iter().map(|(stats_date, occurred_at_epoch_ms)| {
        AnalyticsProjectionBucket {
            stats_date,
            occurred_at_epoch_ms,
        }
    }));
    Ok(())
}

pub(super) async fn refresh_analytics_projection_on_connection(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    buckets: &std::collections::HashSet<AnalyticsProjectionBucket>,
) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TEMP TABLE pg_analytics_projection_affected_buckets (
            stats_date DATE NOT NULL,
            bucket_start TIMESTAMPTZ NOT NULL,
            PRIMARY KEY (stats_date, bucket_start)
        ) ON COMMIT DROP
        "#,
    )
    .execute(&mut **transaction)
    .await
    .context("failed to create PostgreSQL analytics projection bucket set")?;
    for bucket in buckets {
        sqlx::query(
            r#"
            INSERT INTO pg_analytics_projection_affected_buckets (stats_date, bucket_start)
            VALUES ($1::date, to_timestamp(floor($2::double precision / 3600000.0) * 3600))
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(&bucket.stats_date)
        .bind(bucket.occurred_at_epoch_ms)
        .execute(&mut **transaction)
        .await
        .context("failed to record a PostgreSQL analytics projection bucket")?;
    }
    sqlx::query(
        r#"
        DELETE FROM cvm_analytics.analytics_daily_rollups AS rollup
        USING pg_analytics_projection_affected_buckets AS affected
        WHERE rollup.stats_date = affected.stats_date
        "#,
    )
    .execute(&mut **transaction)
    .await
    .context("failed to clear affected PostgreSQL daily analytics rollups")?;
    sqlx::query(
        r#"
        DELETE FROM cvm_analytics.analytics_hourly_rollups AS rollup
        USING pg_analytics_projection_affected_buckets AS affected
        WHERE rollup.bucket_start = affected.bucket_start
        "#,
    )
    .execute(&mut **transaction)
    .await
    .context("failed to clear affected PostgreSQL hourly analytics rollups")?;
    for (table, bucket_column) in [
        ("analytics_daily_rollups", "stats_date"),
        ("analytics_hourly_rollups", "bucket_start"),
    ] {
        let statement = analytics_rollup_insert_sql_with_filter(table, bucket_column, true);
        sqlx::query(&statement)
            .execute(&mut **transaction)
            .await
            .with_context(|| format!("failed to refresh affected PostgreSQL {table}"))?;
    }
    update_projection_progress(transaction).await
}

pub(super) async fn rebuild_analytics_projection_on_connection(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<()> {
    sqlx::query("DELETE FROM cvm_analytics.analytics_daily_rollups")
        .execute(&mut **transaction)
        .await
        .context("failed to clear PostgreSQL daily analytics rollups")?;
    sqlx::query("DELETE FROM cvm_analytics.analytics_hourly_rollups")
        .execute(&mut **transaction)
        .await
        .context("failed to clear PostgreSQL hourly analytics rollups")?;

    for (table, bucket_column) in [
        ("analytics_daily_rollups", "stats_date"),
        ("analytics_hourly_rollups", "bucket_start"),
    ] {
        let statement = analytics_rollup_insert_sql(table, bucket_column);
        sqlx::query(&statement)
            .execute(&mut **transaction)
            .await
            .with_context(|| format!("failed to rebuild PostgreSQL {table}"))?;
    }
    update_projection_progress(transaction).await
}

async fn update_projection_progress(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE cvm_analytics.analytics_state
        SET status = 'preparing',
            statistics_start_date = (
                SELECT MIN(stats_date) FROM cvm_analytics.analytics_facts
            ),
            processed_rows = (
                SELECT COUNT(*) FROM cvm_analytics.analytics_facts
            ),
            total_rows = (
                SELECT COUNT(*) FROM cvm_analytics.analytics_facts
            ),
            last_error_code = NULL,
            updated_at = NOW()
        WHERE id = 1
        "#,
    )
    .execute(&mut **transaction)
    .await
    .context("failed to update PostgreSQL analytics projection progress")?;
    Ok(())
}

fn analytics_rollup_insert_sql(table: &str, bucket_column: &str) -> String {
    analytics_rollup_insert_sql_with_filter(table, bucket_column, false)
}

fn analytics_rollup_insert_sql_with_filter(
    table: &str,
    bucket_column: &str,
    affected_only: bool,
) -> String {
    let bucket_expression = format!("aggregated.{bucket_column}");
    let aggregation_bucket_columns = if bucket_column == "stats_date" {
        "stats_date"
    } else {
        "stats_date, bucket_start"
    };
    let affected_filter = if affected_only {
        if bucket_column == "stats_date" {
            "              AND fact.stats_date IN (SELECT stats_date FROM pg_analytics_projection_affected_buckets)"
        } else {
            "              AND to_timestamp(floor(fact.occurred_at_epoch_ms / 3600000.0) * 3600) IN (SELECT bucket_start FROM pg_analytics_projection_affected_buckets)"
        }
    } else {
        ""
    };
    format!(
        r#"
        WITH base AS (
            SELECT fact.stats_date,
                   to_timestamp(floor(fact.occurred_at_epoch_ms / 3600000.0) * 3600) AS bucket_start,
                   COALESCE(NULLIF(BTRIM(fact.response_model), ''), NULLIF(BTRIM(fact.model), '')) AS model,
                   fact.reasoning_effort, fact.upstream_account_id, fact.upstream_account_name,
                   CASE
                       WHEN LOWER(TRIM(COALESCE(fact.failure_class, ''))) IN
                            ('service_failure', 'client_failure', 'client_abort') THEN FALSE
                       WHEN fact.success_like
                            AND LOWER(TRIM(COALESCE(fact.failure_kind, ''))) IN ('', 'none') THEN TRUE
                       WHEN LOWER(TRIM(COALESCE(fact.status, ''))) IN
                            ('success', 'succeeded', 'completed', 'ok')
                            AND LOWER(TRIM(COALESCE(fact.failure_kind, ''))) IN ('', 'none') THEN TRUE
                       WHEN LOWER(TRIM(COALESCE(fact.status, ''))) = 'warning_success'
                            AND LOWER(TRIM(COALESCE(fact.failure_kind, ''))) IN
                                ('', 'none', 'downstream_closed') THEN TRUE
                       WHEN LOWER(TRIM(COALESCE(fact.status, ''))) = 'http_200'
                            AND LOWER(TRIM(COALESCE(fact.failure_kind, ''))) IN ('', 'none') THEN TRUE
                       ELSE FALSE
                   END AS success_like,
                   fact.total_tokens, fact.cost, fact.t_total_ms, fact.output_tokens,
                   fact.t_upstream_stream_ms, fact.t_req_read_ms, fact.t_req_parse_ms,
                   fact.t_upstream_connect_ms, fact.t_upstream_ttfb_ms
            FROM cvm_analytics.analytics_facts AS fact
            WHERE fact.terminal
{affected_filter}
        ), dimensional AS (
            SELECT stats_date, bucket_start, 'overall'::text AS dimension,
                   'overall'::text AS series_key, '全部'::text AS display_name,
                   ''::text AS reasoning_effort, success_like, total_tokens, cost,
                   t_total_ms, output_tokens, t_upstream_stream_ms, t_req_read_ms,
                   t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms
            FROM base
            UNION ALL
            SELECT stats_date, bucket_start, 'model'::text,
                   'model:' || COALESCE(NULLIF(BTRIM(model), ''), '未知模型') ||
                       ':reasoning:' || COALESCE(NULLIF(BTRIM(reasoning_effort), ''), '未指定'),
                   COALESCE(NULLIF(BTRIM(model), ''), '未知模型'),
                   COALESCE(NULLIF(BTRIM(reasoning_effort), ''), '未指定'),
                   success_like, total_tokens, cost, t_total_ms, output_tokens,
                   t_upstream_stream_ms, t_req_read_ms, t_req_parse_ms,
                   t_upstream_connect_ms, t_upstream_ttfb_ms
            FROM base
            UNION ALL
            SELECT stats_date, bucket_start, 'upstream'::text,
                   CASE WHEN upstream_account_id IS NULL THEN 'unknown'
                        ELSE 'account:' || upstream_account_id::text END,
                   CASE WHEN upstream_account_id IS NULL THEN '未知账号'
                        ELSE COALESCE(NULLIF(BTRIM(upstream_account_name), ''),
                                      '账号 ' || upstream_account_id::text) END,
                   ''::text, success_like, total_tokens, cost, t_total_ms,
                   output_tokens, t_upstream_stream_ms, t_req_read_ms, t_req_parse_ms,
                   t_upstream_connect_ms, t_upstream_ttfb_ms
            FROM base
        ), aggregated AS (
            SELECT {aggregation_bucket_columns}, dimension, series_key,
                   MAX(display_name) AS display_name,
                   MAX(reasoning_effort) AS reasoning_effort, COUNT(*)::bigint AS calls,
                   COALESCE(SUM(total_tokens) FILTER (WHERE total_tokens IS NOT NULL), 0)::bigint AS token_total,
                   COUNT(total_tokens)::bigint AS token_samples,
                   COALESCE(SUM(cost) FILTER (WHERE cost IS NOT NULL), 0)::numeric AS cost_total,
                   COUNT(cost)::bigint AS cost_samples,
                   COALESCE(SUM(t_total_ms) FILTER (
                       WHERE success_like AND t_total_ms IS NOT NULL AND t_total_ms > 0
                   ), 0)::double precision AS usage_time_ms,
                   COUNT(t_total_ms) FILTER (
                       WHERE success_like AND t_total_ms IS NOT NULL AND t_total_ms > 0
                   )::bigint AS usage_time_samples,
                   COALESCE(SUM(t_total_ms) FILTER (
                       WHERE success_like AND t_total_ms IS NOT NULL AND t_total_ms > 0
                   ), 0)::double precision AS wall_time_ms,
                   COUNT(t_total_ms) FILTER (
                       WHERE success_like AND t_total_ms IS NOT NULL AND t_total_ms > 0
                   )::bigint AS wall_time_samples,
                   COALESCE(SUM(output_tokens) FILTER (
                       WHERE success_like AND output_tokens IS NOT NULL
                   ), 0)::bigint AS output_tokens_total,
                   COALESCE(SUM(t_upstream_stream_ms) FILTER (
                       WHERE success_like AND t_upstream_stream_ms IS NOT NULL
                         AND t_upstream_stream_ms > 0
                   ), 0)::double precision AS stream_duration_ms,
                   COUNT(*) FILTER (
                       WHERE success_like AND output_tokens IS NOT NULL
                         AND t_upstream_stream_ms IS NOT NULL AND t_upstream_stream_ms > 0
                   )::bigint AS output_speed_samples,
                   COALESCE(SUM(
                       t_req_read_ms + t_req_parse_ms + t_upstream_connect_ms + t_upstream_ttfb_ms
                   ) FILTER (
                       WHERE success_like AND t_req_read_ms IS NOT NULL
                         AND t_req_parse_ms IS NOT NULL AND t_upstream_connect_ms IS NOT NULL
                         AND t_upstream_ttfb_ms IS NOT NULL
                   ), 0)::double precision AS first_byte_sum_ms,
                   COUNT(*) FILTER (
                       WHERE success_like AND t_req_read_ms IS NOT NULL
                         AND t_req_parse_ms IS NOT NULL AND t_upstream_connect_ms IS NOT NULL
                         AND t_upstream_ttfb_ms IS NOT NULL
                   )::bigint AS first_byte_samples,
                   COALESCE(SUM(t_upstream_stream_ms) FILTER (
                       WHERE success_like AND t_upstream_stream_ms IS NOT NULL
                         AND t_upstream_stream_ms >= 0
                   ), 0)::double precision AS response_sum_ms,
                   COUNT(t_upstream_stream_ms) FILTER (
                       WHERE success_like AND t_upstream_stream_ms IS NOT NULL
                         AND t_upstream_stream_ms >= 0
                   )::bigint AS response_samples
            FROM dimensional
            GROUP BY {aggregation_bucket_columns}, dimension, series_key
        )
        INSERT INTO cvm_analytics.{table} (
            {bucket_column}, dimension, series_key, display_name, reasoning_effort,
            calls, token_total, token_samples, cost_total, cost_samples,
            usage_time_ms, usage_time_samples, wall_time_ms, wall_time_samples,
            output_tokens_total, stream_duration_ms, output_speed_samples,
            first_byte_sum_ms, first_byte_samples, response_sum_ms, response_samples
        )
        SELECT {bucket_expression}, dimension, series_key, display_name, reasoning_effort,
               calls, token_total, token_samples, cost_total, cost_samples,
               usage_time_ms, usage_time_samples, wall_time_ms, wall_time_samples,
               output_tokens_total, stream_duration_ms, output_speed_samples,
               first_byte_sum_ms, first_byte_samples, response_sum_ms, response_samples
        FROM aggregated
        "#,
    )
}

#[derive(Debug, FromRow)]
struct AnalyticsProjectionStateRow {
    status: String,
    statistics_start_date: Option<String>,
    processed_rows: i64,
    total_rows: i64,
    last_error_code: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
struct AnalyticsRollupRow {
    bucket_or_date: String,
    dimension: String,
    series_key: String,
    display_name: String,
    reasoning_effort: String,
    calls: i64,
    token_total: i64,
    token_samples: i64,
    cost_total: f64,
    cost_samples: i64,
    usage_time_ms: f64,
    usage_time_samples: i64,
    wall_time_ms: f64,
    wall_time_samples: i64,
    output_tokens_total: i64,
    stream_duration_ms: f64,
    output_speed_samples: i64,
    first_byte_sum_ms: f64,
    first_byte_samples: i64,
    response_sum_ms: f64,
    response_samples: i64,
}

async fn load_projection_state(
    pool: &sqlx::PgPool,
) -> AnalyticsReadResult<AnalyticsProjectionState> {
    let row = sqlx::query_as::<_, AnalyticsProjectionStateRow>(
        r#"
        SELECT status, statistics_start_date::text AS statistics_start_date,
               processed_rows, total_rows, last_error_code
        FROM cvm_analytics.analytics_state
        WHERE id = 1
        "#,
    )
    .fetch_optional(pool)
    .await
    .map_err(|error| {
        AnalyticsReadError::Internal(anyhow!(
            "failed to load PostgreSQL analytics projection state: {error}"
        ))
    })?
    .ok_or_else(|| {
        AnalyticsReadError::Unavailable(AnalyticsUnavailable::new(
            AnalyticsBackend::Postgres,
            "analytics replica projection state is missing",
        ))
    })?;
    Ok(AnalyticsProjectionState {
        status: row.status,
        statistics_start_date: row.statistics_start_date,
        processed_rows: row.processed_rows,
        total_rows: row.total_rows,
        last_error_code: row.last_error_code,
    })
}

async fn load_daily_rollups(
    pool: &sqlx::PgPool,
    start: NaiveDate,
    end: NaiveDate,
    dimension: Option<AnalyticsDimension>,
    keys: &[String],
) -> AnalyticsReadResult<Vec<AnalyticsRollupRow>> {
    let mut query = QueryBuilder::<sqlx::Postgres>::new(
        r#"
        SELECT stats_date::text AS bucket_or_date, dimension, series_key, display_name,
               reasoning_effort, calls, token_total, token_samples,
               cost_total::double precision AS cost_total, cost_samples,
               usage_time_ms, usage_time_samples, wall_time_ms, wall_time_samples,
               output_tokens_total, stream_duration_ms, output_speed_samples,
               first_byte_sum_ms, first_byte_samples, response_sum_ms, response_samples
        FROM cvm_analytics.analytics_daily_rollups
        WHERE stats_date >= "#,
    );
    query
        .push_bind(start.to_string())
        .push("::date AND stats_date < ")
        .push_bind(end.to_string())
        .push("::date");
    if let Some(dimension) = dimension {
        query
            .push(" AND dimension = ")
            .push_bind(dimension.as_str());
    }
    if !keys.is_empty() {
        query.push(" AND series_key IN (");
        let mut separated = query.separated(", ");
        for key in keys {
            separated.push_bind(key);
        }
        query.push(")");
    }
    query.push(" ORDER BY stats_date ASC, series_key ASC, reasoning_effort ASC");
    query
        .build_query_as::<AnalyticsRollupRow>()
        .fetch_all(pool)
        .await
        .map_err(|error| {
            AnalyticsReadError::Internal(anyhow!(
                "failed to query PostgreSQL analytics daily rollups: {error}"
            ))
        })
}

fn analytics_projection_date_range(range: AnalyticsRange) -> (NaiveDate, NaiveDate) {
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    let start = today
        .checked_sub_signed(ChronoDuration::days(range.days().saturating_sub(1)))
        .unwrap_or(today);
    (start, today.succ_opt().unwrap_or(today))
}

#[derive(Debug, Default)]
struct AnalyticsMetricsAccumulator {
    calls: i64,
    token_total: i64,
    token_samples: i64,
    cost_total: f64,
    cost_samples: i64,
    usage_time_ms: f64,
    usage_time_samples: i64,
    wall_time_ms: f64,
    wall_time_samples: i64,
    output_tokens_total: i64,
    stream_duration_ms: f64,
    output_speed_samples: i64,
    first_byte_sum_ms: f64,
    first_byte_samples: i64,
    response_sum_ms: f64,
    response_samples: i64,
}

impl AnalyticsMetricsAccumulator {
    fn add(&mut self, row: &AnalyticsRollupRow) {
        self.calls += row.calls;
        self.token_total += row.token_total;
        self.token_samples += row.token_samples;
        self.cost_total += row.cost_total;
        self.cost_samples += row.cost_samples;
        self.usage_time_ms += row.usage_time_ms;
        self.usage_time_samples += row.usage_time_samples;
        self.wall_time_ms += row.wall_time_ms;
        self.wall_time_samples += row.wall_time_samples;
        self.output_tokens_total += row.output_tokens_total;
        self.stream_duration_ms += row.stream_duration_ms;
        self.output_speed_samples += row.output_speed_samples;
        self.first_byte_sum_ms += row.first_byte_sum_ms;
        self.first_byte_samples += row.first_byte_samples;
        self.response_sum_ms += row.response_sum_ms;
        self.response_samples += row.response_samples;
    }

    fn metrics(&self) -> AnalyticsMetrics {
        AnalyticsMetrics {
            calls: self.calls,
            tokens: (self.token_samples > 0).then_some(self.token_total),
            token_samples: self.token_samples,
            cost: (self.cost_samples > 0).then_some(self.cost_total),
            cost_samples: self.cost_samples,
            usage_time_ms: (self.usage_time_samples > 0).then_some(self.usage_time_ms),
            usage_time_samples: self.usage_time_samples,
            wall_time_ms: (self.wall_time_samples > 0).then_some(self.wall_time_ms),
            wall_time_samples: self.wall_time_samples,
            output_speed_tokens_per_second: (self.output_speed_samples > 0
                && self.stream_duration_ms > 0.0)
                .then_some(self.output_tokens_total as f64 / (self.stream_duration_ms / 1000.0)),
            output_speed_samples: self.output_speed_samples,
            first_byte_ms: (self.first_byte_samples > 0)
                .then_some(self.first_byte_sum_ms / self.first_byte_samples as f64),
            first_byte_samples: self.first_byte_samples,
            response_ms: (self.response_samples > 0)
                .then_some(self.response_sum_ms / self.response_samples as f64),
            response_samples: self.response_samples,
        }
    }
}

fn metrics_from_rollup(row: &AnalyticsRollupRow) -> AnalyticsMetrics {
    let mut accumulator = AnalyticsMetricsAccumulator::default();
    accumulator.add(row);
    accumulator.metrics()
}

fn normalized_reasoning_effort(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_string())
}

fn series_summary_from_rows(
    rows: impl IntoIterator<Item = AnalyticsRollupRow>,
) -> Vec<AnalyticsSeriesSummary> {
    let mut grouped = BTreeMap::<(String, String), (String, AnalyticsMetricsAccumulator)>::new();
    for row in rows {
        let key = (row.series_key.clone(), row.reasoning_effort.clone());
        grouped
            .entry(key)
            .or_insert_with(|| {
                (
                    row.display_name.clone(),
                    AnalyticsMetricsAccumulator::default(),
                )
            })
            .1
            .add(&row);
    }
    grouped
        .into_iter()
        .map(
            |((series_key, reasoning_effort), (display_name, metrics))| AnalyticsSeriesSummary {
                series_key,
                display_name,
                reasoning_effort: normalized_reasoning_effort(&reasoning_effort),
                metrics: metrics.metrics(),
            },
        )
        .collect()
}

#[async_trait::async_trait]
impl AnalyticsReadStore for PgAnalyticsRuntime {
    async fn overview(
        &self,
        query: AnalyticsOverviewQuery,
    ) -> AnalyticsReadResult<AnalyticsOverview> {
        let pool = self.require_read_pool().await?;
        let state = load_projection_state(pool).await?;
        let (start, end) = analytics_projection_date_range(query.range);
        let rows = load_daily_rollups(pool, start, end, None, &[]).await?;
        let overall_rows = rows
            .iter()
            .filter(|row| row.dimension == AnalyticsDimension::Overall.as_str());
        let mut global = AnalyticsMetricsAccumulator::default();
        for row in overall_rows {
            global.add(row);
        }
        let daily = rows
            .iter()
            .filter(|row| row.dimension == AnalyticsDimension::Overall.as_str())
            .map(|row| AnalyticsDailyPoint {
                date: row.bucket_or_date.clone(),
                metrics: metrics_from_rollup(row),
            })
            .collect();
        let models = series_summary_from_rows(
            rows.iter()
                .filter(|row| row.dimension == AnalyticsDimension::Model.as_str())
                .cloned(),
        );
        let upstreams = series_summary_from_rows(
            rows.iter()
                .filter(|row| row.dimension == AnalyticsDimension::Upstream.as_str())
                .cloned(),
        );
        Ok(AnalyticsOverview {
            state,
            timezone: ANALYTICS_PROJECTION_TIME_ZONE.to_string(),
            range: query.range.as_str().to_string(),
            global: global.metrics(),
            daily,
            models,
            upstreams,
        })
    }

    async fn series(
        &self,
        query: AnalyticsSeriesQuery,
    ) -> AnalyticsReadResult<AnalyticsSeriesResponse> {
        if query.keys.len() > ANALYTICS_PROJECTION_MAX_SERIES_KEYS {
            return Err(AnalyticsReadError::InvalidQuery(format!(
                "analytics series key count exceeds {}",
                ANALYTICS_PROJECTION_MAX_SERIES_KEYS
            )));
        }
        let pool = self.require_read_pool().await?;
        let state = load_projection_state(pool).await?;
        let (start, end) = analytics_projection_date_range(query.range);
        let rows = load_daily_rollups(pool, start, end, Some(query.dimension), &query.keys).await?;
        let mut grouped =
            BTreeMap::<(String, String), (String, String, Vec<AnalyticsDailyPoint>)>::new();
        for row in rows {
            let key = (row.series_key.clone(), row.reasoning_effort.clone());
            let entry = grouped
                .entry(key)
                .or_insert_with(|| (row.series_key.clone(), row.display_name.clone(), Vec::new()));
            entry.2.push(AnalyticsDailyPoint {
                date: row.bucket_or_date.clone(),
                metrics: metrics_from_rollup(&row),
            });
        }
        let series = grouped
            .into_iter()
            .map(
                |((series_key, reasoning_effort), (key, display_name, points))| {
                    debug_assert_eq!(series_key, key);
                    AnalyticsSeries {
                        series_key,
                        display_name,
                        reasoning_effort: normalized_reasoning_effort(&reasoning_effort),
                        points,
                    }
                },
            )
            .collect();
        Ok(AnalyticsSeriesResponse {
            state,
            timezone: ANALYTICS_PROJECTION_TIME_ZONE.to_string(),
            range: query.range.as_str().to_string(),
            dimension: query.dimension,
            series,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_insert_keeps_daily_and_hourly_boundaries_distinct() {
        let daily = analytics_rollup_insert_sql("analytics_daily_rollups", "stats_date");
        let hourly = analytics_rollup_insert_sql("analytics_hourly_rollups", "bucket_start");
        let daily_affected =
            analytics_rollup_insert_sql_with_filter("analytics_daily_rollups", "stats_date", true);
        let hourly_affected = analytics_rollup_insert_sql_with_filter(
            "analytics_hourly_rollups",
            "bucket_start",
            true,
        );
        assert!(daily.contains("SELECT aggregated.stats_date"));
        assert!(hourly.contains("SELECT aggregated.bucket_start"));
        assert!(daily.contains("WHERE fact.terminal"));
        assert!(daily.contains("warning_success"));
        assert!(daily.contains("COALESCE(NULLIF(BTRIM(fact.response_model)"));
        assert!(daily.contains("GROUP BY stats_date, dimension"));
        assert!(!daily.contains("GROUP BY stats_date, bucket_start, dimension"));
        assert!(hourly.contains("GROUP BY stats_date, bucket_start, dimension"));
        assert!(daily.contains("MAX(display_name) AS display_name"));
        assert!(!daily.contains(
            "GROUP BY stats_date, dimension, series_key,\n                     display_name"
        ));
        assert!(!daily.contains("ON CONFLICT"));
        assert!(daily_affected.contains("fact.stats_date IN"));
        assert!(hourly_affected.contains("fact.occurred_at_epoch_ms"));
        assert!(daily_affected.contains("pg_analytics_projection_affected_buckets"));
    }

    #[test]
    fn fact_status_classification_handles_legacy_success_and_failure_metadata() {
        assert!(analytics_status_is_success_like(
            Some("warning_success"),
            Some("downstream_closed"),
            Some("none"),
        ));
        assert!(analytics_status_is_success_like(
            Some("http_200"),
            None,
            None
        ));
        assert!(!analytics_status_is_success_like(
            Some("completed"),
            Some("upstream_response_failed"),
            Some("none"),
        ));
    }

    #[test]
    fn metrics_keep_sample_presence_separate_from_zero_values() {
        let row = AnalyticsRollupRow {
            bucket_or_date: "2026-10-31".to_string(),
            dimension: "overall".to_string(),
            series_key: "overall".to_string(),
            display_name: "全部".to_string(),
            reasoning_effort: String::new(),
            calls: 2,
            token_total: 0,
            token_samples: 1,
            cost_total: 0.0,
            cost_samples: 1,
            usage_time_ms: 0.0,
            usage_time_samples: 1,
            wall_time_ms: 0.0,
            wall_time_samples: 1,
            output_tokens_total: 0,
            stream_duration_ms: 0.0,
            output_speed_samples: 0,
            first_byte_sum_ms: 0.0,
            first_byte_samples: 1,
            response_sum_ms: 0.0,
            response_samples: 1,
        };
        let metrics = metrics_from_rollup(&row);
        assert_eq!(metrics.tokens, Some(0));
        assert_eq!(metrics.cost, Some(0.0));
        assert_eq!(metrics.usage_time_ms, Some(0.0));
        assert_eq!(metrics.output_speed_tokens_per_second, None);
    }
}
