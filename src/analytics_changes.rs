use super::*;

pub(crate) const ANALYTICS_SOURCE_EVENT_VERSION: i64 = 1;
pub(crate) const ANALYTICS_SOURCE_FACT_DATE_SEMANTICS_VERSION: i64 = 2;
const ANALYTICS_CHANGE_CAPACITY_DEFAULT: i64 = 10_000;
const ANALYTICS_CHANGE_BYTES_CAPACITY_DEFAULT: i64 = 64 * 1024 * 1024;
const ANALYTICS_CHANGE_CAPACITY_SAFETY_MARGIN_PERCENT: i64 = 25;
const ANALYTICS_CHANGE_WARNING_PERCENT: i64 = 80;
const ANALYTICS_CHANGE_ROLLBACK_WINDOW_SECS: i64 = 24 * 60 * 60;
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

#[derive(Debug, Clone, FromRow)]
struct AnalyticsChangeStateRow {
    generation: i64,
    coverage_revision: i64,
    retained_from_change_id: i64,
    capacity: i64,
    invalidation_pending: i64,
    historical_replay_required: i64,
    owner_kind: String,
    owner_epoch: i64,
    serving_fence: i64,
    writer_id: Option<String>,
    writer_lease_expires_at: Option<String>,
    serving_ack_change_id: i64,
    retention_floor_change_id: i64,
    journal_event_capacity: i64,
    journal_byte_capacity: i64,
    journal_peak_events: i64,
    journal_peak_bytes: i64,
    journal_warning: i64,
    journal_backpressure: i64,
    analytics_unavailable: i64,
    handoff_state: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct AnalyticsServingFence {
    pub(crate) owner_kind: &'static str,
    pub(crate) owner_epoch: i64,
    pub(crate) serving_fence: i64,
    pub(crate) generation: i64,
    pub(crate) serving_ack_change_id: i64,
    pub(crate) retention_floor_change_id: i64,
    pub(crate) unavailable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct AnalyticsJournalCapacity {
    pub(crate) event_capacity: i64,
    pub(crate) byte_capacity: i64,
    pub(crate) peak_events: i64,
    pub(crate) peak_bytes: i64,
    pub(crate) warning: bool,
    pub(crate) backpressure: bool,
}

pub(crate) fn calculate_analytics_journal_capacity(
    peak_events: i64,
    peak_bytes: i64,
) -> AnalyticsJournalCapacity {
    let margin = 100 + ANALYTICS_CHANGE_CAPACITY_SAFETY_MARGIN_PERCENT;
    let event_capacity = peak_events.max(1).saturating_mul(margin).saturating_add(99) / 100;
    let byte_capacity = peak_bytes.max(1).saturating_mul(margin).saturating_add(99) / 100;
    AnalyticsJournalCapacity {
        event_capacity: event_capacity.max(ANALYTICS_CHANGE_CAPACITY_DEFAULT),
        byte_capacity: byte_capacity.max(ANALYTICS_CHANGE_BYTES_CAPACITY_DEFAULT),
        peak_events,
        peak_bytes,
        warning: false,
        backpressure: false,
    }
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
    upstream_account_id: Option<i64>,
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
    error_message: Option<String>,
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
            fact_date_semantics_version INTEGER NOT NULL DEFAULT 1,
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

    let fact_date_semantics_column_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('analytics_change_events') WHERE name = 'fact_date_semantics_version')",
    )
    .fetch_one(pool)
    .await
    .context("failed to inspect analytics change event compatibility columns")?
        != 0;
    if !fact_date_semantics_column_exists {
        sqlx::query(
            "ALTER TABLE analytics_change_events ADD COLUMN fact_date_semantics_version INTEGER NOT NULL DEFAULT 1",
        )
        .execute(pool)
        .await
        .context("failed to add analytics change event fact-date semantics version")?;
    }

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
            owner_kind TEXT NOT NULL DEFAULT 'sqlite' CHECK(owner_kind IN ('sqlite', 'postgres')),
            owner_epoch INTEGER NOT NULL DEFAULT 1 CHECK(owner_epoch > 0),
            serving_fence INTEGER NOT NULL DEFAULT 1 CHECK(serving_fence > 0),
            writer_id TEXT,
            writer_lease_expires_at TEXT,
            serving_ack_change_id INTEGER NOT NULL DEFAULT 0 CHECK(serving_ack_change_id >= 0),
            retention_floor_change_id INTEGER NOT NULL DEFAULT 1 CHECK(retention_floor_change_id >= 1),
            journal_event_capacity INTEGER NOT NULL DEFAULT 10000 CHECK(journal_event_capacity > 0),
            journal_byte_capacity INTEGER NOT NULL DEFAULT 67108864 CHECK(journal_byte_capacity > 0),
            journal_peak_events INTEGER NOT NULL DEFAULT 0 CHECK(journal_peak_events >= 0),
            journal_peak_bytes INTEGER NOT NULL DEFAULT 0 CHECK(journal_peak_bytes >= 0),
            journal_warning INTEGER NOT NULL DEFAULT 0 CHECK(journal_warning IN (0, 1)),
            journal_backpressure INTEGER NOT NULL DEFAULT 0 CHECK(journal_backpressure IN (0, 1)),
            analytics_unavailable INTEGER NOT NULL DEFAULT 0 CHECK(analytics_unavailable IN (0, 1)),
            handoff_state TEXT NOT NULL DEFAULT 'stable' CHECK(handoff_state IN ('stable', 'handoff', 'rollback', 'unavailable')),
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

