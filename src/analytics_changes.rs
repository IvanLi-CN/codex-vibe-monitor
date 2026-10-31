use super::*;

const ANALYTICS_CHANGE_EVENT_VERSION: i64 = 1;
const ANALYTICS_CHANGE_CAPACITY_DEFAULT: i64 = 10_000;
const ANALYTICS_CHANGE_FACT_MAX_BYTES: usize = 64 * 1024;
const ANALYTICS_CHANGE_ENTITY_KEY_MAX_BYTES: usize = 512;
const ANALYTICS_CHANGE_TEXT_MAX_CHARS: usize = 256;

// These tables are created after this migration on a fresh database, so only existing rows
// count as evidence that a legacy analytics projection may outlive the source journal.
const LEGACY_ANALYTICS_HISTORY_TABLES: &[&str] = &[
    "hourly_rollup_archive_replay",
    "invocation_rollup_daily",
    "invocation_rollup_hourly",
    "invocation_failure_rollup_hourly",
    "proxy_perf_stage_hourly",
    "prompt_cache_rollup_hourly",
    "prompt_cache_upstream_account_hourly",
    "upstream_account_usage_hourly",
    "upstream_account_usage_breakdown_hourly",
    "upstream_account_stats_hourly",
    "upstream_account_stats_minute",
    "long_term_usage_hourly",
    "long_term_usage_daily",
    "long_term_stats_repair_queue",
    "long_term_projection_dirty_buckets",
    "long_term_projection_bucket_state",
    "long_term_projection_daily_backups",
    "long_term_projection_date_publications",
    "long_term_projection_archive_compatibility",
    "long_term_projection_daily_backup_claims",
    "long_term_projection_intervals",
    "long_term_projection_interval_state",
    "long_term_projection_interval_suppressions",
    "long_term_projection_rebuild_members",
    "summary_source_change_journal",
    "summary_source_change_compaction_proof",
    "summary_all_time_coverage_checkpoint",
    "summary_all_time_projection_checkpoint",
    "summary_all_time_projection_account_checkpoint",
    "summary_all_time_projection_usage_checkpoint",
    "summary_live_tail_reconciliation_checkpoint",
    "summary_archive_snapshot",
    "summary_archive_snapshot_backfill_checkpoint",
    "summary_archive_snapshot_backfill_outcome",
    "summary_archive_snapshot_v2_proof",
    "summary_coverage_obligation",
    "summary_compacted_source_recovery_checkpoint",
    "summary_compacted_source_recovery_aggregate",
    "summary_compacted_source_recovery_usage",
    "summary_compacted_source_recovery_record",
    "hourly_rollup_archive_progress",
    "hourly_rollup_archive_repair_progress",
    "hourly_rollup_archive_repair_deferred",
    "hourly_rollup_repair_seen_invocation_ids",
    "hourly_rollup_repair_cleared_buckets",
];

const LEGACY_ANALYTICS_HISTORY_STATE_CHECKS: &[(&str, &str)] = &[
    (
        "long_term_projection_state",
        "cursor_row_id > 0 OR last_flush_at IS NOT NULL OR last_error IS NOT NULL",
    ),
    ("summary_source_change_cursor", "cursor > 0"),
];

#[derive(Debug, Clone, Copy, FromRow)]
struct AnalyticsChangeStateRow {
    generation: i64,
    coverage_revision: i64,
    retained_from_change_id: i64,
    capacity: i64,
    invalidation_pending: i64,
    historical_replay_required: i64,
}

#[derive(Debug, Clone, FromRow)]
struct ExistingAnalyticsChangeRow {
    change_id: i64,
    event_kind: String,
    generation: i64,
    coverage_revision: i64,
    fact_json: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AnalyticsChangeReceipt {
    pub(crate) change_id: i64,
    pub(crate) generation: i64,
    pub(crate) coverage_revision: i64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AnalyticsCoverageReceipt {
    pub(crate) generation: i64,
    pub(crate) coverage_revision: i64,
    pub(crate) covered_through_change_id: i64,
}

#[derive(Debug, FromRow)]
struct InvocationAnalyticsRow {
    id: i64,
    invoke_id: String,
    occurred_at: String,
    occurred_at_epoch_ms: i64,
    stats_date: String,
    source: String,
    model: Option<String>,
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
    status: Option<String>,
    failure_kind: Option<String>,
    failure_class: Option<String>,
    is_actionable: Option<i64>,
    payload: Option<String>,
    t_total_ms: Option<f64>,
    t_req_read_ms: Option<f64>,
    t_req_parse_ms: Option<f64>,
    t_upstream_connect_ms: Option<f64>,
    t_upstream_ttfb_ms: Option<f64>,
    t_upstream_stream_ms: Option<f64>,
    first_token_ms: Option<f64>,
    t_resp_parse_ms: Option<f64>,
}

#[derive(Debug, FromRow)]
struct AttemptAnalyticsRow {
    id: i64,
    attempt_public_id: Option<String>,
    invoke_id: String,
    occurred_at: String,
    endpoint: String,
    request_model: Option<String>,
    upstream_request_model: Option<String>,
    upstream_account_id: Option<i64>,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    status: String,
    phase: Option<String>,
    http_status: Option<i64>,
    downstream_http_status: Option<i64>,
    failure_kind: Option<String>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
}

#[derive(Debug, FromRow)]
struct ArchiveAnalyticsRow {
    id: i64,
    dataset: String,
    month_key: String,
    day_key: Option<String>,
    part_key: Option<String>,
    file_path: String,
    sha256: Option<String>,
    row_count: i64,
    status: String,
    layout: String,
    codec: String,
    writer_version: String,
    coverage_start_at: Option<String>,
    coverage_end_at: Option<String>,
    summary_source_kind: String,
}

pub(crate) async fn ensure_analytics_changes_schema(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS analytics_change_events (
            change_id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_version INTEGER NOT NULL,
            event_kind TEXT NOT NULL CHECK(event_kind IN ('upsert', 'tombstone', 'coverage')),
            entity_kind TEXT NOT NULL,
            source TEXT NOT NULL,
            source_row_id INTEGER,
            entity_key TEXT NOT NULL,
            generation INTEGER NOT NULL,
            coverage_revision INTEGER NOT NULL,
            occurred_at TEXT NOT NULL,
            fact_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            CHECK(length(fact_json) <= 65536)
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure analytics_change_events table")?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS analytics_change_state (
            id INTEGER PRIMARY KEY CHECK(id = 1),
            generation INTEGER NOT NULL DEFAULT 1,
            coverage_revision INTEGER NOT NULL DEFAULT 0,
            retained_from_change_id INTEGER NOT NULL DEFAULT 1,
            capacity INTEGER NOT NULL DEFAULT 10000 CHECK(capacity > 0),
            invalidation_pending INTEGER NOT NULL DEFAULT 0 CHECK(invalidation_pending IN (0, 1)),
            historical_replay_required INTEGER NOT NULL DEFAULT 0 CHECK(historical_replay_required IN (0, 1)),
            historical_history_unknown INTEGER NOT NULL DEFAULT 0 CHECK(historical_history_unknown IN (0, 1)),
            historical_history_initialized INTEGER NOT NULL DEFAULT 0 CHECK(historical_history_initialized IN (0, 1)),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure analytics_change_state table")?;

    let state_columns = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('analytics_change_state') WHERE name = 'historical_replay_required')",
    )
    .fetch_one(pool)
    .await
    .context("failed to inspect analytics change state columns")?;
    if state_columns == 0 {
        sqlx::query(
            "ALTER TABLE analytics_change_state ADD COLUMN historical_replay_required INTEGER NOT NULL DEFAULT 0 CHECK(historical_replay_required IN (0, 1))",
        )
        .execute(pool)
        .await
        .context("failed to add analytics change historical replay state")?;
    }

    let historical_unknown_column_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('analytics_change_state') WHERE name = 'historical_history_unknown')",
    )
    .fetch_one(pool)
    .await
    .context("failed to inspect legacy analytics history state")?
        != 0;
    if !historical_unknown_column_exists {
        sqlx::query(
            "ALTER TABLE analytics_change_state ADD COLUMN historical_history_unknown INTEGER NOT NULL DEFAULT 0 CHECK(historical_history_unknown IN (0, 1))",
        )
        .execute(pool)
        .await
        .context("failed to add analytics change historical verification state")?;
    }

    let historical_initialization_column_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('analytics_change_state') WHERE name = 'historical_history_initialized')",
    )
    .fetch_one(pool)
    .await
    .context("failed to inspect analytics history migration state")?
        != 0;
    if !historical_initialization_column_exists {
        sqlx::query(
            "ALTER TABLE analytics_change_state ADD COLUMN historical_history_initialized INTEGER NOT NULL DEFAULT 0 CHECK(historical_history_initialized IN (0, 1))",
        )
        .execute(pool)
        .await
        .context("failed to add analytics history migration state")?;
    }

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS analytics_change_coverage (
            replica_id TEXT PRIMARY KEY,
            generation INTEGER NOT NULL,
            covered_through_change_id INTEGER NOT NULL,
            coverage_revision INTEGER NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure analytics_change_coverage table")?;

    sqlx::query(
        r#"
        INSERT INTO analytics_change_state (id, capacity)
        VALUES (1, ?1)
        ON CONFLICT(id) DO NOTHING
        "#,
    )
    .bind(ANALYTICS_CHANGE_CAPACITY_DEFAULT)
    .execute(pool)
    .await
    .context("failed to seed analytics change state")?;

    let historical_history_initialized: i64 = sqlx::query_scalar(
        "SELECT historical_history_initialized FROM analytics_change_state WHERE id = 1",
    )
    .fetch_one(pool)
    .await
    .context("failed to load analytics history migration state")?;
    if historical_history_initialized == 0 {
        let invocation_table_exists = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'codex_invocations')",
        )
        .fetch_one(pool)
        .await
        .context("failed to inspect SQLite invocation history")?
            != 0;
        let invocation_rows_exist = if invocation_table_exists {
            sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM codex_invocations)")
                .fetch_one(pool)
                .await
                .context("failed to inspect existing SQLite invocations")?
                != 0
        } else {
            false
        };
        let event_rows_exist =
            sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM analytics_change_events)")
                .fetch_one(pool)
                .await
                .context("failed to inspect existing analytics changes")?
                != 0;
        let mut legacy_projection_rows_exist = false;
        for table_name in LEGACY_ANALYTICS_HISTORY_TABLES {
            let table_exists = sqlx::query_scalar::<_, i64>(&format!(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '{table_name}')"
            ))
            .fetch_one(pool)
            .await
            .with_context(|| format!("failed to inspect legacy analytics table {table_name}"))?
                != 0;
            if table_exists
                && sqlx::query_scalar::<_, i64>(&format!(
                    "SELECT EXISTS(SELECT 1 FROM \"{table_name}\")"
                ))
                .fetch_one(pool)
                .await
                .with_context(|| {
                    format!("failed to inspect legacy analytics rows in {table_name}")
                })? != 0
            {
                legacy_projection_rows_exist = true;
                break;
            }
        }
        if !legacy_projection_rows_exist {
            for (table_name, predicate) in LEGACY_ANALYTICS_HISTORY_STATE_CHECKS {
                let table_exists = sqlx::query_scalar::<_, i64>(&format!(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '{table_name}')"
                ))
                .fetch_one(pool)
                .await
                .with_context(|| {
                    format!("failed to inspect legacy analytics state table {table_name}")
                })? != 0;
                if table_exists
                    && sqlx::query_scalar::<_, i64>(&format!(
                        "SELECT EXISTS(SELECT 1 FROM \"{table_name}\" WHERE {predicate})"
                    ))
                    .fetch_one(pool)
                    .await
                    .with_context(|| {
                        format!("failed to inspect legacy analytics state rows in {table_name}")
                    })? != 0
                {
                    legacy_projection_rows_exist = true;
                    break;
                }
            }
        }
        let invocation_archive_rows_exist = if sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'archive_batches')",
        )
        .fetch_one(pool)
        .await
        .context("failed to inspect legacy invocation archive metadata")?
            != 0
        {
            sqlx::query_scalar::<_, i64>(
                "SELECT EXISTS(SELECT 1 FROM archive_batches WHERE dataset = 'codex_invocations')",
            )
            .fetch_one(pool)
            .await
            .context("failed to inspect legacy invocation archive rows")?
                != 0
        } else {
            false
        };
        let source_data_exists = invocation_rows_exist
            || event_rows_exist
            || legacy_projection_rows_exist
            || invocation_archive_rows_exist;
        sqlx::query(
            r#"
            UPDATE analytics_change_state
            SET historical_history_unknown = CASE WHEN ?1 THEN 1 ELSE historical_history_unknown END,
                historical_history_initialized = 1,
                updated_at = datetime('now')
            WHERE id = 1
            "#,
        )
        .bind(source_data_exists)
        .execute(pool)
        .await
        .context("failed to complete analytics history migration state")?;
    }

    let archive_table_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'archive_batches')",
    )
    .fetch_one(pool)
    .await
    .context("failed to inspect analytics archive metadata")?
        != 0;
    if archive_table_exists {
        sqlx::query(
            r#"
            UPDATE analytics_change_state
            SET historical_replay_required = 1,
                updated_at = datetime('now')
            WHERE id = 1
              AND EXISTS(
                  SELECT 1 FROM archive_batches
                  WHERE dataset = 'codex_invocations'
                    AND status = 'completed'
                    AND COALESCE(summary_source_kind, 'unknown') <> 'live_mirror'
              )
            "#,
        )
        .execute(pool)
        .await
        .context("failed to preserve existing analytics archive coverage")?;
    }

    for statement in [
        "CREATE INDEX IF NOT EXISTS idx_analytics_change_events_generation_cursor ON analytics_change_events (generation, change_id)",
        "CREATE INDEX IF NOT EXISTS idx_analytics_change_events_entity_cursor ON analytics_change_events (entity_kind, source_row_id, change_id)",
        "CREATE INDEX IF NOT EXISTS idx_analytics_change_coverage_generation ON analytics_change_coverage (generation, covered_through_change_id)",
    ] {
        sqlx::query(statement)
            .execute(pool)
            .await
            .with_context(|| format!("failed to ensure analytics change index: {statement}"))?;
    }

    Ok(())
}

fn bounded_text(value: Option<&str>) -> Option<String> {
    let value = value?.replace('\0', "");
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    Some(
        value
            .chars()
            .take(ANALYTICS_CHANGE_TEXT_MAX_CHARS)
            .collect(),
    )
}

fn bounded_required_text(value: &str, label: &str) -> Result<String> {
    let normalized = value.replace('\0', "");
    let value = normalized.trim();
    if value.is_empty() {
        bail!("analytics change {label} must not be empty");
    }
    if value.chars().count() > ANALYTICS_CHANGE_TEXT_MAX_CHARS {
        bail!("analytics change {label} exceeds the bounded size");
    }
    Ok(value.to_string())
}

fn bounded_entity_key(value: &str) -> Result<String> {
    let normalized = value.replace('\0', "");
    let value = normalized.trim();
    if value.is_empty() {
        bail!("analytics change entity key must not be empty");
    }
    if value.len() > ANALYTICS_CHANGE_ENTITY_KEY_MAX_BYTES {
        bail!("analytics change entity key exceeds the bounded size");
    }
    Ok(value.to_string())
}

fn normalized_payload(payload: Option<&str>) -> Option<Value> {
    payload
        .filter(|value| !value.trim().is_empty())
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
}

fn payload_text(payload: Option<&Value>, key: &str) -> Option<String> {
    payload
        .and_then(|value| value.get(key))
        .and_then(Value::as_str)
        .and_then(|value| bounded_text(Some(value)))
}

fn payload_i64(payload: Option<&Value>, key: &str) -> Option<i64> {
    payload.and_then(|value| value.get(key)).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
    })
}

pub(crate) fn normalize_analytics_text(value: Option<&str>) -> Option<String> {
    bounded_text(value)
}

pub(crate) fn normalize_analytics_required_text(value: &str, label: &str) -> Result<String> {
    bounded_required_text(value, label)
}

pub(crate) fn normalize_analytics_payload(payload: Option<&str>) -> Option<Value> {
    normalized_payload(payload)
}