    // These columns were added after the first source-journal release. Keep the migration
    // additive so older SQLite databases can restart safely before the new control plane is used.
    for (name, definition) in [
        ("owner_kind", "TEXT NOT NULL DEFAULT 'sqlite'"),
        ("owner_epoch", "INTEGER NOT NULL DEFAULT 1"),
        ("serving_fence", "INTEGER NOT NULL DEFAULT 1"),
        ("writer_id", "TEXT"),
        ("writer_lease_expires_at", "TEXT"),
        ("serving_ack_change_id", "INTEGER NOT NULL DEFAULT 0"),
        ("retention_floor_change_id", "INTEGER NOT NULL DEFAULT 1"),
        ("journal_event_capacity", "INTEGER NOT NULL DEFAULT 10000"),
        ("journal_byte_capacity", "INTEGER NOT NULL DEFAULT 67108864"),
        ("journal_peak_events", "INTEGER NOT NULL DEFAULT 0"),
        ("journal_peak_bytes", "INTEGER NOT NULL DEFAULT 0"),
        ("journal_warning", "INTEGER NOT NULL DEFAULT 0"),
        ("journal_backpressure", "INTEGER NOT NULL DEFAULT 0"),
        ("analytics_unavailable", "INTEGER NOT NULL DEFAULT 0"),
        ("handoff_state", "TEXT NOT NULL DEFAULT 'stable'"),
    ] {
        let exists = sqlx::query_scalar::<_, i64>(&format!(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('analytics_change_state') WHERE name = '{name}')"
        ))
        .fetch_one(pool)
        .await
        .with_context(|| format!("failed to inspect analytics ownership column {name}"))?
            != 0;
        if !exists {
            sqlx::query(&format!(
                "ALTER TABLE analytics_change_state ADD COLUMN {name} {definition}"
            ))
            .execute(pool)
            .await
            .with_context(|| format!("failed to add analytics ownership column {name}"))?;
        }
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
        CREATE TABLE IF NOT EXISTS analytics_change_consumption (
            replica_id TEXT PRIMARY KEY,
            generation INTEGER NOT NULL,
            consumed_through_change_id INTEGER NOT NULL DEFAULT 0 CHECK(consumed_through_change_id >= 0),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(pool)
    .await
    .context("failed to ensure analytics change consumption table")?;

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

    sqlx::query(
        r#"
        UPDATE analytics_change_state
        SET journal_event_capacity = CASE WHEN journal_event_capacity > 0 THEN journal_event_capacity ELSE ?1 END,
            journal_byte_capacity = CASE WHEN journal_byte_capacity > 0 THEN journal_byte_capacity ELSE ?2 END,
            retention_floor_change_id = CASE WHEN retention_floor_change_id > 0 THEN retention_floor_change_id ELSE 1 END
        WHERE id = 1
        "#,
    )
    .bind(ANALYTICS_CHANGE_CAPACITY_DEFAULT)
    .bind(ANALYTICS_CHANGE_BYTES_CAPACITY_DEFAULT)
    .execute(pool)
    .await
    .context("failed to seed analytics ownership capacity")?;

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
        "CREATE INDEX IF NOT EXISTS idx_analytics_change_consumption_generation ON analytics_change_consumption (generation, consumed_through_change_id)",
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

pub(crate) fn normalize_analytics_prompt_cache_key_digest(
    payload: Option<&Value>,
) -> Option<String> {
    payload
        .and_then(|value| value.get("promptCacheKey"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("sha256:{:x}", Sha256::digest(value.as_bytes())))
}

pub(crate) fn normalize_analytics_failure_kind(
    payload: Option<&Value>,
    column_value: Option<&str>,
) -> Option<String> {
    payload_text(payload, "failureKind")
        .or_else(|| bounded_text(column_value))
        .filter(|value| !value.eq_ignore_ascii_case("none"))
}

pub(crate) fn normalize_analytics_classification_error_message(
    payload: Option<&Value>,
    column_value: Option<&str>,
) -> Option<String> {
    let column_value = bounded_text(column_value);
    let downstream_value = payload_text(payload, "downstreamErrorMessage");
    let combined = match (column_value, downstream_value) {
        (Some(column_value), Some(downstream_value)) => {
            Some(format!("{downstream_value} {column_value}"))
        }
        (Some(column_value), None) => Some(column_value),
        (None, Some(downstream_value)) => Some(downstream_value),
        (None, None) => None,
    }?;
    bounded_text(Some(combined.as_str()))
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

async fn load_analytics_change_state(tx: &mut SqliteConnection) -> Result<AnalyticsChangeStateRow> {
    sqlx::query_as::<_, AnalyticsChangeStateRow>(
        r#"
        SELECT generation, coverage_revision, retained_from_change_id, capacity,
               invalidation_pending, historical_replay_required,
               owner_kind, owner_epoch, serving_fence, writer_id,
               writer_lease_expires_at, serving_ack_change_id,
               retention_floor_change_id, journal_event_capacity,
               journal_byte_capacity, journal_peak_events, journal_peak_bytes,
               journal_warning, journal_backpressure, analytics_unavailable,
               handoff_state
        FROM analytics_change_state
        WHERE id = 1
        "#,
    )
    .fetch_one(&mut *tx)
    .await
    .context("analytics change state is not initialized")
}

async fn refresh_analytics_journal_pressure_tx(
    tx: &mut SqliteConnection,
    state: &AnalyticsChangeStateRow,
) -> Result<AnalyticsJournalCapacity> {
    let event_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM analytics_change_events")
        .fetch_one(&mut *tx)
        .await?;
    let byte_count = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(length(CAST(fact_json AS BLOB))), 0) FROM analytics_change_events",
    )
    .fetch_one(&mut *tx)
    .await?;
    let warning = event_count.saturating_mul(100)
        >= state
            .journal_event_capacity
            .saturating_mul(ANALYTICS_CHANGE_WARNING_PERCENT)
        || byte_count.saturating_mul(100)
            >= state
                .journal_byte_capacity
                .saturating_mul(ANALYTICS_CHANGE_WARNING_PERCENT);
    let backpressure =
        event_count > state.journal_event_capacity || byte_count > state.journal_byte_capacity;
    sqlx::query(
        r#"
        UPDATE analytics_change_state
        SET journal_peak_events = MAX(journal_peak_events, ?1),
            journal_peak_bytes = MAX(journal_peak_bytes, ?2),
            journal_warning = ?3,
            journal_backpressure = ?4,
            analytics_unavailable = CASE WHEN ?4 <> 0 THEN 1 ELSE analytics_unavailable END,
            updated_at = datetime('now')
        WHERE id = 1
        "#,
    )
    .bind(event_count)
    .bind(byte_count)
    .bind(i64::from(warning))
    .bind(i64::from(backpressure))
    .execute(&mut *tx)
    .await?;
    Ok(AnalyticsJournalCapacity {
        event_capacity: state.journal_event_capacity,
        byte_capacity: state.journal_byte_capacity,
        peak_events: event_count,
        peak_bytes: byte_count,
        warning,
        backpressure,
    })
}

async fn enforce_analytics_journal_capacity_tx(
    tx: &mut SqliteConnection,
    state: &AnalyticsChangeStateRow,
    next_fact_bytes: i64,
) -> Result<()> {
    let event_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM analytics_change_events")
        .fetch_one(&mut *tx)
        .await?;
    let byte_count = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(length(CAST(fact_json AS BLOB))), 0) FROM analytics_change_events",
    )
    .fetch_one(&mut *tx)
    .await?;
    if event_count.saturating_add(1) <= state.journal_event_capacity
        && byte_count.saturating_add(next_fact_bytes) <= state.journal_byte_capacity
    {
        return Ok(());
    }
    let consumed_floor = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COALESCE(
            MIN(consumed_through_change_id),
            (SELECT MIN(covered_through_change_id)
             FROM analytics_change_coverage WHERE generation = ?1),
            0
        )
        FROM analytics_change_consumption WHERE generation = ?1
        "#,
    )
    .bind(state.generation)
    .fetch_one(&mut *tx)
    .await?;
    if consumed_floor < 1 {
        sqlx::query(
            "UPDATE analytics_change_state SET journal_warning = 1, journal_backpressure = 1, analytics_unavailable = 1, updated_at = datetime('now') WHERE id = 1",
        )
        .execute(&mut *tx)
        .await?;
        bail!("analytics source journal capacity exhausted; backpressure active");
    }
    let evictable_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM analytics_change_events WHERE change_id <= ?1",
    )
    .bind(consumed_floor)
    .fetch_one(&mut *tx)
    .await?;
    let evictable_bytes = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(length(CAST(fact_json AS BLOB))), 0) FROM analytics_change_events WHERE change_id <= ?1",
    )
    .bind(consumed_floor)
    .fetch_one(&mut *tx)
    .await?;
    if event_count
        .saturating_sub(evictable_count)
        .saturating_add(1)
        > state.journal_event_capacity
        || byte_count
            .saturating_sub(evictable_bytes)
            .saturating_add(next_fact_bytes)
            > state.journal_byte_capacity
    {
        sqlx::query(
            "UPDATE analytics_change_state SET journal_warning = 1, journal_backpressure = 1, analytics_unavailable = 1, updated_at = datetime('now') WHERE id = 1",
        )
        .execute(&mut *tx)
        .await?;
        bail!("analytics source journal capacity exhausted; backpressure active");
    }
    sqlx::query("DELETE FROM analytics_change_events WHERE change_id <= ?1")
        .bind(consumed_floor)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE analytics_change_state SET retained_from_change_id = MAX(retained_from_change_id, ?1), retention_floor_change_id = MAX(retention_floor_change_id, ?1), updated_at = datetime('now') WHERE id = 1",
    )
    .bind(consumed_floor.saturating_add(1))
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn load_analytics_serving_fence(
    pool: &Pool<Sqlite>,
) -> Result<AnalyticsServingFence> {
    let state = sqlx::query_as::<_, AnalyticsChangeStateRow>(
        r#"
        SELECT generation, coverage_revision, retained_from_change_id, capacity,
               invalidation_pending, historical_replay_required,
               owner_kind, owner_epoch, serving_fence, writer_id,
               writer_lease_expires_at, serving_ack_change_id,
               retention_floor_change_id, journal_event_capacity,
               journal_byte_capacity, journal_peak_events, journal_peak_bytes,
               journal_warning, journal_backpressure, analytics_unavailable,
               handoff_state
        FROM analytics_change_state WHERE id = 1
        "#,
    )
    .fetch_one(pool)
    .await
    .context("analytics serving fence is not initialized")?;
    Ok(AnalyticsServingFence {
        owner_kind: match state.owner_kind.as_str() {
            "postgres" => "postgres",
            _ => "sqlite",
        },
        owner_epoch: state.owner_epoch,
        serving_fence: state.serving_fence,
        generation: state.generation,
        serving_ack_change_id: state.serving_ack_change_id,
        retention_floor_change_id: state.retention_floor_change_id,
        unavailable: state.analytics_unavailable != 0
            || state.journal_backpressure != 0
            || state.handoff_state == "unavailable",
    })
}