pub(crate) fn normalize_analytics_payload_text(
    payload: Option<&Value>,
    key: &str,
) -> Option<String> {
    payload_text(payload, key)
}

pub(crate) fn normalize_analytics_payload_i64(payload: Option<&Value>, key: &str) -> Option<i64> {
    payload_i64(payload, key)
}

fn terminal_status(status: Option<&str>) -> bool {
    !matches!(
        status
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("running") | Some("pending")
    )
}

fn success_like_status(status: Option<&str>) -> bool {
    matches!(
        status
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("success") | Some("succeeded") | Some("completed") | Some("ok")
    )
}

async fn load_analytics_change_state(tx: &mut SqliteConnection) -> Result<AnalyticsChangeStateRow> {
    sqlx::query_as::<_, AnalyticsChangeStateRow>(
        r#"
        SELECT generation, coverage_revision, retained_from_change_id, capacity,
               invalidation_pending, historical_replay_required
        FROM analytics_change_state
        WHERE id = 1
        "#,
    )
    .fetch_one(&mut *tx)
    .await
    .context("analytics change state is not initialized")
}

struct AnalyticsChangeInput<'a> {
    event_kind: &'a str,
    entity_kind: &'a str,
    source: &'a str,
    source_row_id: Option<i64>,
    entity_key: &'a str,
    occurred_at: &'a str,
    fact_json: &'a str,
}

async fn append_analytics_change_tx(
    tx: &mut SqliteConnection,
    input: AnalyticsChangeInput<'_>,
) -> Result<AnalyticsChangeReceipt> {
    let AnalyticsChangeInput {
        event_kind,
        entity_kind,
        source,
        source_row_id,
        entity_key,
        occurred_at,
        fact_json,
    } = input;
    if !matches!(event_kind, "upsert" | "tombstone" | "coverage") {
        bail!("unsupported analytics change event kind: {event_kind}");
    }
    let entity_kind = bounded_required_text(entity_kind, "entity kind")?;
    let source = bounded_required_text(source, "source")?;
    let occurred_at = bounded_required_text(occurred_at, "occurred_at")?;
    let entity_key = bounded_entity_key(entity_key)?;
    let fact: Value =
        serde_json::from_str(fact_json).context("analytics change fact is not valid JSON")?;
    if !fact.is_object() {
        bail!("analytics change fact must be a JSON object");
    }
    if fact_json.len() > ANALYTICS_CHANGE_FACT_MAX_BYTES {
        bail!("analytics change fact exceeds the bounded size");
    }

    if let Some(existing) = sqlx::query_as::<_, ExistingAnalyticsChangeRow>(
        r#"
        SELECT change_id, event_kind, generation, coverage_revision, fact_json
        FROM analytics_change_events
        WHERE entity_kind = ?1
          AND source = ?2
          AND source_row_id IS ?3
          AND entity_key = ?4
        ORDER BY change_id DESC
        LIMIT 1
        "#,
    )
    .bind(&entity_kind)
    .bind(&source)
    .bind(source_row_id)
    .bind(&entity_key)
    .fetch_optional(&mut *tx)
    .await?
        && existing.event_kind == event_kind
        && existing.fact_json == fact_json
    {
        return Ok(AnalyticsChangeReceipt {
            change_id: existing.change_id,
            generation: existing.generation,
            coverage_revision: existing.coverage_revision,
        });
    }

    let state = load_analytics_change_state(tx).await?;
    let coverage_revision = state
        .coverage_revision
        .checked_add(1)
        .context("analytics change coverage revision overflow")?;
    let result = sqlx::query(
        r#"
        INSERT INTO analytics_change_events (
            event_version,
            event_kind,
            entity_kind,
            source,
            source_row_id,
            entity_key,
            generation,
            coverage_revision,
            occurred_at,
            fact_json
        )
        VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(event_kind)
    .bind(&entity_kind)
    .bind(&source)
    .bind(source_row_id)
    .bind(&entity_key)
    .bind(state.generation)
    .bind(coverage_revision)
    .bind(&occurred_at)
    .bind(fact_json)
    .execute(&mut *tx)
    .await?;
    let change_id = result.last_insert_rowid();

    sqlx::query(
        "UPDATE analytics_change_state SET coverage_revision = ?1, updated_at = datetime('now') WHERE id = 1",
    )
    .bind(coverage_revision)
    .execute(&mut *tx)
    .await?;

    let event_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM analytics_change_events")
        .fetch_one(&mut *tx)
        .await?;
    let generation = if event_count > state.capacity {
        let retained_from_change_id = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COALESCE(MIN(change_id), ?1)
            FROM (
                SELECT change_id
                FROM analytics_change_events
                ORDER BY change_id DESC
                LIMIT ?2
            )
            "#,
        )
        .bind(change_id)
        .bind(state.capacity)
        .fetch_one(&mut *tx)
        .await?;
        let evicted_through_change_id = retained_from_change_id.saturating_sub(1);
        let stale_coverage_exists = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT EXISTS(
                SELECT 1
                FROM analytics_change_coverage
                WHERE generation <> ?1
                   OR covered_through_change_id < ?2
            )
            "#,
        )
        .bind(state.generation)
        .bind(evicted_through_change_id)
        .fetch_one(&mut *tx)
        .await?
            != 0;
        let generation = if stale_coverage_exists && state.invalidation_pending == 0 {
            state
                .generation
                .checked_add(1)
                .context("analytics change generation overflow")?
        } else {
            state.generation
        };
        sqlx::query("DELETE FROM analytics_change_events WHERE change_id < ?1")
            .bind(retained_from_change_id)
            .execute(&mut *tx)
            .await?;
        if generation != state.generation {
            sqlx::query("UPDATE analytics_change_events SET generation = ?1 WHERE change_id >= ?2")
                .bind(generation)
                .bind(retained_from_change_id)
                .execute(&mut *tx)
                .await?;
        }
        let invalidation_pending =
            i64::from(state.invalidation_pending != 0 || stale_coverage_exists);
        sqlx::query(
            r#"
            UPDATE analytics_change_state
            SET generation = ?1,
                retained_from_change_id = ?2,
                invalidation_pending = ?3,
                updated_at = datetime('now')
            WHERE id = 1
            "#,
        )
        .bind(generation)
        .bind(retained_from_change_id)
        .bind(invalidation_pending)
        .execute(&mut *tx)
        .await?;
        generation
    } else {
        state.generation
    };

    Ok(AnalyticsChangeReceipt {
        change_id,
        generation,
        coverage_revision,
    })
}

pub(crate) async fn append_invocation_upsert_for_row_tx(
    tx: &mut SqliteConnection,
    row_id: i64,
) -> Result<AnalyticsChangeReceipt> {
    let row = sqlx::query_as::<_, InvocationAnalyticsRow>(
        r#"
        SELECT
            id,
            invoke_id,
            occurred_at,
            CAST(COALESCE(ROUND((julianday(
                occurred_at,
                CASE WHEN instr(occurred_at, 'T') > 0 THEN '+0 hours' ELSE '-8 hours' END
            ) - 2440587.5) * 86400000.0), 0) AS INTEGER) AS occurred_at_epoch_ms,
            substr(occurred_at, 1, 10) AS stats_date,
            source,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            total_tokens,
            cost,
            cost_input,
            cost_cache_write,
            cost_cache_read,
            cost_output,
            cost_reasoning,
            status,
            failure_kind,
            failure_class,
            is_actionable,
            payload,
            t_total_ms,
            t_req_read_ms,
            t_req_parse_ms,
            t_upstream_connect_ms,
            t_upstream_ttfb_ms,
            t_upstream_stream_ms,
            first_token_ms,
            t_resp_parse_ms
        FROM codex_invocations
        WHERE id = ?1
        "#,
    )
    .bind(row_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| {
        anyhow!("cannot publish analytics invocation change for missing row {row_id}")
    })?;

    let payload = normalized_payload(row.payload.as_deref());
    let request_model = payload_text(payload.as_ref(), "requestModel");
    let response_model = payload_text(payload.as_ref(), "responseModel");
    let reasoning_effort = payload_text(payload.as_ref(), "reasoningEffort");
    let upstream_account_name = payload_text(payload.as_ref(), "upstreamAccountName");
    let upstream_account_id = payload_i64(payload.as_ref(), "upstreamAccountId");
    let status = bounded_text(row.status.as_deref());
    let failure_kind = bounded_text(row.failure_kind.as_deref());
    let failure_class = bounded_text(row.failure_class.as_deref());
    let source = bounded_required_text(&row.source, "source")?;
    let invoke_id = bounded_required_text(&row.invoke_id, "invoke_id")?;
    let entity_key = format!("invocation:{}:{}", source, row.id);
    let fact_json = json!({
        "factVersion": ANALYTICS_CHANGE_EVENT_VERSION,
        "source": source,
        "sourceRowId": row.id,
        "invokeId": invoke_id,
        "occurredAt": row.occurred_at,
        "occurredAtEpochMs": row.occurred_at_epoch_ms,
        "statsDate": row.stats_date,
        "model": bounded_text(row.model.as_deref()),
        "requestModel": request_model,
        "responseModel": response_model,
        "reasoningEffort": reasoning_effort,
        "upstreamAccountId": upstream_account_id,
        "upstreamAccountName": upstream_account_name,
        "status": status.clone(),
        "successLike": success_like_status(status.as_deref()),
        "terminal": terminal_status(status.as_deref()),
        "failureKind": failure_kind,
        "failureClass": failure_class,
        "isActionable": row.is_actionable.map(|value| value != 0),
        "inputTokens": row.input_tokens,
        "outputTokens": row.output_tokens,
        "cacheInputTokens": row.cache_input_tokens,
        "reasoningTokens": row.reasoning_tokens,
        "totalTokens": row.total_tokens,
        "cost": row.cost,
        "costInput": row.cost_input,
        "costCacheWrite": row.cost_cache_write,
        "costCacheRead": row.cost_cache_read,
        "costOutput": row.cost_output,
        "costReasoning": row.cost_reasoning,
        "tTotalMs": row.t_total_ms,
        "tReqReadMs": row.t_req_read_ms,
        "tReqParseMs": row.t_req_parse_ms,
        "tUpstreamConnectMs": row.t_upstream_connect_ms,
        "tUpstreamTtfbMs": row.t_upstream_ttfb_ms,
        "tUpstreamStreamMs": row.t_upstream_stream_ms,
        "firstTokenMs": row.first_token_ms,
        "tRespParseMs": row.t_resp_parse_ms,
    })
    .to_string();
    append_analytics_change_tx(
        tx,
        AnalyticsChangeInput {
            event_kind: "upsert",
            entity_kind: "invocation",
            source: &source,
            source_row_id: Some(row.id),
            entity_key: &entity_key,
            occurred_at: &row.occurred_at,
            fact_json: &fact_json,
        },
    )
    .await
}

pub(crate) async fn append_invocation_tombstone_for_identity_tx(
    tx: &mut SqliteConnection,
    source: &str,
    source_row_id: i64,
    invoke_id: &str,
    occurred_at: &str,
) -> Result<AnalyticsChangeReceipt> {
    let source = bounded_required_text(source, "source")?;
    let invoke_id = bounded_required_text(invoke_id, "invoke_id")?;
    let fact_json = json!({
        "factVersion": ANALYTICS_CHANGE_EVENT_VERSION,
        "source": source,
        "sourceRowId": source_row_id,
        "invokeId": invoke_id,
        "occurredAt": occurred_at,
        "deleted": true,
    })
    .to_string();
    append_analytics_change_tx(
        tx,
        AnalyticsChangeInput {
            event_kind: "tombstone",
            entity_kind: "invocation",
            source: &source,
            source_row_id: Some(source_row_id),
            entity_key: &format!("invocation:{}:{}", source, source_row_id),
            occurred_at,
            fact_json: &fact_json,
        },
    )
    .await
}

pub(crate) async fn delete_invocation_with_analytics_tombstone_tx(
    tx: &mut SqliteConnection,
    row_id: i64,
) -> Result<bool> {
    let identity = sqlx::query_as::<_, (String, String, String)>(
        "SELECT source, invoke_id, occurred_at FROM codex_invocations WHERE id = ?1",
    )
    .bind(row_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((source, invoke_id, occurred_at)) = identity else {
        return Ok(false);
    };
    let result = sqlx::query("DELETE FROM codex_invocations WHERE id = ?1")
        .bind(row_id)
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() == 1 {
        append_invocation_tombstone_for_identity_tx(tx, &source, row_id, &invoke_id, &occurred_at)
            .await?;
        return Ok(true);
    }
    Ok(false)
}

async fn load_attempt_analytics_row_tx(
    tx: &mut SqliteConnection,
    query: &str,
    value: i64,
) -> Result<AttemptAnalyticsRow> {
    sqlx::query_as::<_, AttemptAnalyticsRow>(query)
        .bind(value)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow!("cannot publish analytics attempt change for missing row {value}"))
}

pub(crate) async fn append_attempt_upsert_for_row_tx(
    tx: &mut SqliteConnection,
    row_id: i64,
) -> Result<AnalyticsChangeReceipt> {
    let row = load_attempt_analytics_row_tx(
        tx,
        r#"
        SELECT id, attempt_public_id, invoke_id, occurred_at, endpoint,
               request_model, upstream_request_model, upstream_account_id,
               attempt_index, distinct_account_index, same_account_retry_index,
               status, phase, http_status, downstream_http_status, failure_kind,
               connect_latency_ms, first_byte_latency_ms, stream_latency_ms
        FROM pool_upstream_request_attempts
        WHERE id = ?1
        "#,
        row_id,
    )
    .await?;
    append_attempt_analytics_row_tx(tx, row).await
}