pub(crate) async fn claim_analytics_owner(
    pool: &Pool<Sqlite>,
    owner_kind: &str,
    writer_id: &str,
    expected_owner_epoch: i64,
    expected_serving_fence: i64,
) -> Result<AnalyticsServingFence> {
    if !matches!(owner_kind, "sqlite" | "postgres") {
        bail!("unsupported analytics owner: {owner_kind}");
    }
    let writer_id = bounded_required_text(writer_id, "analytics writer id")?;
    let mut tx = pool.begin().await?;
    let state = load_analytics_change_state(tx.as_mut()).await?;
    if state.owner_epoch != expected_owner_epoch {
        bail!("analytics owner epoch is stale");
    }
    if state.serving_fence != expected_serving_fence {
        bail!("analytics serving fence is stale");
    }
    if state.writer_id.as_deref().is_some_and(|id| id != writer_id) {
        bail!("analytics writer is already owned");
    }
    let next_epoch = state
        .owner_epoch
        .checked_add(1)
        .context("analytics owner epoch overflow")?;
    let next_fence = state
        .serving_fence
        .checked_add(1)
        .context("analytics serving fence overflow")?;
    sqlx::query(
        r#"
        UPDATE analytics_change_state
        SET owner_kind = ?1,
            owner_epoch = ?2,
            serving_fence = ?3,
            writer_id = ?4,
            writer_lease_expires_at = NULL,
            handoff_state = 'stable',
            analytics_unavailable = 0,
            journal_backpressure = 0,
            updated_at = datetime('now')
        WHERE id = 1 AND owner_epoch = ?5 AND serving_fence = ?6
        "#,
    )
    .bind(owner_kind)
    .bind(next_epoch)
    .bind(next_fence)
    .bind(&writer_id)
    .bind(expected_owner_epoch)
    .bind(expected_serving_fence)
    .execute(tx.as_mut())
    .await?;
    tx.commit().await?;
    load_analytics_serving_fence(pool).await
}

pub(crate) async fn release_analytics_owner(
    pool: &Pool<Sqlite>,
    writer_id: &str,
    expected_owner_epoch: i64,
    expected_serving_fence: i64,
) -> Result<AnalyticsServingFence> {
    let writer_id = bounded_required_text(writer_id, "analytics writer id")?;
    let result = sqlx::query(
        r#"
        UPDATE analytics_change_state
        SET writer_id = NULL,
            writer_lease_expires_at = NULL,
            handoff_state = 'unavailable',
            analytics_unavailable = 1,
            serving_fence = serving_fence + 1,
            updated_at = datetime('now')
        WHERE id = 1 AND writer_id = ?1 AND owner_epoch = ?2 AND serving_fence = ?3
        "#,
    )
    .bind(&writer_id)
    .bind(expected_owner_epoch)
    .bind(expected_serving_fence)
    .execute(pool)
    .await?;
    if result.rows_affected() != 1 {
        bail!("analytics owner release fence is stale");
    }
    load_analytics_serving_fence(pool).await
}

pub(crate) async fn acknowledge_analytics_journal_consumption(
    pool: &Pool<Sqlite>,
    replica_id: &str,
    generation: i64,
    consumed_through_change_id: i64,
) -> Result<AnalyticsServingFence> {
    let replica_id = bounded_required_text(replica_id, "replica id")?;
    let mut tx = pool.begin().await?;
    let state = load_analytics_change_state(tx.as_mut()).await?;
    if state.generation != generation {
        bail!("analytics journal consumption generation is stale");
    }
    let latest = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(change_id), 0) FROM analytics_change_events",
    )
    .fetch_one(tx.as_mut())
    .await?;
    if consumed_through_change_id < 0 || consumed_through_change_id > latest {
        bail!("analytics journal consumption is outside the source journal");
    }
    sqlx::query(
        r#"
        INSERT INTO analytics_change_consumption (replica_id, generation, consumed_through_change_id)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(replica_id) DO UPDATE SET
            generation = excluded.generation,
            consumed_through_change_id = MAX(analytics_change_consumption.consumed_through_change_id, excluded.consumed_through_change_id),
            updated_at = datetime('now')
        "#,
    )
    .bind(&replica_id)
    .bind(generation)
    .bind(consumed_through_change_id)
    .execute(tx.as_mut())
    .await?;
    sqlx::query(
        r#"
        UPDATE analytics_change_state
        SET serving_ack_change_id = MAX(serving_ack_change_id, ?1),
            retention_floor_change_id = MAX(retention_floor_change_id, ?1),
            updated_at = datetime('now')
        WHERE id = 1 AND generation = ?2
        "#,
    )
    .bind(consumed_through_change_id)
    .bind(generation)
    .execute(tx.as_mut())
    .await?;
    tx.commit().await?;
    load_analytics_serving_fence(pool).await
}