pub(crate) async fn append_attempt_upsert_for_public_id_tx(
    tx: &mut SqliteConnection,
    attempt_public_id: &str,
) -> Result<AnalyticsChangeReceipt> {
    let row = sqlx::query_as::<_, AttemptAnalyticsRow>(
        r#"
        SELECT id, attempt_public_id, invoke_id, occurred_at, endpoint,
               request_model, upstream_request_model, upstream_account_id,
               attempt_index, distinct_account_index, same_account_retry_index,
               status, phase, http_status, downstream_http_status, failure_kind,
               connect_latency_ms, first_byte_latency_ms, stream_latency_ms
        FROM pool_upstream_request_attempts
        WHERE attempt_public_id = ?1
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(attempt_public_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| anyhow!("cannot publish analytics attempt change for missing public id"))?;
    append_attempt_analytics_row_tx(tx, row).await
}

async fn append_attempt_analytics_row_tx(
    tx: &mut SqliteConnection,
    row: AttemptAnalyticsRow,
) -> Result<AnalyticsChangeReceipt> {
    let source = "pool_upstream_request_attempts";
    let attempt_public_id = bounded_text(row.attempt_public_id.as_deref());
    let fact_json = json!({
        "factVersion": ANALYTICS_CHANGE_EVENT_VERSION,
        "source": source,
        "sourceRowId": row.id,
        "attemptPublicId": attempt_public_id,
        "invokeId": bounded_required_text(&row.invoke_id, "invoke_id")?,
        "occurredAt": row.occurred_at,
        "endpoint": bounded_text(Some(&row.endpoint)),
        "requestModel": bounded_text(row.request_model.as_deref()),
        "upstreamRequestModel": bounded_text(row.upstream_request_model.as_deref()),
        "upstreamAccountId": row.upstream_account_id,
        "attemptIndex": row.attempt_index,
        "distinctAccountIndex": row.distinct_account_index,
        "sameAccountRetryIndex": row.same_account_retry_index,
        "status": bounded_text(Some(&row.status)),
        "phase": bounded_text(row.phase.as_deref()),
        "httpStatus": row.http_status,
        "downstreamHttpStatus": row.downstream_http_status,
        "failureKind": bounded_text(row.failure_kind.as_deref()),
        "connectLatencyMs": row.connect_latency_ms,
        "firstByteLatencyMs": row.first_byte_latency_ms,
        "streamLatencyMs": row.stream_latency_ms,
    })
    .to_string();
    append_analytics_change_tx(
        tx,
        AnalyticsChangeInput {
            event_kind: "upsert",
            entity_kind: "pool_attempt",
            source,
            source_row_id: Some(row.id),
            entity_key: &format!("attempt:{}", row.id),
            occurred_at: &row.occurred_at,
            fact_json: &fact_json,
        },
    )
    .await
}

pub(crate) async fn append_archive_coverage_for_file_tx(
    tx: &mut SqliteConnection,
    file_path: &str,
) -> Result<AnalyticsChangeReceipt> {
    let row = sqlx::query_as::<_, ArchiveAnalyticsRow>(
        r#"
        SELECT id, dataset, month_key, day_key, part_key, file_path, sha256,
               row_count, status, layout, codec, writer_version,
               coverage_start_at, coverage_end_at, summary_source_kind
        FROM archive_batches
        WHERE file_path = ?1
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind(file_path)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| anyhow!("cannot publish analytics archive change for missing file"))?;
    if row.status != ARCHIVE_STATUS_COMPLETED {
        bail!("cannot publish incomplete analytics archive coverage: {file_path}");
    }
    let occurred_at = row
        .coverage_start_at
        .as_deref()
        .unwrap_or(row.month_key.as_str());
    let fact_json = json!({
        "factVersion": ANALYTICS_CHANGE_EVENT_VERSION,
        "source": row.dataset,
        "sourceRowId": row.id,
        "dataset": row.dataset,
        "monthKey": row.month_key,
        "dayKey": row.day_key,
        "partKey": row.part_key,
        "filePath": bounded_text(Some(&row.file_path)),
        "sha256": bounded_text(row.sha256.as_deref()),
        "rowCount": row.row_count,
        "status": row.status,
        "layout": bounded_text(Some(&row.layout)),
        "codec": bounded_text(Some(&row.codec)),
        "writerVersion": bounded_text(Some(&row.writer_version)),
        "coverageStartAt": row.coverage_start_at,
        "coverageEndAt": row.coverage_end_at,
        "summarySourceKind": bounded_text(Some(&row.summary_source_kind)),
    })
    .to_string();
    let receipt = append_analytics_change_tx(
        tx,
        AnalyticsChangeInput {
            event_kind: "coverage",
            entity_kind: "archive_coverage",
            source: &row.dataset,
            source_row_id: Some(row.id),
            entity_key: &format!(
                "archive:{}:{}:{}",
                row.dataset, row.month_key, row.file_path
            ),
            occurred_at,
            fact_json: &fact_json,
        },
    )
    .await?;
    if row.dataset == "codex_invocations" && row.summary_source_kind != "live_mirror" {
        sqlx::query(
            "UPDATE analytics_change_state SET historical_replay_required = 1, updated_at = datetime('now') WHERE id = 1",
        )
        .execute(&mut *tx)
        .await
        .context("failed to persist analytics historical replay requirement")?;
    }
    Ok(receipt)
}

pub(crate) async fn claim_analytics_change_coverage(
    pool: &Pool<Sqlite>,
    replica_id: &str,
    expected_generation: i64,
    expected_coverage_revision: i64,
    from_change_id: i64,
    through_change_id: i64,
) -> Result<AnalyticsCoverageReceipt> {
    if from_change_id < 1 || through_change_id < 0 || from_change_id > through_change_id + 1 {
        bail!("invalid analytics change coverage range");
    }
    let replica_id = bounded_required_text(replica_id, "replica id")?;
    let mut tx = pool.begin().await?;
    let state = load_analytics_change_state(tx.as_mut()).await?;
    if state.invalidation_pending != 0 {
        bail!("analytics change generation requires a replica rebuild");
    }
    if state.generation != expected_generation {
        bail!("analytics change generation is stale");
    }
    if state.coverage_revision != expected_coverage_revision {
        bail!("analytics change coverage revision is stale");
    }
    if from_change_id < state.retained_from_change_id {
        bail!("analytics change range predates the retained source journal");
    }
    let latest_change_id = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(change_id), 0) FROM analytics_change_events",
    )
    .fetch_one(tx.as_mut())
    .await?;
    if through_change_id > latest_change_id {
        bail!("analytics change coverage is ahead of the source journal");
    }
    let previous_coverage = sqlx::query_as::<_, (i64, i64)>(
        "SELECT generation, covered_through_change_id FROM analytics_change_coverage WHERE replica_id = ?1",
    )
    .bind(&replica_id)
    .fetch_optional(tx.as_mut())
    .await?;
    if let Some((previous_generation, previous_through_change_id)) = previous_coverage.as_ref() {
        if *previous_generation != state.generation {
            bail!("analytics change coverage belongs to a stale generation");
        }
        if through_change_id > *previous_through_change_id
            && from_change_id > (*previous_through_change_id).saturating_add(1)
        {
            bail!("analytics change coverage skips source journal entries");
        }
    } else if from_change_id != state.retained_from_change_id {
        bail!("initial analytics change coverage must start at the retained journal boundary");
    }
    let covered_through_change_id = previous_coverage
        .map(|(_, previous_through_change_id)| previous_through_change_id.max(through_change_id))
        .unwrap_or(through_change_id);
    sqlx::query(
        r#"
        INSERT INTO analytics_change_coverage (
            replica_id, generation, covered_through_change_id, coverage_revision, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, datetime('now'))
        ON CONFLICT(replica_id) DO UPDATE SET
            generation = excluded.generation,
            covered_through_change_id = excluded.covered_through_change_id,
            coverage_revision = excluded.coverage_revision,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(&replica_id)
    .bind(state.generation)
    .bind(covered_through_change_id)
    .bind(state.coverage_revision)
    .execute(tx.as_mut())
    .await?;
    tx.commit().await?;
    Ok(AnalyticsCoverageReceipt {
        generation: state.generation,
        coverage_revision: state.coverage_revision,
        covered_through_change_id,
    })
}

pub(crate) async fn acknowledge_analytics_change_generation(
    pool: &Pool<Sqlite>,
    replica_id: &str,
    generation: i64,
    covered_through_change_id: i64,
) -> Result<AnalyticsCoverageReceipt> {
    let replica_id = bounded_required_text(replica_id, "replica id")?;
    let mut tx = pool.begin().await?;
    let state = load_analytics_change_state(tx.as_mut()).await?;
    if state.generation != generation {
        bail!("cannot acknowledge a stale analytics change generation");
    }
    let latest_change_id = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(change_id), 0) FROM analytics_change_events",
    )
    .fetch_one(tx.as_mut())
    .await?;
    if covered_through_change_id < latest_change_id {
        bail!("analytics change generation acknowledgement is incomplete");
    }
    if covered_through_change_id > latest_change_id {
        bail!("analytics change generation acknowledgement is ahead of the source journal");
    }
    sqlx::query(
        r#"
        INSERT INTO analytics_change_coverage (
            replica_id, generation, covered_through_change_id, coverage_revision, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, datetime('now'))
        ON CONFLICT(replica_id) DO UPDATE SET
            generation = excluded.generation,
            covered_through_change_id = excluded.covered_through_change_id,
            coverage_revision = excluded.coverage_revision,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(&replica_id)
    .bind(state.generation)
    .bind(covered_through_change_id)
    .bind(state.coverage_revision)
    .execute(tx.as_mut())
    .await?;
    let stale_coverage_exists = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM analytics_change_coverage
            WHERE generation <> ?1
               OR covered_through_change_id < ?2
        )
        "#,
    )
    .bind(state.generation)
    .bind(latest_change_id)
    .fetch_one(tx.as_mut())
    .await?
        != 0;
    sqlx::query(
        "UPDATE analytics_change_state SET invalidation_pending = ?2, updated_at = datetime('now') WHERE id = 1 AND generation = ?1",
    )
    .bind(generation)
    .bind(i64::from(stale_coverage_exists))
    .execute(tx.as_mut())
    .await?;
    tx.commit().await?;
    Ok(AnalyticsCoverageReceipt {
        generation: state.generation,
        coverage_revision: state.coverage_revision,
        covered_through_change_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect analytics changes test database");
        crate::schema::ensure_schema(&pool)
            .await
            .expect("ensure analytics changes schema");
        pool
    }

    async fn insert_invocation(pool: &Pool<Sqlite>, invoke_id: &str, status: &str) -> i64 {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            ) VALUES (?1, '2026-10-30 12:00:00', 'proxy', ?2, 7, 0.07,
                      '{"requestModel":"gpt-5","upstreamAccountId":42}', '{}')
            "#,
        )
        .bind(invoke_id)
        .bind(status)
        .execute(pool)
        .await
        .expect("insert invocation fixture")
        .last_insert_rowid()
    }

    async fn append_test_fact(pool: &Pool<Sqlite>, fact: &str) -> AnalyticsChangeReceipt {
        let mut tx = pool.begin().await.expect("begin source change transaction");
        let receipt = append_analytics_change_tx(
            tx.as_mut(),
            AnalyticsChangeInput {
                event_kind: "upsert",
                entity_kind: "invocation",
                source: "proxy",
                source_row_id: Some(1),
                entity_key: "invocation:proxy:1",
                occurred_at: "2026-10-30 12:00:00",
                fact_json: fact,
            },
        )
        .await
        .expect("append source change");
        tx.commit().await.expect("commit source change transaction");
        receipt
    }

    #[tokio::test]
    async fn source_change_append_rolls_back_with_business_transaction() {
        let pool = test_pool().await;
        let mut tx = pool.begin().await.expect("begin rollback transaction");
        let row_id = sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, raw_response) VALUES ('rollback', '2026-10-30 12:00:00', 'proxy', 'success', '{}')",
        )
        .execute(tx.as_mut())
        .await
        .expect("insert rollback fixture")
        .last_insert_rowid();
        append_invocation_upsert_for_row_tx(tx.as_mut(), row_id)
            .await
            .expect("append rollback event");
        tx.rollback().await.expect("rollback source transaction");

        let source_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .expect("count source rows");
        let event_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM analytics_change_events")
            .fetch_one(&pool)
            .await
            .expect("count change rows");
        assert_eq!(source_count, 0);
        assert_eq!(event_count, 0);
    }

    #[tokio::test]
    async fn historical_replay_marker_migrates_legacy_state_without_false_positive() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect legacy analytics changes test database");
        sqlx::query(
            r#"
            CREATE TABLE analytics_change_state (
                id INTEGER PRIMARY KEY CHECK(id = 1),
                generation INTEGER NOT NULL DEFAULT 1,
                coverage_revision INTEGER NOT NULL DEFAULT 0,
                retained_from_change_id INTEGER NOT NULL DEFAULT 1,
                capacity INTEGER NOT NULL DEFAULT 10000,
                invalidation_pending INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create legacy analytics change state");
        sqlx::query("INSERT INTO analytics_change_state (id) VALUES (1)")
            .execute(&pool)
            .await
            .expect("seed legacy analytics change state");

        ensure_analytics_changes_schema(&pool)
            .await
            .expect("migrate legacy analytics change state");

        let historical_replay_required: i64 = sqlx::query_scalar(
            "SELECT historical_replay_required FROM analytics_change_state WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("load migrated historical replay marker");
        assert_eq!(historical_replay_required, 0);
        let historical_history_unknown: i64 = sqlx::query_scalar(
            "SELECT historical_history_unknown FROM analytics_change_state WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("load migrated historical verification marker");
        assert_eq!(historical_history_unknown, 0);
        let historical_history_initialized: i64 = sqlx::query_scalar(
            "SELECT historical_history_initialized FROM analytics_change_state WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("load migrated historical initialization marker");
        assert_eq!(historical_history_initialized, 1);
    }

    #[tokio::test]
    async fn historical_history_migration_marks_existing_source_data_unknown() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect legacy analytics history test database");
        sqlx::query(
            r#"
            CREATE TABLE analytics_change_state (
                id INTEGER PRIMARY KEY CHECK(id = 1),
                generation INTEGER NOT NULL DEFAULT 1,
                coverage_revision INTEGER NOT NULL DEFAULT 0,
                retained_from_change_id INTEGER NOT NULL DEFAULT 1,
                capacity INTEGER NOT NULL DEFAULT 10000,
                invalidation_pending INTEGER NOT NULL DEFAULT 0 CHECK(invalidation_pending IN (0, 1)),
                historical_replay_required INTEGER NOT NULL DEFAULT 0 CHECK(historical_replay_required IN (0, 1)),
                historical_history_unknown INTEGER NOT NULL DEFAULT 0 CHECK(historical_history_unknown IN (0, 1)),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create legacy analytics history state");
        sqlx::query("INSERT INTO analytics_change_state (id) VALUES (1)")
            .execute(&pool)
            .await
            .expect("seed legacy analytics history state");
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .expect("create source invocation table");
        sqlx::query("INSERT INTO codex_invocations (id, invoke_id) VALUES (1, 'legacy')")
            .execute(&pool)
            .await
            .expect("seed source invocation row");

        ensure_analytics_changes_schema(&pool)
            .await
            .expect("migrate legacy analytics history state with source data");

        let markers: (i64, i64) = sqlx::query_as(
            "SELECT historical_history_unknown, historical_history_initialized FROM analytics_change_state WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("load migrated analytics history markers");
        assert_eq!(markers, (1, 1));
    }

    #[tokio::test]
    async fn historical_history_migration_ignores_default_control_singletons() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect legacy control state test database");
        sqlx::query(
            r#"
            CREATE TABLE analytics_change_state (
                id INTEGER PRIMARY KEY CHECK(id = 1),
                generation INTEGER NOT NULL DEFAULT 1,
                coverage_revision INTEGER NOT NULL DEFAULT 0,
                retained_from_change_id INTEGER NOT NULL DEFAULT 1,
                capacity INTEGER NOT NULL DEFAULT 10000,
                invalidation_pending INTEGER NOT NULL DEFAULT 0 CHECK(invalidation_pending IN (0, 1)),
                historical_replay_required INTEGER NOT NULL DEFAULT 0 CHECK(historical_replay_required IN (0, 1)),
                historical_history_unknown INTEGER NOT NULL DEFAULT 0 CHECK(historical_history_unknown IN (0, 1)),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create legacy control state");
        sqlx::query("INSERT INTO analytics_change_state (id) VALUES (1)")
            .execute(&pool)
            .await
            .expect("seed legacy control state");
        sqlx::query("CREATE TABLE long_term_stats_state (id INTEGER PRIMARY KEY, status TEXT)")
            .execute(&pool)
            .await
            .expect("create legacy long-term control state");
        sqlx::query("INSERT INTO long_term_stats_state (id, status) VALUES (1, 'preparing')")
            .execute(&pool)
            .await
            .expect("seed legacy long-term control state");
        for table_name in [
            "summary_coverage_revision",
            "summary_account_coverage_revision",
        ] {
            sqlx::query(&format!(
                "CREATE TABLE {table_name} (id INTEGER PRIMARY KEY, revision INTEGER NOT NULL)"
            ))
            .execute(&pool)
            .await
            .expect("create legacy Summary revision state");
            sqlx::query(&format!(
                "INSERT INTO {table_name} (id, revision) VALUES (1, 1)"
            ))
            .execute(&pool)
            .await
            .expect("seed legacy Summary revision state");
        }
        sqlx::query(
            "CREATE TABLE long_term_projection_state (consumer TEXT PRIMARY KEY, cursor_row_id INTEGER NOT NULL, last_flush_at TEXT, last_error TEXT)",
        )
        .execute(&pool)
        .await
        .expect("create legacy projection cursor state");
        sqlx::query(
            "INSERT INTO long_term_projection_state (consumer, cursor_row_id) VALUES ('analytics', 0)",
        )
        .execute(&pool)
        .await
        .expect("seed legacy projection cursor state");
        sqlx::query(
            "CREATE TABLE summary_source_change_cursor (scope TEXT PRIMARY KEY, cursor INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await
        .expect("create legacy Summary source cursor state");
        sqlx::query(
            "INSERT INTO summary_source_change_cursor (scope, cursor) VALUES ('summary', 0)",
        )
        .execute(&pool)
        .await
        .expect("seed legacy Summary source cursor state");

        ensure_analytics_changes_schema(&pool)
            .await
            .expect("migrate legacy control state");

        let markers: (i64, i64) = sqlx::query_as(
            "SELECT historical_history_unknown, historical_history_initialized FROM analytics_change_state WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("load migrated control history markers");
        assert_eq!(markers, (0, 1));

        sqlx::query("UPDATE long_term_projection_state SET cursor_row_id = 1")
            .execute(&pool)
            .await
            .expect("advance legacy projection cursor state");
        sqlx::query(
            "UPDATE analytics_change_state SET historical_history_unknown = 0, historical_history_initialized = 0 WHERE id = 1",
        )
        .execute(&pool)
        .await
        .expect("reset legacy history migration marker");
        ensure_analytics_changes_schema(&pool)
            .await
            .expect("retry legacy control state migration");
        let markers: (i64, i64) = sqlx::query_as(
            "SELECT historical_history_unknown, historical_history_initialized FROM analytics_change_state WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("load retried control history markers");
        assert_eq!(markers, (1, 1));

        sqlx::query("UPDATE long_term_projection_state SET cursor_row_id = 0")
            .execute(&pool)
            .await
            .expect("reset legacy projection cursor state");
        sqlx::query("UPDATE summary_source_change_cursor SET cursor = 1")
            .execute(&pool)
            .await
            .expect("advance legacy Summary source cursor state");
        sqlx::query(
            "UPDATE analytics_change_state SET historical_history_unknown = 0, historical_history_initialized = 0 WHERE id = 1",
        )
        .execute(&pool)
        .await
        .expect("reset legacy history migration marker for source cursor");
        ensure_analytics_changes_schema(&pool)
            .await
            .expect("retry legacy source cursor migration");
        let markers: (i64, i64) = sqlx::query_as(
            "SELECT historical_history_unknown, historical_history_initialized FROM analytics_change_state WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("load retried source cursor history markers");
        assert_eq!(markers, (1, 1));
    }

    #[tokio::test]
    async fn historical_history_migration_marks_existing_aggregate_data_unknown() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect legacy aggregate history test database");
        sqlx::query(
            r#"
            CREATE TABLE analytics_change_state (
                id INTEGER PRIMARY KEY CHECK(id = 1),
                generation INTEGER NOT NULL DEFAULT 1,
                coverage_revision INTEGER NOT NULL DEFAULT 0,
                retained_from_change_id INTEGER NOT NULL DEFAULT 1,
                capacity INTEGER NOT NULL DEFAULT 10000,
                invalidation_pending INTEGER NOT NULL DEFAULT 0 CHECK(invalidation_pending IN (0, 1)),
                historical_replay_required INTEGER NOT NULL DEFAULT 0 CHECK(historical_replay_required IN (0, 1)),
                historical_history_unknown INTEGER NOT NULL DEFAULT 0 CHECK(historical_history_unknown IN (0, 1)),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create legacy aggregate history state");
        sqlx::query("INSERT INTO analytics_change_state (id) VALUES (1)")
            .execute(&pool)
            .await
            .expect("seed legacy aggregate history state");
        sqlx::query(
            "CREATE TABLE long_term_usage_daily (stats_date TEXT NOT NULL, dimension TEXT NOT NULL, series_key TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .expect("create legacy aggregate table");
        sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key) VALUES ('2026-10-30', 'overall', 'all')",
        )
        .execute(&pool)
        .await
        .expect("seed legacy aggregate row");
        sqlx::query("CREATE TABLE summary_all_time_projection_checkpoint (scope TEXT PRIMARY KEY)")
            .execute(&pool)
            .await
            .expect("create legacy Summary checkpoint table");
        sqlx::query(
            "INSERT INTO summary_all_time_projection_checkpoint (scope) VALUES ('all-time')",
        )
        .execute(&pool)
        .await
        .expect("seed legacy Summary checkpoint row");
        sqlx::query(
            "CREATE TABLE archive_batches (dataset TEXT NOT NULL, status TEXT NOT NULL, summary_source_kind TEXT)",
        )
        .execute(&pool)
        .await
        .expect("create legacy archive metadata table");
        sqlx::query(
            "INSERT INTO archive_batches (dataset, status, summary_source_kind) VALUES ('codex_invocations', 'completed', 'live_mirror')",
        )
        .execute(&pool)
        .await
        .expect("seed legacy invocation archive metadata");

        ensure_analytics_changes_schema(&pool)
            .await
            .expect("migrate legacy aggregate history state");

        let markers: (i64, i64) = sqlx::query_as(
            "SELECT historical_history_unknown, historical_history_initialized FROM analytics_change_state WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("load migrated aggregate history markers");
        assert_eq!(markers, (1, 1));
    }

    #[tokio::test]
    async fn source_change_deduplication_only_suppresses_adjacent_identical_facts() {
        let pool = test_pool().await;
        let fact_a = json!({"factVersion": 1, "status": "running"}).to_string();
        let fact_b = json!({"factVersion": 1, "status": "success"}).to_string();

        let first = append_test_fact(&pool, &fact_a).await;
        let duplicate = append_test_fact(&pool, &fact_a).await;
        let replacement = append_test_fact(&pool, &fact_b).await;
        let restored = append_test_fact(&pool, &fact_a).await;

        assert_eq!(duplicate.change_id, first.change_id);
        assert_eq!(replacement.change_id, first.change_id + 1);
        assert_eq!(restored.change_id, replacement.change_id + 1);
        let event_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM analytics_change_events")
            .fetch_one(&pool)
            .await
            .expect("count adjacent source changes");
        assert_eq!(event_count, 3);
    }

    #[tokio::test]
    async fn invocation_replacement_attempt_writers_and_archive_replacement_publish_structured_events()
     {
        let pool = test_pool().await;
        let invocation_id = insert_invocation(&pool, "replacement", "running").await;
        let mut tx = pool
            .begin()
            .await
            .expect("begin initial source transaction");
        append_invocation_upsert_for_row_tx(tx.as_mut(), invocation_id)
            .await
            .expect("append initial invocation event");
        tx.commit()
            .await
            .expect("commit initial source transaction");

        sqlx::query(
            "UPDATE codex_invocations SET status = 'success', total_tokens = 11 WHERE id = ?1",
        )
        .bind(invocation_id)
        .execute(&pool)
        .await
        .expect("update invocation fixture");
        let mut tx = pool.begin().await.expect("begin correction transaction");
        append_invocation_upsert_for_row_tx(tx.as_mut(), invocation_id)
            .await
            .expect("append correction event");
        tx.commit().await.expect("commit correction transaction");

        let attempt_id = sqlx::query(
            r#"
            INSERT INTO pool_upstream_request_attempts (
                attempt_public_id, invoke_id, occurred_at, endpoint, route_mode,
                attempt_index, distinct_account_index, same_account_retry_index, status, phase
            ) VALUES ('attempt-replacement', 'replacement', '2026-10-30 12:00:00',
                      '/v1/responses', 'pool', 0, 0, 0, 'pending', 'sending_request')
            "#,
        )
        .execute(&pool)
        .await
        .expect("insert attempt fixture")
        .last_insert_rowid();
        let mut tx = pool.begin().await.expect("begin attempt transaction");
        append_attempt_upsert_for_row_tx(tx.as_mut(), attempt_id)
            .await
            .expect("append attempt event");
        tx.commit().await.expect("commit attempt transaction");

        sqlx::query("UPDATE pool_upstream_request_attempts SET status = 'success', phase = 'completed' WHERE id = ?1")
            .bind(attempt_id)
            .execute(&pool)
            .await
            .expect("update attempt fixture");
        let mut tx = pool
            .begin()
            .await
            .expect("begin attempt correction transaction");
        append_attempt_upsert_for_row_tx(tx.as_mut(), attempt_id)
            .await
            .expect("append attempt correction event");
        tx.commit()
            .await
            .expect("commit attempt correction transaction");

        sqlx::query(
            r#"
            INSERT INTO archive_batches (
                dataset, month_key, file_path, sha256, row_count, status,
                summary_source_kind, coverage_start_at, coverage_end_at
            ) VALUES ('pool_upstream_request_attempts', '2026-10', '/tmp/archive-a.gz', 'hash-a', 1,
                      'completed', 'unknown', '2026-10-01 00:00:00', '2026-10-31 23:59:59')
            "#,
        )
        .execute(&pool)
        .await
        .expect("insert archive fixture");
        let mut tx = pool.begin().await.expect("begin archive transaction");
        append_archive_coverage_for_file_tx(tx.as_mut(), "/tmp/archive-a.gz")
            .await
            .expect("append archive coverage event");
        tx.commit().await.expect("commit archive transaction");

        sqlx::query("UPDATE archive_batches SET sha256 = 'hash-b', row_count = 2 WHERE file_path = '/tmp/archive-a.gz'")
            .execute(&pool)
            .await
            .expect("replace archive fixture");
        let mut tx = pool
            .begin()
            .await
            .expect("begin archive replacement transaction");
        append_archive_coverage_for_file_tx(tx.as_mut(), "/tmp/archive-a.gz")
            .await
            .expect("append archive replacement event");
        tx.commit()
            .await
            .expect("commit archive replacement transaction");

        let counts = sqlx::query_as::<_, (String, i64)>(
            "SELECT event_kind, COUNT(*) FROM analytics_change_events GROUP BY event_kind ORDER BY event_kind",
        )
        .fetch_all(&pool)
        .await
        .expect("load event counts");
        assert_eq!(
            counts,
            vec![("coverage".to_string(), 2), ("upsert".to_string(), 4)]
        );
        let fact_json: String = sqlx::query_scalar(
            "SELECT fact_json FROM analytics_change_events WHERE entity_kind = 'invocation' ORDER BY change_id DESC LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("load invocation fact");
        let fact: Value = serde_json::from_str(&fact_json).expect("parse invocation fact");
        assert_eq!(fact["status"], "success");
        assert!(!fact_json.contains("raw_response"));
    }

    #[tokio::test]
    async fn invocation_archive_coverage_requires_historical_replay() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, summary_source_kind) VALUES ('codex_invocations', '2026-10', '/tmp/archive-invocations.gz', 'hash', 1, 'completed', 'unknown')",
        )
        .execute(&pool)
        .await
        .expect("insert invocation archive fixture");

        let mut tx = pool
            .begin()
            .await
            .expect("begin archive coverage transaction");
        append_archive_coverage_for_file_tx(tx.as_mut(), "/tmp/archive-invocations.gz")
            .await
            .expect("append invocation archive coverage event");
        tx.commit()
            .await
            .expect("commit archive coverage transaction");

        let replay_required: i64 = sqlx::query_scalar(
            "SELECT historical_replay_required FROM analytics_change_state WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("load historical replay requirement");
        assert_eq!(replay_required, 1);
    }

    #[tokio::test]
    async fn business_delete_publishes_tombstone_but_archive_deletion_does_not() {
        let pool = test_pool().await;
        let invocation_id = insert_invocation(&pool, "delete-me", "success").await;
        let mut tx = pool.begin().await.expect("begin delete transaction");
        delete_invocation_with_analytics_tombstone_tx(tx.as_mut(), invocation_id)
            .await
            .expect("delete invocation with tombstone");
        tx.commit().await.expect("commit delete transaction");

        sqlx::query(
            "INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status) VALUES ('codex_invocations', '2026-10', '/tmp/archive-delete.gz', 'hash', 1, 'completed')",
        )
        .execute(&pool)
        .await
        .expect("insert archive deletion fixture");
        sqlx::query("DELETE FROM archive_batches WHERE file_path = '/tmp/archive-delete.gz'")
            .execute(&pool)
            .await
            .expect("delete archive metadata fixture");

        let event_kinds: Vec<String> =
            sqlx::query_scalar("SELECT event_kind FROM analytics_change_events ORDER BY change_id")
                .fetch_all(&pool)
                .await
                .expect("load tombstone events");
        assert_eq!(event_kinds, vec!["tombstone"]);
        let entity_key: String =
            sqlx::query_scalar("SELECT entity_key FROM analytics_change_events LIMIT 1")
                .fetch_one(&pool)
                .await
                .expect("load tombstone entity key");
        assert_eq!(entity_key, format!("invocation:proxy:{invocation_id}"));
    }

    #[tokio::test]
    async fn capacity_invalidation_blocks_stale_claim_until_generation_acknowledgement() {
        let pool = test_pool().await;
        sqlx::query("UPDATE analytics_change_state SET capacity = 2 WHERE id = 1")
            .execute(&pool)
            .await
            .expect("set bounded capacity");
        for index in 0..2 {
            let mut tx = pool.begin().await.expect("begin bounded event transaction");
            append_analytics_change_tx(
                tx.as_mut(),
                AnalyticsChangeInput {
                    event_kind: "upsert",
                    entity_kind: "test",
                    source: "test",
                    source_row_id: Some(index),
                    entity_key: &format!("test:{index}"),
                    occurred_at: "2026-10-30 12:00:00",
                    fact_json: &json!({"factVersion": 1, "index": index}).to_string(),
                },
            )
            .await
            .expect("append bounded event");
            tx.commit().await.expect("commit bounded event");
        }
        assert!(
            claim_analytics_change_coverage(&pool, "gap", 1, 2, 2, 2)
                .await
                .is_err()
        );
        claim_analytics_change_coverage(&pool, "replica", 1, 2, 1, 0)
            .await
            .expect("record lagging replica coverage");
        let mut tx = pool.begin().await.expect("begin bounded event transaction");
        append_analytics_change_tx(
            tx.as_mut(),
            AnalyticsChangeInput {
                event_kind: "upsert",
                entity_kind: "test",
                source: "test",
                source_row_id: Some(2),
                entity_key: "test:2",
                occurred_at: "2026-10-30 12:00:00",
                fact_json: &json!({"factVersion": 1, "index": 2}).to_string(),
            },
        )
        .await
        .expect("append bounded event");
        tx.commit().await.expect("commit bounded event");
        let (generation, revision, pending, retained_from, count): (i64, i64, i64, i64, i64) =
            sqlx::query_as(
                "SELECT generation, coverage_revision, invalidation_pending, retained_from_change_id, (SELECT COUNT(*) FROM analytics_change_events) FROM analytics_change_state WHERE id = 1",
            )
            .fetch_one(&pool)
            .await
            .expect("load bounded state");
        assert!(generation > 1);
        assert_eq!(revision, 3);
        assert_eq!(pending, 1);
        assert_eq!(retained_from, 2);
        assert_eq!(count, 2);
        assert!(
            claim_analytics_change_coverage(&pool, "replica", generation, revision, 2, 3)
                .await
                .is_err()
        );
        acknowledge_analytics_change_generation(&pool, "replica", generation, 3)
            .await
            .expect("acknowledge rebuilt generation");
        claim_analytics_change_coverage(&pool, "replica", generation, revision, 2, 3)
            .await
            .expect("claim current retained generation");
        claim_analytics_change_coverage(&pool, "replica", generation, revision, 2, 2)
            .await
            .expect("replay an already covered range");
        let covered_through: i64 = sqlx::query_scalar(
            "SELECT covered_through_change_id FROM analytics_change_coverage WHERE replica_id = 'replica'",
        )
        .fetch_one(&pool)
        .await
        .expect("load monotonic coverage cursor");
        assert_eq!(covered_through, 3);

        let mut tx = pool
            .begin()
            .await
            .expect("begin post-ack event transaction");
        append_analytics_change_tx(
            tx.as_mut(),
            AnalyticsChangeInput {
                event_kind: "upsert",
                entity_kind: "test",
                source: "test",
                source_row_id: Some(3),
                entity_key: "test:3",
                occurred_at: "2026-10-30 12:00:00",
                fact_json: &json!({"factVersion": 1, "index": 3}).to_string(),
            },
        )
        .await
        .expect("append post-ack bounded event");
        tx.commit().await.expect("commit post-ack bounded event");
        let (generation_after_ack, pending_after_ack, retained_after_ack): (i64, i64, i64) =
            sqlx::query_as(
                "SELECT generation, invalidation_pending, retained_from_change_id FROM analytics_change_state WHERE id = 1",
            )
            .fetch_one(&pool)
            .await
            .expect("load post-ack bounded state");
        assert_eq!(generation_after_ack, generation);
        assert_eq!(pending_after_ack, 0);
        assert_eq!(retained_after_ack, 3);
    }
}