pub(crate) async fn refresh_analytics_journal_capacity(
    pool: &Pool<Sqlite>,
) -> Result<AnalyticsJournalCapacity> {
    let mut tx = pool.begin().await?;
    let event_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM analytics_change_events WHERE created_at >= datetime('now', '-1 day')",
    )
    .fetch_one(tx.as_mut())
    .await?;
    let byte_count = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(length(CAST(fact_json AS BLOB))), 0) FROM analytics_change_events WHERE created_at >= datetime('now', '-1 day')",
    )
    .fetch_one(tx.as_mut())
    .await?;
    let mut capacity = calculate_analytics_journal_capacity(event_count, byte_count);
    capacity.warning = false;
    capacity.backpressure = false;
    sqlx::query(
        r#"
        UPDATE analytics_change_state
        SET journal_event_capacity = MAX(journal_event_capacity, ?1),
            journal_byte_capacity = MAX(journal_byte_capacity, ?2),
            journal_peak_events = MAX(journal_peak_events, ?3),
            journal_peak_bytes = MAX(journal_peak_bytes, ?4),
            updated_at = datetime('now')
        WHERE id = 1
        "#,
    )
    .bind(capacity.event_capacity)
    .bind(capacity.byte_capacity)
    .bind(event_count)
    .bind(byte_count)
    .execute(tx.as_mut())
    .await?;
    tx.commit().await?;
    Ok(capacity)
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
    if state.owner_kind == "postgres" {
        enforce_analytics_journal_capacity_tx(tx, &state, fact_json.len() as i64).await?;
    }
    let coverage_revision = state
        .coverage_revision
        .checked_add(1)
        .context("analytics change coverage revision overflow")?;
    let result = sqlx::query(
        r#"
        INSERT INTO analytics_change_events (
            event_version,
            fact_date_semantics_version,
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
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind(ANALYTICS_SOURCE_EVENT_VERSION)
    .bind(ANALYTICS_SOURCE_FACT_DATE_SEMANTICS_VERSION)
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

    let pressure = refresh_analytics_journal_pressure_tx(tx, &state).await?;
    if pressure.backpressure && state.owner_kind == "postgres" {
        bail!("analytics source journal capacity exhausted; backpressure active");
    }
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
    let attempt_upstream_account_id_sql =
        crate::api::invocation_upstream_account_id_from_attempt_fallback_sql("codex_invocations");
    let row_query = format!(
        r#"
        SELECT
            id,
            invoke_id,
            occurred_at,
            CAST(COALESCE(ROUND((julianday(
                occurred_at,
                CASE WHEN instr(occurred_at, 'T') > 0 THEN '+0 hours' ELSE '-8 hours' END
            ) - 2440587.5) * 86400000.0), 0) AS INTEGER) AS occurred_at_epoch_ms,
            date(
                occurred_at,
                CASE WHEN instr(occurred_at, 'T') > 0 THEN '+8 hours' ELSE '+0 hours' END
            ) AS stats_date,
            source,
            {attempt_upstream_account_id_sql} AS upstream_account_id,
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
            error_message,
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
        "#
    );
    let row = sqlx::query_as::<_, InvocationAnalyticsRow>(&row_query)
        .bind(row_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            anyhow!("cannot publish analytics invocation change for missing row {row_id}")
        })?;

    let payload = normalized_payload(row.payload.as_deref());
    let prompt_cache_key_digest = normalize_analytics_prompt_cache_key_digest(payload.as_ref());
    let request_model = payload_text(payload.as_ref(), "requestModel");
    let response_model = payload_text(payload.as_ref(), "responseModel");
    let reasoning_effort = payload_text(payload.as_ref(), "reasoningEffort");
    let upstream_account_name = payload_text(payload.as_ref(), "upstreamAccountName");
    let upstream_account_id =
        payload_i64(payload.as_ref(), "upstreamAccountId").or(row.upstream_account_id);
    let status = bounded_text(row.status.as_deref());
    let error_message = bounded_text(row.error_message.as_deref());
    let classification_error_message = normalize_analytics_classification_error_message(
        payload.as_ref(),
        row.error_message.as_deref(),
    );
    let stored_failure_kind =
        normalize_analytics_failure_kind(payload.as_ref(), row.failure_kind.as_deref());
    let stored_failure_class = bounded_text(row.failure_class.as_deref());
    let classification = crate::api::resolve_failure_classification(
        status.as_deref(),
        classification_error_message.as_deref(),
        stored_failure_kind.as_deref(),
        stored_failure_class.as_deref(),
        row.is_actionable,
    );
    let failure_kind = stored_failure_kind.or(classification.failure_kind.clone());
    let legacy_success_status = status.as_deref().is_some_and(|status| {
        matches!(
            status.to_ascii_lowercase().as_str(),
            "http_200" | "warning_success"
        )
    });
    let failure_class = match stored_failure_class {
        Some(_) => Some(classification.failure_class.as_str().to_string()),
        None => (classification.failure_class != crate::api::FailureClass::None
            || (legacy_success_status && error_message.as_deref().is_none_or(str::is_empty)))
        .then(|| classification.failure_class.as_str().to_string()),
    };
    let is_actionable = row
        .is_actionable
        .filter(|value| (*value != 0) == classification.is_actionable)
        .or_else(|| {
            (classification.failure_class != crate::api::FailureClass::None)
                .then_some(i64::from(classification.is_actionable))
        });
    let success_like = crate::api::prompt_invocation_status_is_success_like(
        status.as_deref(),
        error_message.as_deref(),
    ) && classification.failure_class == crate::api::FailureClass::None;
    let source = bounded_required_text(&row.source, "source")?;
    let invoke_id = bounded_required_text(&row.invoke_id, "invoke_id")?;
    let entity_key = format!("invocation:{}:{}", source, row.id);
    let fact_json = json!({
        "factVersion": ANALYTICS_SOURCE_EVENT_VERSION,
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
        "promptCacheKeyDigest": prompt_cache_key_digest,
        "upstreamAccountId": upstream_account_id,
        "upstreamAccountName": upstream_account_name,
        "status": status.clone(),
        "successLike": success_like,
        "terminal": terminal_status(status.as_deref()),
        "failureKind": failure_kind,
        "failureClass": failure_class,
        "isActionable": is_actionable.map(|value| value != 0),
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
        "factVersion": ANALYTICS_SOURCE_EVENT_VERSION,
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
        "factVersion": ANALYTICS_SOURCE_EVENT_VERSION,
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
    let receipt = append_analytics_change_tx(
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
    .await?;

    // Attempt rows can be corrected after the terminal invocation fact was published. Re-publish
    // the matching invocation so the projector can recompute account fallback and final timing.
    let exact_invocation_ids = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2 ORDER BY id",
    )
    .bind(&row.invoke_id)
    .bind(&row.occurred_at)
    .fetch_all(&mut *tx)
    .await?;
    let invocation_ids = if exact_invocation_ids.is_empty() {
        // Older source rows can carry equivalent timestamps with different textual precision.
        sqlx::query_scalar::<_, i64>(
            "SELECT id FROM codex_invocations WHERE invoke_id = ?1 ORDER BY id",
        )
        .bind(&row.invoke_id)
        .fetch_all(&mut *tx)
        .await?
    } else {
        exact_invocation_ids
    };
    for invocation_id in invocation_ids {
        append_invocation_upsert_for_row_tx(tx, invocation_id).await?;
    }
    Ok(receipt)
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
        "factVersion": ANALYTICS_SOURCE_EVENT_VERSION,
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
    sqlx::query(
        r#"
        INSERT INTO analytics_change_consumption (replica_id, generation, consumed_through_change_id)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(replica_id) DO UPDATE SET
            generation = excluded.generation,
            consumed_through_change_id = MAX(analytics_change_consumption.consumed_through_change_id, excluded.consumed_through_change_id),
            updated_at = datetime('now')
        "#,
    )
    .bind(&replica_id)
    .bind(state.generation)
    .bind(covered_through_change_id)
    .execute(tx.as_mut())
    .await?;
    sqlx::query(
        "UPDATE analytics_change_state SET serving_ack_change_id = MAX(serving_ack_change_id, ?1), retention_floor_change_id = MAX(retention_floor_change_id, ?1), updated_at = datetime('now') WHERE id = 1",
    )
    .bind(covered_through_change_id)
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

    #[test]
    fn classification_message_preserves_downstream_kind_before_long_column_error() {
        let payload = serde_json::json!({
            "downstreamErrorMessage": "upstream stream error"
        });
        let column_error = "x".repeat(ANALYTICS_CHANGE_TEXT_MAX_CHARS);
        let message = normalize_analytics_classification_error_message(
            Some(&payload),
            Some(column_error.as_str()),
        )
        .expect("classification message");
        assert!(message.starts_with("upstream stream error"));

        let classification = crate::api::resolve_failure_classification(
            Some("completed"),
            Some(message.as_str()),
            None,
            None,
            None,
        );
        assert_eq!(
            classification.failure_kind.as_deref(),
            Some("upstream_stream_error")
        );
        assert_eq!(
            classification.failure_class,
            crate::api::FailureClass::ServiceFailure
        );
    }

    async fn insert_invocation(pool: &Pool<Sqlite>, invoke_id: &str, status: &str) -> i64 {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            ) VALUES (?1, '2026-10-30 12:00:00', 'proxy', ?2, 7, 0.07,
                      '{"requestModel":"gpt-5"}', '{}')
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
    async fn invocation_upsert_uses_asia_shanghai_fact_date_at_utc_boundary() {
        let pool = test_pool().await;
        for (invoke_id, occurred_at, expected_stats_date) in [
            (
                "fact-date-before-boundary",
                "2026-10-30T15:59:59Z",
                "2026-10-30",
            ),
            (
                "fact-date-at-boundary",
                "2026-10-30T16:00:00Z",
                "2026-10-31",
            ),
            (
                "fact-date-naive-shanghai",
                "2026-10-31 00:00:00",
                "2026-10-31",
            ),
        ] {
            let row_id = sqlx::query(
                "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, raw_response) VALUES (?1, ?2, 'proxy', 'success', '{}')",
            )
            .bind(invoke_id)
            .bind(occurred_at)
            .execute(&pool)
            .await
            .expect("insert fact-date boundary invocation")
            .last_insert_rowid();

            let mut tx = pool
                .begin()
                .await
                .expect("begin fact-date boundary transaction");
            append_invocation_upsert_for_row_tx(tx.as_mut(), row_id)
                .await
                .expect("append fact-date boundary invocation event");
            tx.commit()
                .await
                .expect("commit fact-date boundary transaction");

            let (
                event_version,
                fact_date_semantics_version,
                event_kind,
                entity_kind,
                entity_key,
                event_occurred_at,
                fact_json,
            ): (i64, i64, String, String, String, String, String) =
                sqlx::query_as(
                    "SELECT event_version, fact_date_semantics_version, event_kind, entity_kind, entity_key, occurred_at, fact_json FROM analytics_change_events WHERE source_row_id = ?1",
                )
                .bind(row_id)
                .fetch_one(&pool)
                .await
                .expect("load fact-date boundary invocation event");
            assert_eq!(event_version, ANALYTICS_SOURCE_EVENT_VERSION);
            assert_eq!(
                fact_date_semantics_version,
                ANALYTICS_SOURCE_FACT_DATE_SEMANTICS_VERSION
            );
            assert_eq!(event_kind, "upsert");
            assert_eq!(entity_kind, "invocation");
            assert_eq!(entity_key, format!("invocation:proxy:{row_id}"));
            assert_eq!(event_occurred_at, occurred_at);

            let fact: Value =
                serde_json::from_str(&fact_json).expect("parse fact-date boundary invocation fact");
            assert_eq!(fact["factVersion"], 1);
            assert_eq!(fact["sourceRowId"], row_id);
            assert_eq!(fact["statsDate"], expected_stats_date);
        }
    }

    #[tokio::test]
    async fn invocation_upsert_resolves_attempt_account_and_legacy_failure_classification() {
        let pool = test_pool().await;
        let row_id = sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, error_message, failure_class, payload, raw_response
            ) VALUES ('incremental-attempt-account', '2026-10-30 12:00:00', 'proxy',
                      'completed', 'failed to contact upstream', 'none',
                      '{"upstreamAccountName":"attempt-account"}', '{}')
            "#,
        )
        .execute(&pool)
        .await
        .expect("insert incremental legacy invocation")
        .last_insert_rowid();
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_request_attempts (
                invoke_id, occurred_at, endpoint, route_mode, attempt_index,
                distinct_account_index, same_account_retry_index, upstream_account_id, status
            ) VALUES ('incremental-attempt-account', '2026-10-30 12:00:00', '/v1/responses',
                      'pool', 1, 1, 0, 42, 'success')
            "#,
        )
        .execute(&pool)
        .await
        .expect("insert incremental attempt account");

        let mut tx = pool
            .begin()
            .await
            .expect("begin incremental invocation transaction");
        append_invocation_upsert_for_row_tx(tx.as_mut(), row_id)
            .await
            .expect("append incremental invocation event");
        tx.commit()
            .await
            .expect("commit incremental invocation transaction");

        let fact_json: String = sqlx::query_scalar(
            "SELECT fact_json FROM analytics_change_events WHERE source_row_id = ?1",
        )
        .bind(row_id)
        .fetch_one(&pool)
        .await
        .expect("load incremental invocation fact");
        let fact: Value = serde_json::from_str(&fact_json).expect("parse incremental fact");
        assert_eq!(fact["upstreamAccountId"], 42);
        assert_eq!(fact["successLike"], false);
        assert_eq!(fact["failureClass"], "service_failure");
        assert_eq!(fact["isActionable"], true);
    }

    #[tokio::test]
    async fn analytics_change_schema_marks_legacy_events_and_current_writes() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect legacy analytics event test database");
        sqlx::query(
            r#"
            CREATE TABLE analytics_change_events (
                change_id INTEGER PRIMARY KEY AUTOINCREMENT,
                event_version INTEGER NOT NULL,
                event_kind TEXT NOT NULL,
                entity_kind TEXT NOT NULL,
                source TEXT NOT NULL,
                source_row_id INTEGER,
                entity_key TEXT NOT NULL,
                generation INTEGER NOT NULL,
                coverage_revision INTEGER NOT NULL,
                occurred_at TEXT NOT NULL,
                fact_json TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create legacy analytics event table");
        sqlx::query(
            r#"
            INSERT INTO analytics_change_events (
                event_version, event_kind, entity_kind, source, source_row_id,
                entity_key, generation, coverage_revision, occurred_at, fact_json
            ) VALUES (1, 'upsert', 'invocation', 'proxy', 1, 'legacy', 1, 1,
                      '2026-10-30 12:00:00', '{}')
            "#,
        )
        .execute(&pool)
        .await
        .expect("insert legacy analytics event");

        ensure_analytics_changes_schema(&pool)
            .await
            .expect("migrate legacy analytics event table");

        let legacy_version: i64 = sqlx::query_scalar(
            "SELECT fact_date_semantics_version FROM analytics_change_events WHERE entity_key = 'legacy'",
        )
        .fetch_one(&pool)
        .await
        .expect("load legacy event semantics version");
        assert_eq!(legacy_version, 1);

        let mut tx = pool
            .begin()
            .await
            .expect("begin current analytics event transaction");
        append_analytics_change_tx(
            tx.as_mut(),
            AnalyticsChangeInput {
                event_kind: "upsert",
                entity_kind: "invocation",
                source: "proxy",
                source_row_id: Some(2),
                entity_key: "current",
                occurred_at: "2026-10-30 12:00:01",
                fact_json: "{}",
            },
        )
        .await
        .expect("append current analytics event");
        tx.commit()
            .await
            .expect("commit current analytics event transaction");

        let current_version: i64 = sqlx::query_scalar(
            "SELECT fact_date_semantics_version FROM analytics_change_events WHERE entity_key = 'current'",
        )
        .fetch_one(&pool)
        .await
        .expect("load current event semantics version");
        assert_eq!(
            current_version,
            ANALYTICS_SOURCE_FACT_DATE_SEMANTICS_VERSION
        );
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
                attempt_index, distinct_account_index, same_account_retry_index, upstream_account_id, status, phase
            ) VALUES ('attempt-replacement', 'replacement', '2026-10-30 12:00:00',
                      '/v1/responses', 'pool', 0, 0, 0, 41, 'pending', 'sending_request')
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

        sqlx::query(
            "UPDATE pool_upstream_request_attempts SET status = 'success', phase = 'completed', upstream_account_id = 42 WHERE id = ?1",
        )
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
            vec![("coverage".to_string(), 2), ("upsert".to_string(), 6)]
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

    #[tokio::test]
    async fn ownership_claim_uses_epoch_and_serving_fence_cas() {
        let pool = test_pool().await;
        let initial = load_analytics_serving_fence(&pool)
            .await
            .expect("load initial serving fence");
        assert_eq!(initial.owner_kind, "sqlite");
        let claimed = claim_analytics_owner(&pool, "postgres", "writer-a", 1, 1)
            .await
            .expect("claim postgres ownership");
        assert_eq!(claimed.owner_kind, "postgres");
        assert_eq!(claimed.owner_epoch, 2);
        assert_eq!(claimed.serving_fence, 2);
        assert!(
            claim_analytics_owner(&pool, "postgres", "writer-b", 1, 1)
                .await
                .is_err()
        );
        let released = release_analytics_owner(&pool, "writer-a", 2, 2)
            .await
            .expect("release ownership");
        assert!(released.unavailable);
    }

    #[tokio::test]
    async fn postgres_owner_rejects_unconsumed_journal_at_hard_capacity() {
        let pool = test_pool().await;
        sqlx::query(
            "UPDATE analytics_change_state SET journal_event_capacity = 1, journal_byte_capacity = 1024 WHERE id = 1",
        )
        .execute(&pool)
        .await
        .expect("set bounded ownership capacity");
        claim_analytics_owner(&pool, "postgres", "writer-a", 1, 1)
            .await
            .expect("claim postgres ownership");

        for index in 0..2 {
            let mut tx = pool
                .begin()
                .await
                .expect("begin ownership source transaction");
            let result = append_analytics_change_tx(
                tx.as_mut(),
                AnalyticsChangeInput {
                    event_kind: "upsert",
                    entity_kind: "test",
                    source: "test",
                    source_row_id: Some(index),
                    entity_key: &format!("test:{index}"),
                    occurred_at: "2026-10-30 12:00:00",
                    fact_json: "{}",
                },
            )
            .await;
            if index == 0 {
                result.expect("first journal event fits capacity");
                tx.commit().await.expect("commit first journal event");
            } else {
                assert!(
                    result.is_err(),
                    "unconsumed journal must apply backpressure"
                );
                tx.rollback()
                    .await
                    .expect("rollback rejected journal event");
            }
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM analytics_change_events")
            .fetch_one(&pool)
            .await
            .expect("count retained journal events");
        assert_eq!(count, 1);
    }

    #[test]
    fn journal_capacity_adds_safety_margin_and_minimums() {
        let capacity = calculate_analytics_journal_capacity(100, 1_000);
        assert_eq!(capacity.event_capacity, ANALYTICS_CHANGE_CAPACITY_DEFAULT);
        assert_eq!(
            capacity.byte_capacity,
            ANALYTICS_CHANGE_BYTES_CAPACITY_DEFAULT
        );
        let capacity = calculate_analytics_journal_capacity(20_000, 100_000_000);
        assert_eq!(capacity.event_capacity, 25_000);
        assert_eq!(capacity.byte_capacity, 125_000_000);
    }
}
