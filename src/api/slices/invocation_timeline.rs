use super::*;
use anyhow::{Context, anyhow};
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, QueryBuilder, Sqlite, SqliteConnection};
use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

const INVOCATION_TIMELINE_PAGE_SIZE: i64 = 500;
const INVOCATION_TIMELINE_MAX_PAGE_SIZE: i64 = 2_000;
const INVOCATION_TIMELINE_MAX_DURATION_MS: f64 = 30.0 * 24.0 * 60.0 * 60.0 * 1_000.0;
const INVOCATION_TIMELINE_SNAPSHOT_TTL: Duration = Duration::from_secs(30 * 60);
const INVOCATION_TIMELINE_SNAPSHOT_CACHE_LIMIT: usize = 256;
const INVOCATION_TIMELINE_MAX_SNAPSHOT_ROWS: usize = 100_000;
const INVOCATION_TIMELINE_MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
const INVOCATION_TIMELINE_MAX_CACHE_ROWS: usize = 500_000;
const INVOCATION_TIMELINE_MAX_CACHE_BYTES: usize = 256 * 1024 * 1024;
const INVOCATION_TIMELINE_PUBLISH_BATCH_SIZE: usize = 100;
const TIMELINE_SAFE_ACCOUNT_ID_EXCLUSIVE: i64 = 9_007_199_254_740_992;
const INVOCATION_TIMELINE_SNAPSHOT_TABLE: &str = "invocation_timeline_snapshot_rows";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TimelineCursor {
    as_of: String,
    occurred_at: Option<String>,
    invoke_id: Option<String>,
}

#[derive(Debug, Clone)]
struct TimelineSnapshot {
    snapshot_id: i64,
    attempt_snapshot_id: i64,
    revision: u64,
    range_start: String,
    range_end: String,
    upstream_account_id: Option<i64>,
    include_live: bool,
    source_scope: InvocationSourceScope,
    row_count: usize,
    byte_count: usize,
    expires_at: Instant,
}

static INVOCATION_TIMELINE_SNAPSHOTS: once_cell::sync::Lazy<
    StdMutex<HashMap<String, TimelineSnapshot>>,
> = once_cell::sync::Lazy::new(|| StdMutex::new(HashMap::new()));
static INVOCATION_TIMELINE_MATERIALIZATION_LOCK: once_cell::sync::Lazy<tokio::sync::Mutex<()>> =
    once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(()));

#[cfg(test)]
#[derive(Clone)]
struct TimelineMaterializationPause {
    range_start: String,
    range_end: String,
    upstream_account_id: Option<i64>,
    include_live: bool,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    token: Arc<StdMutex<Option<String>>>,
}

#[cfg(test)]
static INVOCATION_TIMELINE_TEST_MATERIALIZATION_PAUSE: once_cell::sync::Lazy<
    StdMutex<Option<TimelineMaterializationPause>>,
> = once_cell::sync::Lazy::new(|| StdMutex::new(None));

#[cfg(test)]
async fn pause_after_timeline_materialization(
    token: &str,
    range_start: &str,
    range_end: &str,
    upstream_account_id: Option<i64>,
    include_live: bool,
) {
    let pause = {
        let mut configured_pause = INVOCATION_TIMELINE_TEST_MATERIALIZATION_PAUSE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if configured_pause.as_ref().is_some_and(|pause| {
            pause.range_start == range_start
                && pause.range_end == range_end
                && pause.upstream_account_id == upstream_account_id
                && pause.include_live == include_live
        }) {
            configured_pause.take()
        } else {
            None
        }
    };
    if let Some(pause) = pause {
        *pause
            .token
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(token.to_string());
        pause.entered.notify_one();
        pause.release.notified().await;
    }
}

#[cfg(not(test))]
async fn pause_after_timeline_materialization(_: &str, _: &str, _: &str, _: Option<i64>, _: bool) {}

struct TimelineSnapshotCleanupGuard {
    token: String,
    pool: Pool<Sqlite>,
    armed: bool,
}

impl TimelineSnapshotCleanupGuard {
    fn new(pool: &Pool<Sqlite>, token: String) -> Self {
        Self {
            token,
            pool: pool.clone(),
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    async fn cleanup_now(&mut self) {
        if !self.armed {
            return;
        }
        self.armed = false;
        remove_timeline_snapshot_reservation(&self.token);
        if let Err(error) = delete_timeline_snapshot_rows(&self.pool, &self.token).await {
            tracing::warn!(?error, snapshot_token = %self.token, "failed to clean up invocation timeline snapshot");
        }
    }
}

impl Drop for TimelineSnapshotCleanupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.armed = false;
        remove_timeline_snapshot_reservation(&self.token);
        let pool = self.pool.clone();
        let token = self.token.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Err(error) = delete_timeline_snapshot_rows(&pool, &token).await {
                    tracing::warn!(?error, snapshot_token = %token, "failed to clean up canceled invocation timeline snapshot");
                }
            });
        }
    }
}

fn encode_timeline_cursor(cursor: &TimelineCursor) -> Result<String, ApiError> {
    let payload = serde_json::to_vec(cursor)
        .map_err(|error| ApiError::from(anyhow!("encode timeline cursor: {error}")))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload))
}

fn decode_timeline_cursor(raw: &str) -> Result<TimelineCursor, ApiError> {
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| ApiError::bad_request(anyhow!("invalid invocation timeline cursor")))?;
    serde_json::from_slice(&payload)
        .map_err(|_| ApiError::bad_request(anyhow!("invalid invocation timeline cursor")))
}

fn append_timeline_db_predicates(
    query: &mut QueryBuilder<'_, Sqlite>,
    persisted_filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
    attempt_snapshot_id: i64,
    upstream_account_id: Option<i64>,
    start_bound: &str,
    overlap_start_bound: &str,
    end_bound: &str,
    include_live: bool,
    cursor: Option<&TimelineCursor>,
) {
    apply_invocation_records_filters(
        query,
        persisted_filters,
        source_scope,
        Some(SnapshotConstraint::UpTo(snapshot_id)),
    );
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND ")
            .push(timeline_upstream_account_id_sql(
                "codex_invocations",
                Some(attempt_snapshot_id),
            ))
            .push(" = ")
            .push_bind(upstream_account_id);
    }
    query
        .push(" AND (occurred_at < ")
        .push_bind(end_bound.to_string())
        .push(" AND occurred_at >= ")
        .push_bind(overlap_start_bound.to_string())
        .push(" AND (occurred_at >= ")
        .push_bind(start_bound.to_string())
        .push(" OR (t_total_ms IS NOT NULL AND t_total_ms >= 0 AND t_total_ms <= ")
        .push_bind(INVOCATION_TIMELINE_MAX_DURATION_MS)
        .push(" AND julianday(occurred_at) + t_total_ms / 86400000.0 >= julianday(")
        .push_bind(start_bound.to_string())
        .push("))");
    if include_live {
        query.push(" OR LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')");
    }
    query.push("))");
    if let Some(cursor) = cursor
        && let (Some(occurred_at), Some(invoke_id)) = (&cursor.occurred_at, &cursor.invoke_id)
    {
        query
            .push(" AND (occurred_at > ")
            .push_bind(occurred_at.clone())
            .push(" OR (occurred_at = ")
            .push_bind(occurred_at.clone())
            .push(" AND invoke_id > ")
            .push_bind(invoke_id.clone())
            .push("))");
    }
}

fn create_timeline_snapshot(
    snapshot_id: i64,
    attempt_snapshot_id: i64,
    range_start: String,
    range_end: String,
    upstream_account_id: Option<i64>,
    include_live: bool,
    _runtime_records: Vec<ApiInvocation>,
) -> Result<String, ApiError> {
    create_timeline_snapshot_for_scope(
        snapshot_id,
        attempt_snapshot_id,
        range_start,
        range_end,
        upstream_account_id,
        include_live,
        InvocationSourceScope::All,
    )
}

fn create_timeline_snapshot_for_scope(
    snapshot_id: i64,
    attempt_snapshot_id: i64,
    range_start: String,
    range_end: String,
    upstream_account_id: Option<i64>,
    include_live: bool,
    source_scope: InvocationSourceScope,
) -> Result<String, ApiError> {
    let token = nanoid::nanoid!(16);
    let now = Instant::now();
    let mut snapshots = INVOCATION_TIMELINE_SNAPSHOTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    snapshots.retain(|_, snapshot| snapshot.expires_at > now);
    if snapshots.len() >= INVOCATION_TIMELINE_SNAPSHOT_CACHE_LIMIT {
        return Err(ApiError::unavailable(anyhow!(
            "invocation timeline snapshot capacity exhausted"
        )));
    }
    snapshots.insert(
        token.clone(),
        TimelineSnapshot {
            snapshot_id,
            attempt_snapshot_id,
            revision: current_dashboard_activity_live_revision(),
            range_start,
            range_end,
            upstream_account_id,
            include_live,
            source_scope,
            row_count: 0,
            byte_count: 0,
            expires_at: now + INVOCATION_TIMELINE_SNAPSHOT_TTL,
        },
    );
    Ok(token)
}

fn load_timeline_snapshot(
    token: &str,
    range_start: &str,
    range_end: &str,
    upstream_account_id: Option<i64>,
    include_live: bool,
) -> Result<TimelineSnapshot, ApiError> {
    let now = Instant::now();
    let mut snapshots = INVOCATION_TIMELINE_SNAPSHOTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    snapshots.retain(|_, snapshot| snapshot.expires_at > now);
    let snapshot = snapshots
        .get(token)
        .cloned()
        .ok_or_else(|| ApiError::bad_request(anyhow!("invocation timeline snapshot expired")))?;
    if snapshot.upstream_account_id != upstream_account_id || snapshot.include_live != include_live
    {
        return Err(ApiError::bad_request(anyhow!(
            "invocation timeline snapshot does not match query"
        )));
    }
    if snapshot.range_start != range_start || snapshot.range_end != range_end {
        return Err(ApiError::bad_request(anyhow!(
            "invocation timeline snapshot does not match window"
        )));
    }
    Ok(snapshot)
}

async fn resolve_timeline_snapshot_watermarks(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
) -> Result<(i64, i64), ApiError> {
    #[derive(Debug, FromRow)]
    struct WatermarkRow {
        invocation_snapshot_id: Option<i64>,
        attempt_snapshot_id: Option<i64>,
    }

    let mut query =
        QueryBuilder::<Sqlite>::new("SELECT (SELECT MAX(id) FROM codex_invocations WHERE 1 = 1");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(
        ") AS invocation_snapshot_id, (SELECT MAX(id) FROM pool_upstream_request_attempts) AS attempt_snapshot_id",
    );
    let row = query
        .build_query_as::<WatermarkRow>()
        .fetch_one(pool)
        .await?;
    Ok((
        row.invocation_snapshot_id.unwrap_or(0),
        row.attempt_snapshot_id.unwrap_or(0),
    ))
}

#[derive(Debug, FromRow)]
struct TimelineAccountFallbackRow {
    invoke_id: String,
    occurred_at: String,
    upstream_account_id: Option<i64>,
    payload_upstream_account_id_is_valid: i64,
    upstream_account_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationTimelineQuery {
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) include_live: Option<bool>,
    pub(crate) limit: Option<i64>,
    pub(crate) cursor: Option<String>,
    pub(crate) as_of: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationTimelineRecord {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) end_at: Option<String>,
    pub(crate) is_in_flight: bool,
    pub(crate) status: Option<String>,
    pub(crate) live_phase: Option<String>,
    pub(crate) first_token_ms: Option<f64>,
    pub(crate) t_total_ms: Option<f64>,
    pub(crate) pool_attempt_count: Option<i64>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) failure_class: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationTimelineResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) as_of: String,
    pub(crate) total: i64,
    pub(crate) has_more: bool,
    pub(crate) next_cursor: Option<String>,
    pub(crate) records: Vec<InvocationTimelineRecord>,
}

#[derive(Debug, FromRow)]
struct TimelineSnapshotDbRow {
    invoke_id: String,
    occurred_at: String,
    payload: String,
}

fn timeline_snapshot_payload_size(record: &InvocationTimelineRecord) -> Result<usize, ApiError> {
    serde_json::to_vec(record)
        .map(|payload| payload.len())
        .map_err(|error| ApiError::from(anyhow!("encode timeline snapshot row: {error}")))
}

fn timeline_snapshot_cache_usage(excluding_token: &str) -> (usize, usize) {
    let now = Instant::now();
    let snapshots = INVOCATION_TIMELINE_SNAPSHOTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    snapshots
        .iter()
        .filter(|(token, snapshot)| token.as_str() != excluding_token && snapshot.expires_at > now)
        .fold((0, 0), |(rows, bytes), (_, snapshot)| {
            (
                rows.saturating_add(snapshot.row_count),
                bytes.saturating_add(snapshot.byte_count),
            )
        })
}

fn set_timeline_snapshot_usage(
    token: &str,
    row_count: usize,
    byte_count: usize,
) -> Result<(), ApiError> {
    let mut snapshots = INVOCATION_TIMELINE_SNAPSHOTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let snapshot = snapshots
        .get_mut(token)
        .ok_or_else(|| ApiError::unavailable(anyhow!("invocation timeline snapshot expired")))?;
    snapshot.row_count = row_count;
    snapshot.byte_count = byte_count;
    Ok(())
}

fn reserve_timeline_snapshot_budget(
    row_count: &mut usize,
    byte_count: &mut usize,
    cache_row_base: usize,
    cache_byte_base: usize,
    record: &InvocationTimelineRecord,
    replaced_payload_size: Option<usize>,
) -> Result<(), ApiError> {
    let payload_size = timeline_snapshot_payload_size(record)?;
    let next_row_count = match replaced_payload_size {
        Some(_) => *row_count,
        None => row_count.checked_add(1).ok_or_else(|| {
            ApiError::unavailable(anyhow!("invocation timeline snapshot is too large"))
        })?,
    };
    let current_bytes = if let Some(size) = replaced_payload_size {
        byte_count.checked_sub(size).ok_or_else(|| {
            ApiError::unavailable(anyhow!("invocation timeline snapshot is too large"))
        })?
    } else {
        *byte_count
    };
    let next_byte_count = current_bytes.checked_add(payload_size).ok_or_else(|| {
        ApiError::unavailable(anyhow!("invocation timeline snapshot is too large"))
    })?;
    if next_row_count > INVOCATION_TIMELINE_MAX_SNAPSHOT_ROWS
        || next_byte_count > INVOCATION_TIMELINE_MAX_SNAPSHOT_BYTES
        || cache_row_base.saturating_add(next_row_count) > INVOCATION_TIMELINE_MAX_CACHE_ROWS
        || cache_byte_base.saturating_add(next_byte_count) > INVOCATION_TIMELINE_MAX_CACHE_BYTES
    {
        return Err(ApiError::unavailable(anyhow!(
            "invocation timeline snapshot capacity exhausted"
        )));
    }
    *row_count = next_row_count;
    *byte_count = next_byte_count;
    Ok(())
}

async fn ensure_timeline_snapshot_table(pool: &Pool<Sqlite>) -> Result<(), ApiError> {
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS {INVOCATION_TIMELINE_SNAPSHOT_TABLE} (\
            snapshot_token TEXT NOT NULL,\
            invoke_id TEXT NOT NULL,\
            occurred_at TEXT NOT NULL,\
            record_id INTEGER NOT NULL,\
            is_runtime INTEGER NOT NULL,\
            is_in_flight INTEGER NOT NULL,\
            payload TEXT NOT NULL,\
            PRIMARY KEY (snapshot_token, invoke_id, occurred_at)\
        )"
    ))
    .execute(pool)
    .await?;
    sqlx::query(&format!(
        "CREATE INDEX IF NOT EXISTS {INVOCATION_TIMELINE_SNAPSHOT_TABLE}_order \
         ON {INVOCATION_TIMELINE_SNAPSHOT_TABLE} (snapshot_token, occurred_at, invoke_id)"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

fn remove_timeline_snapshot_reservation(token: &str) {
    INVOCATION_TIMELINE_SNAPSHOTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(token);
}

async fn delete_timeline_snapshot_rows(
    pool: &Pool<Sqlite>,
    token: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(&format!(
        "DELETE FROM {INVOCATION_TIMELINE_SNAPSHOT_TABLE} WHERE snapshot_token = ?1"
    ))
    .bind(token)
    .execute(pool)
    .await?;
    Ok(())
}

async fn prune_timeline_snapshot_rows(pool: &Pool<Sqlite>) -> Result<(), ApiError> {
    ensure_timeline_snapshot_table(pool).await?;
    let active_tokens = {
        let now = Instant::now();
        let mut snapshots = INVOCATION_TIMELINE_SNAPSHOTS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        snapshots.retain(|_, snapshot| snapshot.expires_at > now);
        snapshots.keys().cloned().collect::<Vec<_>>()
    };
    if active_tokens.is_empty() {
        sqlx::query(&format!("DELETE FROM {INVOCATION_TIMELINE_SNAPSHOT_TABLE}"))
            .execute(pool)
            .await?;
        return Ok(());
    }
    let mut query = QueryBuilder::<Sqlite>::new(&format!(
        "DELETE FROM {INVOCATION_TIMELINE_SNAPSHOT_TABLE} WHERE snapshot_token NOT IN ("
    ));
    for (index, token) in active_tokens.iter().enumerate() {
        if index > 0 {
            query.push(", ");
        }
        query.push_bind(token);
    }
    query.push(")");
    query.build().execute(pool).await?;
    Ok(())
}

#[derive(Debug)]
struct TimelineSnapshotInsertRow {
    invoke_id: String,
    occurred_at: String,
    record_id: i64,
    is_runtime: i64,
    is_in_flight: i64,
    payload: String,
}

fn encode_timeline_snapshot_insert_row(
    record: &InvocationTimelineRecord,
    is_runtime: bool,
) -> Result<TimelineSnapshotInsertRow, ApiError> {
    let payload = serde_json::to_string(record)
        .map_err(|error| ApiError::from(anyhow!("encode timeline snapshot row: {error}")))?;
    Ok(TimelineSnapshotInsertRow {
        invoke_id: record.invoke_id.clone(),
        occurred_at: record.occurred_at.clone(),
        record_id: record.id,
        is_runtime: i64::from(is_runtime),
        is_in_flight: i64::from(record.is_in_flight),
        payload,
    })
}

async fn insert_timeline_snapshot_records_on_connection(
    connection: &mut SqliteConnection,
    snapshot_token: &str,
    rows: &[TimelineSnapshotInsertRow],
) -> Result<(), ApiError> {
    for batch in rows.chunks(INVOCATION_TIMELINE_PUBLISH_BATCH_SIZE) {
        let mut query = QueryBuilder::<Sqlite>::new(&format!(
            "INSERT INTO {INVOCATION_TIMELINE_SNAPSHOT_TABLE} (snapshot_token, invoke_id, occurred_at, record_id, is_runtime, is_in_flight, payload) "
        ));
        query.push_values(batch, |mut values, row| {
            values
                .push_bind(snapshot_token)
                .push_bind(&row.invoke_id)
                .push_bind(&row.occurred_at)
                .push_bind(row.record_id)
                .push_bind(row.is_runtime)
                .push_bind(row.is_in_flight)
                .push_bind(&row.payload);
        });
        query.push(format!(
            " ON CONFLICT(snapshot_token, invoke_id, occurred_at) DO UPDATE SET record_id = excluded.record_id, is_runtime = excluded.is_runtime, is_in_flight = excluded.is_in_flight, payload = excluded.payload WHERE {INVOCATION_TIMELINE_SNAPSHOT_TABLE}.is_runtime = 0 OR excluded.record_id > {INVOCATION_TIMELINE_SNAPSHOT_TABLE}.record_id"
        ));
        query.build().execute(&mut *connection).await?;
    }
    Ok(())
}

fn timeline_record_from_api(record: &ApiInvocation, is_runtime: bool) -> InvocationTimelineRecord {
    let is_in_flight = runtime_record_is_in_flight(record);
    let total_ms = valid_timeline_duration_ms(record.t_total_ms);
    let end_at = if is_in_flight {
        None
    } else {
        total_ms.and_then(|duration| {
            parse_to_utc_datetime(&record.occurred_at)
                .and_then(|start| timeline_end_at(start, Some(duration)))
        })
    };
    let occurred_at = record.occurred_at.clone();
    InvocationTimelineRecord {
        id: record.id,
        invoke_id: record.invoke_id.clone(),
        occurred_at: parse_to_utc_datetime(&occurred_at)
            .map(format_utc_iso_precise)
            .unwrap_or(occurred_at),
        end_at,
        is_in_flight,
        status: invocation_display_status_value(record).map(str::to_string),
        live_phase: if is_runtime {
            runtime_record_live_phase(record).map(str::to_string)
        } else {
            record.live_phase.clone()
        },
        first_token_ms: if is_runtime {
            runtime_record_first_token_ms(record)
        } else {
            record
                .first_token_ms
                .filter(|value| value.is_finite() && *value >= 0.0)
        },
        t_total_ms: total_ms,
        pool_attempt_count: record.pool_attempt_count,
        upstream_account_id: record.upstream_account_id,
        upstream_account_name: record.upstream_account_name.clone(),
        failure_class: record.failure_class.clone(),
    }
}

async fn materialize_timeline_snapshot(
    state: &AppState,
    snapshot_token: &str,
    source_scope: InvocationSourceScope,
    filters: &InvocationRecordsFilters,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    start_bound: &str,
    overlap_start_bound: &str,
    end_bound: &str,
    snapshot_id: i64,
    attempt_snapshot_id: i64,
    upstream_account_id: Option<i64>,
    include_live: bool,
) -> Result<(), ApiError> {
    ensure_timeline_snapshot_table(&state.pool).await?;
    sqlx::query(&format!(
        "DELETE FROM {INVOCATION_TIMELINE_SNAPSHOT_TABLE} WHERE snapshot_token = ?1"
    ))
    .bind(snapshot_token)
    .execute(&state.pool)
    .await?;
    let mut persisted_filters = filters.clone();
    persisted_filters.upstream_account_id = None;
    let (cache_row_base, cache_byte_base) = timeline_snapshot_cache_usage(snapshot_token);
    let mut source_transaction = state.pool.begin().await?;
    let mut materialized_records = Vec::new();
    let mut materialized_sources = Vec::new();
    let mut materialized_row_count = 0;
    let mut materialized_byte_count = 0;
    let mut cursor = None;
    loop {
        let mut query = build_invocation_select_query();
        append_timeline_db_predicates(
            &mut query,
            &persisted_filters,
            source_scope,
            snapshot_id,
            attempt_snapshot_id,
            upstream_account_id,
            start_bound,
            overlap_start_bound,
            end_bound,
            include_live,
            cursor.as_ref(),
        );
        query
            .push(" AND NOT EXISTS (SELECT 1 FROM codex_invocations AS duplicate WHERE duplicate.invoke_id = codex_invocations.invoke_id AND duplicate.occurred_at = codex_invocations.occurred_at AND duplicate.id > codex_invocations.id AND duplicate.id <= ")
            .push_bind(snapshot_id)
            .push(") ORDER BY occurred_at ASC, invoke_id ASC, id ASC LIMIT ")
            .push_bind(INVOCATION_TIMELINE_PAGE_SIZE + 1);
        let mut page = query
            .build_query_as::<ApiInvocation>()
            .fetch_all(&mut *source_transaction)
            .await?;
        let has_more = page.len() as i64 > INVOCATION_TIMELINE_PAGE_SIZE;
        page.truncate(INVOCATION_TIMELINE_PAGE_SIZE as usize);
        let Some(last) = page.last() else {
            break;
        };
        let next_cursor = TimelineCursor {
            as_of: snapshot_token.to_string(),
            occurred_at: Some(last.occurred_at.clone()),
            invoke_id: Some(last.invoke_id.clone()),
        };
        for record in &mut page {
            hydrate_api_invocation_blocked_binding(record);
        }
        hydrate_timeline_accounts_on_connection(
            &mut source_transaction,
            &mut page,
            source_scope,
            snapshot_id,
            attempt_snapshot_id,
            false,
        )
        .await?;
        for record in page
            .iter()
            .filter(|record| runtime_record_matches_filters(record, filters, source_scope))
        {
            let timeline_record = timeline_record_from_api(record, false);
            reserve_timeline_snapshot_budget(
                &mut materialized_row_count,
                &mut materialized_byte_count,
                cache_row_base,
                cache_byte_base,
                &timeline_record,
                None,
            )?;
            materialized_records.push(timeline_record);
            materialized_sources.push(false);
        }
        if !has_more {
            break;
        }
        cursor = Some(next_cursor);
    }

    let mut runtime_records = if include_live {
        runtime_overlay_snapshot(state)
    } else {
        Vec::new()
    };
    let mut runtime_filters = filters.clone();
    runtime_filters.upstream_account_id = None;
    runtime_records.retain(|record| {
        if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
            return false;
        }
        let Some(occurred_at) = parse_to_utc_datetime(&record.occurred_at) else {
            return false;
        };
        occurred_at < range_end
            && (runtime_record_is_in_flight(record)
                || timeline_record_overlaps(occurred_at, record.t_total_ms, range_start, range_end))
            && runtime_record_matches_filters(record, &runtime_filters, source_scope)
    });
    hydrate_timeline_accounts_on_connection(
        &mut source_transaction,
        &mut runtime_records,
        source_scope,
        snapshot_id,
        attempt_snapshot_id,
        true,
    )
    .await?;
    runtime_records.retain(|record| runtime_record_matches_filters(record, filters, source_scope));
    let mut record_indexes = materialized_records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            (
                (record.invoke_id.clone(), record.occurred_at.clone()),
                index,
            )
        })
        .collect::<HashMap<_, _>>();
    for record in runtime_records {
        let timeline_record = timeline_record_from_api(&record, true);
        let key = (
            timeline_record.invoke_id.clone(),
            timeline_record.occurred_at.clone(),
        );
        if let Some(index) = record_indexes.get(&key).copied() {
            let current = &materialized_records[index];
            if !materialized_sources[index] && !current.is_in_flight {
                continue;
            }
            if materialized_sources[index] && timeline_record.id <= current.id {
                continue;
            }
            let previous_payload_size = timeline_snapshot_payload_size(current)?;
            reserve_timeline_snapshot_budget(
                &mut materialized_row_count,
                &mut materialized_byte_count,
                cache_row_base,
                cache_byte_base,
                &timeline_record,
                Some(previous_payload_size),
            )?;
            materialized_records[index] = timeline_record;
            materialized_sources[index] = true;
        } else {
            reserve_timeline_snapshot_budget(
                &mut materialized_row_count,
                &mut materialized_byte_count,
                cache_row_base,
                cache_byte_base,
                &timeline_record,
                None,
            )?;
            let index = materialized_records.len();
            record_indexes.insert(key, index);
            materialized_records.push(timeline_record);
            materialized_sources.push(true);
        }
    }
    source_transaction.commit().await?;

    let mut published_records = materialized_records
        .into_iter()
        .zip(materialized_sources)
        .collect::<Vec<_>>();
    published_records.sort_by(|(left, _), (right, _)| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.invoke_id.cmp(&right.invoke_id))
            .then_with(|| left.id.cmp(&right.id))
    });
    let published_rows = published_records
        .iter()
        .map(|(record, is_runtime)| encode_timeline_snapshot_insert_row(record, *is_runtime))
        .collect::<Result<Vec<_>, _>>()?;
    let mut publish_transaction = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    insert_timeline_snapshot_records_on_connection(
        &mut publish_transaction,
        snapshot_token,
        &published_rows,
    )
    .await?;
    publish_transaction.commit().await?;
    set_timeline_snapshot_usage(
        snapshot_token,
        materialized_row_count,
        materialized_byte_count,
    )?;
    Ok(())
}

pub(crate) async fn fetch_timeline(
    State(state): State<Arc<AppState>>,
    Query(params): Query<InvocationTimelineQuery>,
) -> Result<Json<InvocationTimelineResponse>, ApiError> {
    let parse_bound = |value: &str, field: &str| {
        DateTime::parse_from_rfc3339(value.trim())
            .with_context(|| format!("invalid {field}: {value}"))
            .map(|value| value.with_timezone(&Utc))
            .map_err(ApiError::bad_request)
    };
    let range_start = parse_bound(&params.from, "from")?;
    let range_end = parse_bound(&params.to, "to")?;
    if range_start >= range_end {
        return Err(ApiError::bad_request(anyhow!("from must be before to")));
    }
    if params
        .upstream_account_id
        .is_some_and(|account_id| account_id <= 0)
    {
        return Err(ApiError::bad_request(anyhow!("invalid upstream account")));
    }

    let include_live = params.include_live.unwrap_or(true);
    let canonical_range_start = format_utc_iso(range_start);
    let canonical_range_end = format_utc_iso(range_end);
    let page_size = params
        .limit
        .unwrap_or(INVOCATION_TIMELINE_PAGE_SIZE)
        .clamp(1, INVOCATION_TIMELINE_MAX_PAGE_SIZE);
    let cursor = params
        .cursor
        .as_deref()
        .map(decode_timeline_cursor)
        .transpose()?;
    if cursor
        .as_ref()
        .is_some_and(|value| value.occurred_at.is_some() != value.invoke_id.is_some())
    {
        return Err(ApiError::bad_request(anyhow!(
            "invalid invocation timeline cursor position"
        )));
    }
    let filters = build_invocation_filters(&ListQuery {
        upstream_account_id: params.upstream_account_id,
        ..ListQuery::default()
    })?;
    let start_bound = crate::db_occurred_at_lower_bound(range_start);
    let overlap_start =
        range_start - chrono::Duration::milliseconds(INVOCATION_TIMELINE_MAX_DURATION_MS as i64);
    let overlap_start_bound = crate::db_occurred_at_lower_bound(overlap_start);
    let end_bound = crate::db_occurred_at_upper_bound(range_end);
    let mut cleanup_guard: Option<TimelineSnapshotCleanupGuard> = None;
    let (as_of, _snapshot) = if let Some(as_of) = params.as_of.as_deref() {
        if let Some(cursor) = cursor.as_ref()
            && cursor.as_of != as_of
        {
            return Err(ApiError::bad_request(anyhow!(
                "invocation timeline cursor does not match asOf"
            )));
        }
        (
            as_of.to_string(),
            load_timeline_snapshot(
                as_of,
                &canonical_range_start,
                &canonical_range_end,
                params.upstream_account_id,
                include_live,
            )?,
        )
    } else {
        if cursor.is_some() {
            return Err(ApiError::bad_request(anyhow!(
                "asOf is required with an invocation timeline cursor"
            )));
        }
        let _materialization_guard = INVOCATION_TIMELINE_MATERIALIZATION_LOCK.lock().await;
        let source_scope = resolve_default_source_scope(&state.pool).await?;
        let (snapshot_id, attempt_snapshot_id) =
            resolve_timeline_snapshot_watermarks(&state.pool, source_scope).await?;
        prune_timeline_snapshot_rows(&state.pool).await?;
        let as_of = create_timeline_snapshot_for_scope(
            snapshot_id,
            attempt_snapshot_id,
            canonical_range_start.clone(),
            canonical_range_end.clone(),
            params.upstream_account_id,
            include_live,
            source_scope,
        )?;
        let mut snapshot_cleanup_guard =
            TimelineSnapshotCleanupGuard::new(&state.pool, as_of.clone());
        let materialize_result = materialize_timeline_snapshot(
            state.as_ref(),
            &as_of,
            source_scope,
            &filters,
            range_start,
            range_end,
            &start_bound,
            &overlap_start_bound,
            &end_bound,
            snapshot_id,
            attempt_snapshot_id,
            params.upstream_account_id,
            include_live,
        )
        .await;
        if let Err(error) = materialize_result {
            snapshot_cleanup_guard.cleanup_now().await;
            return Err(error);
        }
        let snapshot = load_timeline_snapshot(
            &as_of,
            &canonical_range_start,
            &canonical_range_end,
            params.upstream_account_id,
            include_live,
        )?;
        cleanup_guard = Some(snapshot_cleanup_guard);
        pause_after_timeline_materialization(
            &as_of,
            &canonical_range_start,
            &canonical_range_end,
            params.upstream_account_id,
            include_live,
        )
        .await;
        (as_of.clone(), snapshot)
    };
    ensure_timeline_snapshot_table(&state.pool).await?;
    let mut page_query = QueryBuilder::<Sqlite>::new(&format!(
        "SELECT invoke_id, occurred_at, payload FROM {INVOCATION_TIMELINE_SNAPSHOT_TABLE} WHERE snapshot_token = "
    ));
    page_query.push_bind(as_of.clone());
    if let Some(cursor) = cursor.as_ref()
        && let (Some(occurred_at), Some(invoke_id)) = (&cursor.occurred_at, &cursor.invoke_id)
    {
        page_query
            .push(" AND (occurred_at > ")
            .push_bind(occurred_at.clone())
            .push(" OR (occurred_at = ")
            .push_bind(occurred_at.clone())
            .push(" AND invoke_id > ")
            .push_bind(invoke_id.clone())
            .push("))");
    }
    page_query
        .push(" ORDER BY occurred_at ASC, invoke_id ASC LIMIT ")
        .push_bind(page_size + 1);
    let mut rows = page_query
        .build_query_as::<TimelineSnapshotDbRow>()
        .fetch_all(&state.pool)
        .await?;
    let has_more = rows.len() as i64 > page_size;
    rows.truncate(page_size as usize);
    let total = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT COUNT(*) FROM {INVOCATION_TIMELINE_SNAPSHOT_TABLE} WHERE snapshot_token = ?1"
    ))
    .bind(as_of.clone())
    .fetch_one(&state.pool)
    .await?;
    let next_cursor = if has_more {
        rows.last()
            .map(|row| {
                encode_timeline_cursor(&TimelineCursor {
                    as_of: as_of.clone(),
                    occurred_at: Some(row.occurred_at.clone()),
                    invoke_id: Some(row.invoke_id.clone()),
                })
            })
            .transpose()?
    } else {
        None
    };
    let timeline_records = rows
        .into_iter()
        .map(|row| {
            serde_json::from_str::<InvocationTimelineRecord>(&row.payload).map_err(|error| {
                ApiError::from(anyhow!(
                    "decode invocation timeline snapshot row {}: {error}",
                    row.invoke_id
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    if let Some(cleanup_guard) = cleanup_guard.as_mut() {
        cleanup_guard.disarm();
    }
    Ok(Json(InvocationTimelineResponse {
        range_start: format_utc_iso(range_start),
        range_end: format_utc_iso(range_end),
        as_of: as_of.clone(),
        total,
        has_more,
        next_cursor,
        records: timeline_records,
    }))
}

fn timeline_record_overlaps(
    occurred_at: DateTime<Utc>,
    total_ms: Option<f64>,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> bool {
    if occurred_at >= range_end {
        return false;
    }
    match valid_timeline_duration_ms(total_ms) {
        Some(duration) => timeline_end_at(occurred_at, Some(duration))
            .and_then(|value| parse_to_utc_datetime(&value))
            .is_some_and(|end| end >= range_start),
        None => occurred_at >= range_start,
    }
}

fn valid_timeline_duration_ms(value: Option<f64>) -> Option<f64> {
    value.filter(|value| {
        value.is_finite() && *value >= 0.0 && *value <= INVOCATION_TIMELINE_MAX_DURATION_MS
    })
}

fn timeline_end_at(start: DateTime<Utc>, duration: Option<f64>) -> Option<String> {
    let milliseconds = valid_timeline_duration_ms(duration)?.round();
    if milliseconds > i64::MAX as f64 {
        return None;
    }
    start
        .checked_add_signed(chrono::Duration::milliseconds(milliseconds as i64))
        .map(format_utc_iso_precise)
}

async fn hydrate_timeline_accounts(
    pool: &Pool<Sqlite>,
    records: &mut [ApiInvocation],
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
    attempt_snapshot_id: i64,
) -> Result<(), ApiError> {
    let mut connection = pool.acquire().await?;
    hydrate_timeline_accounts_on_connection(
        &mut connection,
        records,
        source_scope,
        snapshot_id,
        attempt_snapshot_id,
        false,
    )
    .await
}

async fn hydrate_timeline_accounts_on_connection(
    connection: &mut SqliteConnection,
    records: &mut [ApiInvocation],
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
    attempt_snapshot_id: i64,
    preserve_valid_runtime_account: bool,
) -> Result<(), ApiError> {
    let mut invalid_account_keys = HashSet::new();
    for record in records.iter_mut() {
        if record.upstream_account_id.is_none()
            || record.upstream_account_id.is_some_and(|account_id| {
                account_id <= 0 || account_id >= TIMELINE_SAFE_ACCOUNT_ID_EXCLUSIVE
            })
        {
            invalid_account_keys.insert((record.invoke_id.clone(), record.occurred_at.clone()));
            record.upstream_account_id = None;
            record.upstream_account_name = None;
        }
    }
    let keys = records
        .iter()
        .map(|record| (record.invoke_id.clone(), record.occurred_at.clone()))
        .collect::<HashSet<_>>();
    if keys.is_empty() {
        return Ok(());
    }
    let resolved_id =
        timeline_upstream_account_id_sql("codex_invocations", Some(attempt_snapshot_id));
    let mut rows = Vec::new();
    let key_list = keys.into_iter().collect::<Vec<_>>();
    for chunk in key_list.chunks(100) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT codex_invocations.invoke_id, codex_invocations.occurred_at, ",
        );
        query
            .push(resolved_id.as_str())
            .push(" AS upstream_account_id, ")
            .push(timeline_payload_account_id_is_valid_sql(
                "codex_invocations",
            ))
            .push(" AS payload_upstream_account_id_is_valid, ")
            .push(INVOCATION_UPSTREAM_ACCOUNT_NAME_SQL)
            .push(" AS upstream_account_name FROM codex_invocations WHERE (");
        for (index, (invoke_id, occurred_at)) in chunk.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(codex_invocations.invoke_id = ")
                .push_bind(invoke_id.as_str())
                .push(" AND codex_invocations.occurred_at = ")
                .push_bind(occurred_at.as_str())
                .push(")");
        }
        query.push(")");
        if source_scope == InvocationSourceScope::ProxyOnly {
            query
                .push(" AND codex_invocations.source = ")
                .push_bind(SOURCE_PROXY);
        }
        query
            .push(" AND codex_invocations.id <= ")
            .push_bind(snapshot_id)
            .push(" ORDER BY codex_invocations.id ASC");
        rows.extend(
            query
                .build_query_as::<TimelineAccountFallbackRow>()
                .fetch_all(&mut *connection)
                .await?,
        );
    }
    let fallbacks = rows
        .into_iter()
        .map(|row| ((row.invoke_id.clone(), row.occurred_at.clone()), row))
        .collect::<std::collections::HashMap<_, _>>();
    for record in records {
        if let Some(row) = fallbacks.get(&(record.invoke_id.clone(), record.occurred_at.clone())) {
            if let Some(upstream_account_id) = row.upstream_account_id {
                record.upstream_account_id = Some(upstream_account_id);
                record.upstream_account_name = (row.payload_upstream_account_id_is_valid == 1)
                    .then(|| row.upstream_account_name.clone())
                    .flatten();
            } else if !preserve_valid_runtime_account
                && row.payload_upstream_account_id_is_valid != 1
            {
                record.upstream_account_id = None;
                record.upstream_account_name = None;
            }
        }
        if invalid_account_keys.contains(&(record.invoke_id.clone(), record.occurred_at.clone()))
            && record.upstream_account_id.is_none()
        {
            record.upstream_account_id = sqlx::query_scalar(
                "SELECT upstream_account_id FROM pool_upstream_request_attempts WHERE invoke_id = ?1 AND occurred_at = ?2 AND id > 0 AND id <= ?3 AND typeof(upstream_account_id) = 'integer' AND upstream_account_id IS NOT NULL AND upstream_account_id > 0 AND upstream_account_id < 9007199254740992 ORDER BY attempt_index DESC, id DESC LIMIT 1",
            )
            .bind(&record.invoke_id)
            .bind(&record.occurred_at)
            .bind(attempt_snapshot_id)
            .fetch_optional(&mut *connection)
            .await?;
        }
    }
    Ok(())
}

fn timeline_upstream_account_id_sql(
    invocation_ref: &str,
    attempt_snapshot_id: Option<i64>,
) -> String {
    let payload_is_valid = format!("json_valid({invocation_ref}.payload)");
    let attempt_snapshot_predicate = attempt_snapshot_id
        .map(|snapshot_id| format!(" AND attempt.id <= {snapshot_id}"))
        .unwrap_or_default();
    let payload_id = format!(
        "CASE WHEN {payload_is_valid} THEN {} END",
        timeline_payload_account_id_sql(invocation_ref)
    );
    format!(
        "COALESCE({payload_id}, (SELECT attempt.upstream_account_id FROM pool_upstream_request_attempts attempt WHERE attempt.invoke_id = {invocation_ref}.invoke_id AND attempt.occurred_at = {invocation_ref}.occurred_at AND attempt.id > 0{attempt_snapshot_predicate} AND typeof(attempt.upstream_account_id) = 'integer' AND attempt.upstream_account_id IS NOT NULL AND attempt.upstream_account_id > 0 AND attempt.upstream_account_id < 9007199254740992 ORDER BY attempt.attempt_index DESC, attempt.id DESC LIMIT 1))",
    )
}

fn timeline_payload_account_id_parts(invocation_ref: &str) -> (String, String) {
    let payload_is_valid = format!("json_valid({invocation_ref}.payload)");
    let value = format!(
        "CASE WHEN {payload_is_valid} THEN json_extract({invocation_ref}.payload, '$.upstreamAccountId') END"
    );
    let value_type = format!(
        "CASE WHEN {payload_is_valid} THEN json_type({invocation_ref}.payload, '$.upstreamAccountId') END"
    );
    let trimmed = format!("TRIM({value})");
    let normalized = format!("ltrim({trimmed}, '0')");
    let valid_payload_id = format!(
        "({value} IS NOT NULL AND (({value_type} = 'integer' AND typeof({value}) = 'integer' AND {value} > 0 AND {value} < 9007199254740992) OR ({value_type} = 'real' AND {value} > 0 AND {value} < 9007199254740992.0 AND CAST({value} AS REAL) = CAST({value} AS INTEGER)) OR ({value_type} = 'text' AND {trimmed} <> '' AND {trimmed} NOT GLOB '*[^0-9]*' AND {normalized} <> '' AND (length({normalized}) < 16 OR (length({normalized}) = 16 AND {normalized} <= '9007199254740991')))))"
    );
    (value, valid_payload_id)
}

fn timeline_payload_account_id_sql(invocation_ref: &str) -> String {
    let (value, valid_payload_id) = timeline_payload_account_id_parts(invocation_ref);
    format!("CASE WHEN {valid_payload_id} THEN CAST({value} AS INTEGER) END")
}

fn timeline_payload_account_id_is_valid_sql(invocation_ref: &str) -> String {
    let (_, valid_payload_id) = timeline_payload_account_id_parts(invocation_ref);
    format!("CASE WHEN {valid_payload_id} THEN 1 ELSE 0 END")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use sqlx::Connection;
    use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0)
            .single()
            .expect("valid test timestamp")
    }

    #[test]
    fn cross_midnight_window_includes_only_overlapping_invocations() {
        let range_start = at(86_400);
        let range_end = at(90_000);
        assert!(timeline_record_overlaps(
            at(86_399),
            Some(2_000.0),
            range_start,
            range_end
        ));
        assert!(!timeline_record_overlaps(
            at(86_399),
            Some(500.0),
            range_start,
            range_end
        ));
        assert!(timeline_record_overlaps(
            at(86_401),
            None,
            range_start,
            range_end
        ));
        assert!(!timeline_record_overlaps(
            at(90_000),
            Some(1_000.0),
            range_start,
            range_end
        ));
    }

    #[test]
    fn invalid_total_duration_is_treated_as_unknown_and_stays_in_window() {
        let range_start = at(100);
        let range_end = at(200);
        assert!(timeline_record_overlaps(
            at(150),
            Some(f64::NAN),
            range_start,
            range_end
        ));
        assert!(timeline_record_overlaps(
            at(150),
            Some(-1.0),
            range_start,
            range_end
        ));
        assert!(timeline_record_overlaps(
            at(150),
            Some(INVOCATION_TIMELINE_MAX_DURATION_MS + 1.0),
            range_start,
            range_end
        ));
        assert!(!timeline_record_overlaps(
            at(99),
            Some(f64::NAN),
            range_start,
            range_end
        ));
    }

    #[test]
    fn snapshot_budget_rejects_cache_wide_capacity() {
        let record =
            crate::api::slices::invocations_and_summary::summary_projection_test_invocation();
        let timeline_record = timeline_record_from_api(&record, false);
        let mut row_count = 0;
        let mut byte_count = 0;
        let result = reserve_timeline_snapshot_budget(
            &mut row_count,
            &mut byte_count,
            INVOCATION_TIMELINE_MAX_CACHE_ROWS,
            0,
            &timeline_record,
            None,
        );
        assert!(matches!(result, Err(ApiError::Unavailable(_))));
    }

    #[test]
    fn snapshot_cursor_survives_a_burst_of_new_first_page_requests() {
        let first_token = create_timeline_snapshot(
            1,
            1,
            "snapshot-retention-start".to_string(),
            "snapshot-retention-end".to_string(),
            None,
            false,
            Vec::new(),
        )
        .expect("create oldest snapshot");
        for index in 0..32 {
            create_timeline_snapshot(
                index + 2,
                index + 2,
                format!("snapshot-retention-start-{index}"),
                format!("snapshot-retention-end-{index}"),
                None,
                false,
                Vec::new(),
            )
            .expect("create newer snapshot");
        }

        let snapshot = load_timeline_snapshot(
            &first_token,
            "snapshot-retention-start",
            "snapshot-retention-end",
            None,
            false,
        )
        .expect("oldest unexpired snapshot remains available");
        assert_eq!(snapshot.snapshot_id, 1);
    }

    #[tokio::test]
    async fn dropping_snapshot_cleanup_guard_releases_an_in_progress_reservation() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        ensure_timeline_snapshot_table(&state.pool)
            .await
            .expect("create snapshot table");
        let token = create_timeline_snapshot(
            1,
            1,
            "cancellation-start".to_string(),
            "cancellation-end".to_string(),
            None,
            true,
            Vec::new(),
        )
        .expect("reserve snapshot");
        sqlx::query(&format!(
            "INSERT INTO {INVOCATION_TIMELINE_SNAPSHOT_TABLE} (snapshot_token, invoke_id, occurred_at, record_id, is_runtime, is_in_flight, payload) VALUES (?1, 'canceled', '2026-01-01T00:00:00.000Z', 1, 0, 0, '{{}}')"
        ))
        .bind(&token)
        .execute(&state.pool)
        .await
        .expect("insert canceled snapshot row");

        let guard_pool = state.pool.clone();
        let guard_token = token.clone();
        let task = tokio::spawn(async move {
            let _guard = TimelineSnapshotCleanupGuard::new(&guard_pool, guard_token);
            std::future::pending::<()>().await;
        });
        tokio::task::yield_now().await;
        task.abort();
        task.await
            .expect_err("pending snapshot task should be aborted");

        assert!(
            load_timeline_snapshot(&token, "cancellation-start", "cancellation-end", None, true,)
                .is_err()
        );
        for _ in 0..20 {
            let row_count: i64 = sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM {INVOCATION_TIMELINE_SNAPSHOT_TABLE} WHERE snapshot_token = ?1"
            ))
            .bind(&token)
            .fetch_one(&state.pool)
            .await
            .expect("count canceled snapshot rows");
            if row_count == 0 {
                state.pool.close().await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("canceled snapshot rows were not cleaned up");
    }

    #[tokio::test]
    async fn aborting_fetch_after_materialization_cleans_up_snapshot() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let range_start = at(22_000);
        let range_end = at(22_100);
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES ('aborted-fetch', ?1, 'proxy', 'success', 100, '{\"upstreamAccountId\":42}', '', 'full')",
        )
        .bind(db_occurred_at_lower_bound(range_start))
        .execute(&state.pool)
        .await
        .expect("insert abort fixture");

        let pause = TimelineMaterializationPause {
            range_start: format_utc_iso(range_start),
            range_end: format_utc_iso(range_end),
            upstream_account_id: Some(42),
            include_live: false,
            entered: Arc::new(tokio::sync::Notify::new()),
            release: Arc::new(tokio::sync::Notify::new()),
            token: Arc::new(StdMutex::new(None)),
        };
        *INVOCATION_TIMELINE_TEST_MATERIALIZATION_PAUSE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(pause.clone());
        let task = tokio::spawn(fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(range_start),
                to: format_utc_iso(range_end),
                upstream_account_id: Some(42),
                include_live: Some(false),
                limit: None,
                cursor: None,
                as_of: None,
            }),
        ));
        pause.entered.notified().await;
        let token = pause
            .token
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .expect("materialization pause should publish a token");
        task.abort();
        task.await.expect_err("fetch task should be aborted");
        *INVOCATION_TIMELINE_TEST_MATERIALIZATION_PAUSE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;

        assert!(
            load_timeline_snapshot(
                &token,
                &format_utc_iso(range_start),
                &format_utc_iso(range_end),
                Some(42),
                false,
            )
            .is_err()
        );
        for _ in 0..20 {
            let row_count: i64 = sqlx::query_scalar(&format!(
                "SELECT COUNT(*) FROM {INVOCATION_TIMELINE_SNAPSHOT_TABLE} WHERE snapshot_token = ?1"
            ))
            .bind(&token)
            .fetch_one(&state.pool)
            .await
            .expect("count aborted fetch snapshot rows");
            if row_count == 0 {
                state.pool.close().await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("aborted fetch snapshot rows were not cleaned up");
    }

    #[tokio::test]
    async fn snapshot_publication_executes_multiple_insert_batches() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let range_start = at(20_000);
        let range_end = at(21_000);
        for index in 0..=100 {
            sqlx::query(
                "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES (?1, ?2, 'proxy', 'success', 100, '{\"upstreamAccountId\":42}', '', 'full')",
            )
            .bind(format!("batched-{index:03}"))
            .bind(db_occurred_at_lower_bound(range_start + chrono::Duration::seconds(index)))
            .execute(&state.pool)
            .await
            .expect("insert batched publication fixture");
        }

        let Json(response) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(range_start),
                to: format_utc_iso(range_end),
                upstream_account_id: Some(42),
                include_live: Some(false),
                limit: Some(200),
                cursor: None,
                as_of: None,
            }),
        )
        .await
        .expect("fetch batched publication fixture");
        assert!(!response.has_more);
        assert_eq!(response.total, 101);
        assert_eq!(response.records.len(), 101);
        assert_eq!(
            response
                .records
                .first()
                .map(|record| record.invoke_id.as_str()),
            Some("batched-000")
        );
        assert_eq!(
            response
                .records
                .last()
                .map(|record| record.invoke_id.as_str()),
            Some("batched-100")
        );
        state.pool.close().await;
    }

    #[tokio::test]
    async fn snapshot_publication_waits_for_file_backed_writer_with_busy_timeout() {
        let db_path = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-timeline-contention-{}-{}.sqlite",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        ));
        let db_url = format!("sqlite://{}", db_path.display());
        let options = db_url
            .parse::<SqliteConnectOptions>()
            .expect("parse sqlite URL")
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(1));
        let schema_pool = sqlx::SqlitePool::connect_with(options.clone())
            .await
            .expect("connect schema pool");
        ensure_timeline_snapshot_table(&schema_pool)
            .await
            .expect("create snapshot table");
        schema_pool.close().await;

        let mut lock_connection = SqliteConnection::connect_with(&options)
            .await
            .expect("connect lock database");
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut lock_connection)
            .await
            .expect("acquire writer lock");

        let rows = (0..101)
            .map(|index| TimelineSnapshotInsertRow {
                invoke_id: format!("contention-{index:03}"),
                occurred_at: format!("2026-01-01T00:00:{index:02}.000Z"),
                record_id: index,
                is_runtime: 0,
                is_in_flight: 0,
                payload: "{}".to_string(),
            })
            .collect::<Vec<_>>();
        let mut blocked_connection = SqliteConnection::connect_with(&options)
            .await
            .expect("connect blocked publishing database");
        let blocked_begin = tokio::time::timeout(
            Duration::from_millis(100),
            blocked_connection.begin_with("BEGIN IMMEDIATE"),
        )
        .await;
        assert!(
            blocked_begin.is_err(),
            "publisher must remain blocked while another writer owns the database"
        );
        drop(blocked_begin);
        blocked_connection
            .close()
            .await
            .expect("close blocked publishing connection");
        sqlx::query("COMMIT")
            .execute(&mut lock_connection)
            .await
            .expect("release writer lock");

        let mut publish_connection = SqliteConnection::connect_with(&options)
            .await
            .expect("connect publishing database");
        let mut transaction = publish_connection
            .begin_with("BEGIN IMMEDIATE")
            .await
            .expect("begin publication after writer release");
        insert_timeline_snapshot_records_on_connection(&mut transaction, "contention-token", &rows)
            .await
            .expect("publish contention rows");
        transaction.commit().await.expect("commit published rows");
        publish_connection
            .close()
            .await
            .expect("close publishing connection");

        let mut verify_connection = SqliteConnection::connect_with(&options)
            .await
            .expect("connect verification database");
        let row_count: i64 = sqlx::query_scalar(&format!(
            "SELECT COUNT(*) FROM {INVOCATION_TIMELINE_SNAPSHOT_TABLE} WHERE snapshot_token = 'contention-token'"
        ))
        .fetch_one(&mut verify_connection)
        .await
        .expect("count published contention rows");
        assert_eq!(row_count, 101);
        verify_connection
            .close()
            .await
            .expect("close verification connection");
        lock_connection
            .close()
            .await
            .expect("close lock connection");
        for suffix in ["", "-shm", "-wal"] {
            let path = std::path::PathBuf::from(format!("{}{}", db_path.display(), suffix));
            let _ = std::fs::remove_file(path);
        }
    }

    #[tokio::test]
    async fn account_hydration_scopes_snapshot_predicates_to_every_key() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let snapshot_id =
            sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(id) FROM codex_invocations")
                .fetch_one(&state.pool)
                .await
                .expect("read invocation snapshot watermark")
                .unwrap_or(0);
        let attempt_snapshot_id = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT MAX(id) FROM pool_upstream_request_attempts",
        )
        .fetch_one(&state.pool)
        .await
        .expect("read attempt snapshot watermark")
        .unwrap_or(0);
        let keys = [
            ("hydration-key-a", at(11_000)),
            ("hydration-key-b", at(11_001)),
            ("hydration-key-c", at(11_002)),
        ];
        let mut records = Vec::new();
        for (invoke_id, occurred_at) in keys {
            let mut record =
                crate::api::slices::invocations_and_summary::summary_projection_test_invocation();
            record.id = 99_200 + records.len() as i64;
            record.invoke_id = invoke_id.to_string();
            record.occurred_at = db_occurred_at_lower_bound(occurred_at);
            record.source = SOURCE_PROXY.to_string();
            record.upstream_account_id = Some(42);
            records.push(record);
            sqlx::query(
                "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES (?1, ?2, 'proxy', 'success', 100, '{\"upstreamAccountId\":99}', '', 'full')",
            )
            .bind(invoke_id)
            .bind(db_occurred_at_lower_bound(occurred_at))
            .execute(&state.pool)
            .await
            .expect("insert post-snapshot hydration row");
        }

        hydrate_timeline_accounts(
            &state.pool,
            &mut records,
            InvocationSourceScope::ProxyOnly,
            snapshot_id,
            attempt_snapshot_id,
        )
        .await
        .expect("hydrate snapshot records");
        assert!(
            records
                .iter()
                .all(|record| record.upstream_account_id == Some(42))
        );
        state.pool.close().await;
    }

    #[tokio::test]
    async fn endpoint_returns_cross_window_overlap_once_with_account_filter() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let range_start = at(86_400);
        let range_end = at(90_000);
        for (invoke_id, occurred_at, total_ms, account_id) in [
            ("cross", at(86_399), 2_000_i64, 42_i64),
            ("outside", at(86_399), 500_i64, 42_i64),
            ("inside", at(86_401), 100_i64, 99_i64),
        ] {
            sqlx::query(
                "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES (?1, ?2, 'proxy', 'success', ?3, ?4, '', 'full')",
            )
            .bind(invoke_id)
            .bind(db_occurred_at_lower_bound(occurred_at))
            .bind(total_ms)
            .bind(format!("{{\"upstreamAccountId\":{account_id}}}"))
            .execute(&state.pool)
            .await
            .expect("insert timeline fixture");
        }
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, payload, raw_response, detail_level) VALUES ('live-cross', ?1, 'proxy', 'running', ?2, '', 'full')",
        )
        .bind(db_occurred_at_lower_bound(at(86_399)))
        .bind(r#"{"upstreamAccountId":42}"#)
        .execute(&state.pool)
        .await
        .expect("insert live timeline fixture");
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, payload, raw_response, detail_level) VALUES ('live-future', ?1, 'proxy', 'running', ?2, '', 'full')",
        )
        .bind(db_occurred_at_lower_bound(at(90_001)))
        .bind(r#"{"upstreamAccountId":42}"#)
        .execute(&state.pool)
        .await
        .expect("insert future live timeline fixture");
        let Json(response) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(range_start),
                to: format_utc_iso(range_end),
                upstream_account_id: Some(42),
                include_live: None,
                limit: None,
                cursor: None,
                as_of: None,
            }),
        )
        .await
        .expect("fetch timeline fixture");
        assert!(!response.has_more);
        assert_eq!(response.total, 2);
        assert_eq!(response.records.len(), 2);
        assert_eq!(response.records[0].invoke_id, "cross");
        assert_eq!(response.records[0].t_total_ms, Some(2_000.0));
        assert_eq!(response.records[1].invoke_id, "live-cross");
        assert!(response.records[1].is_in_flight);
        assert_eq!(response.records[1].end_at, None);

        let Json(closed_response) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(range_start),
                to: format_utc_iso(range_end),
                upstream_account_id: Some(42),
                include_live: Some(false),
                limit: None,
                cursor: None,
                as_of: None,
            }),
        )
        .await
        .expect("fetch closed timeline fixture");
        assert_eq!(closed_response.total, 1);
        assert_eq!(closed_response.records.len(), 1);
        assert_eq!(closed_response.records[0].invoke_id, "cross");
        state.pool.close().await;
    }

    #[tokio::test]
    async fn account_filter_rejects_overflow_payload_and_uses_attempt_fallback() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let occurred_at = db_occurred_at_lower_bound(at(86_400));
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES ('overflow', ?1, 'proxy', 'success', 100, ?2, '', 'full')",
        )
        .bind(&occurred_at)
        .bind(r#"{"upstreamAccountId":"9223372036854775808","upstreamAccountName":"unsafe"}"#)
        .execute(&state.pool)
        .await
        .expect("insert overflow timeline fixture");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, endpoint, route_mode, attempt_index, distinct_account_index, same_account_retry_index, status, upstream_account_id) VALUES (1, 'overflow', ?1, '/v1/responses', 'pool', 1, 1, 0, 'success', 42)",
        )
        .bind(&occurred_at)
        .execute(&state.pool)
        .await
        .expect("insert attempt fallback fixture");
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES ('overflow-integer', ?1, 'proxy', 'success', 100, ?2, '', 'full')",
        )
        .bind(db_occurred_at_lower_bound(at(86_500)))
        .bind(r#"{"upstreamAccountId":9223372036854775808}"#)
        .execute(&state.pool)
        .await
        .expect("insert integer overflow timeline fixture");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, endpoint, route_mode, attempt_index, distinct_account_index, same_account_retry_index, status, upstream_account_id) VALUES (2, 'overflow-integer', ?1, '/v1/responses', 'pool', 1, 1, 0, 'success', 43)",
        )
        .bind(db_occurred_at_lower_bound(at(86_500)))
        .execute(&state.pool)
            .await
            .expect("insert integer attempt fallback fixture");
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES ('malformed', ?1, 'proxy', 'success', 100, '{bad-json', '', 'full')",
        )
        .bind(db_occurred_at_lower_bound(at(86_600)))
        .execute(&state.pool)
        .await
        .expect("insert malformed timeline fixture");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, endpoint, route_mode, attempt_index, distinct_account_index, same_account_retry_index, status, upstream_account_id) VALUES (3, 'malformed', ?1, '/v1/responses', 'pool', 1, 1, 0, 'success', 44)",
        )
        .bind(db_occurred_at_lower_bound(at(86_600)))
        .execute(&state.pool)
        .await
        .expect("insert malformed attempt fallback fixture");
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES ('fractional-account-name', ?1, 'proxy', 'success', 100, ?2, '', 'full')",
        )
        .bind(db_occurred_at_lower_bound(at(86_650)))
        .bind(r#"{"upstreamAccountId":1.5,"upstreamAccountName":"fractional"}"#)
        .execute(&state.pool)
        .await
        .expect("insert fractional account name fixture");
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES ('exact-max', ?1, 'proxy', 'success', 100, ?2, '', 'full')",
        )
        .bind(db_occurred_at_lower_bound(at(86_700)))
        .bind(r#"{"upstreamAccountId":9223372036854775807}"#)
        .execute(&state.pool)
        .await
        .expect("insert exact max timeline fixture");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, endpoint, route_mode, attempt_index, distinct_account_index, same_account_retry_index, status, upstream_account_id) VALUES (4, 'exact-max', ?1, '/v1/responses', 'pool', 1, 1, 0, 'success', 45)",
        )
        .bind(db_occurred_at_lower_bound(at(86_700)))
        .execute(&state.pool)
        .await
        .expect("insert exact max attempt fallback fixture");
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, payload, raw_response, detail_level) VALUES ('live-overflow', ?1, 'proxy', 'running', ?2, '', 'full')",
        )
        .bind(db_occurred_at_lower_bound(at(86_800)))
        .bind(r#"{"upstreamAccountId":"9223372036854775808"}"#)
        .execute(&state.pool)
        .await
        .expect("insert live overflow timeline fixture");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, endpoint, route_mode, attempt_index, distinct_account_index, same_account_retry_index, status, upstream_account_id) VALUES (5, 'live-overflow', ?1, '/v1/responses', 'pool', 1, 1, 0, 'requesting', 46)",
        )
        .bind(db_occurred_at_lower_bound(at(86_800)))
        .execute(&state.pool)
        .await
        .expect("insert live overflow attempt fallback fixture");
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES ('invalid-attempt-account', ?1, 'proxy', 'success', 100, ?2, '', 'full')",
        )
        .bind(db_occurred_at_lower_bound(at(86_900)))
        .bind(r#"{"upstreamAccountId":9223372036854775807}"#)
        .execute(&state.pool)
        .await
        .expect("insert invalid attempt account fixture");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, endpoint, route_mode, attempt_index, distinct_account_index, same_account_retry_index, status, upstream_account_id) VALUES (6, 'invalid-attempt-account', ?1, '/v1/responses', 'pool', 1, 1, 0, 'success', 9223372036854775807)",
        )
        .bind(db_occurred_at_lower_bound(at(86_900)))
        .execute(&state.pool)
        .await
        .expect("insert invalid attempt account fallback fixture");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, endpoint, route_mode, attempt_index, distinct_account_index, same_account_retry_index, status, upstream_account_id) VALUES (7, 'runtime-account-fallback', ?1, '/v1/responses', 'pool', 1, 1, 0, 'requesting', 47)",
        )
        .bind(db_occurred_at_lower_bound(at(86_860)))
        .execute(&state.pool)
        .await
        .expect("insert runtime account fallback fixture");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, endpoint, route_mode, attempt_index, distinct_account_index, same_account_retry_index, status, upstream_account_id) VALUES (0, 'runtime-nonpositive-fallback', ?1, '/v1/responses', 'pool', 1, 1, 0, 'requesting', 49)",
        )
        .bind(db_occurred_at_lower_bound(at(86_870)))
        .execute(&state.pool)
        .await
        .expect("insert non-positive runtime account fallback fixture");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, endpoint, route_mode, attempt_index, distinct_account_index, same_account_retry_index, status, upstream_account_id) VALUES (8, 'runtime-fractional-fallback', ?1, '/v1/responses', 'pool', 1, 1, 0, 'requesting', 49.5)",
        )
        .bind(db_occurred_at_lower_bound(at(86_871)))
        .execute(&state.pool)
        .await
        .expect("insert fractional runtime account fallback fixture");
        for (invoke_id, occurred_at) in [
            ("runtime-nonpositive-fallback", at(86_870)),
            ("runtime-fractional-fallback", at(86_871)),
        ] {
            sqlx::query(
                "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES (?1, ?2, 'proxy', 'success', 100, '{bad-json', '', 'full')",
            )
            .bind(invoke_id)
            .bind(db_occurred_at_lower_bound(occurred_at))
            .execute(&state.pool)
            .await
            .expect("insert malformed persisted account fallback fixture");
        }
        let mut live_overflow =
            crate::api::slices::invocations_and_summary::summary_projection_test_invocation();
        live_overflow.id = 99_002;
        live_overflow.invoke_id = "live-overflow".to_string();
        live_overflow.occurred_at = db_occurred_at_lower_bound(at(86_800));
        live_overflow.source = SOURCE_PROXY.to_string();
        live_overflow.status = Some("running".to_string());
        live_overflow.live_phase = Some("requesting".to_string());
        live_overflow.t_total_ms = None;
        live_overflow.upstream_account_id = Some(46);
        state.proxy_runtime_invocations.upsert(live_overflow);
        let mut runtime_invalid_account =
            crate::api::slices::invocations_and_summary::summary_projection_test_invocation();
        runtime_invalid_account.id = 99_003;
        runtime_invalid_account.invoke_id = "runtime-invalid-account".to_string();
        runtime_invalid_account.occurred_at = db_occurred_at_lower_bound(at(86_850));
        runtime_invalid_account.source = SOURCE_PROXY.to_string();
        runtime_invalid_account.status = Some("running".to_string());
        runtime_invalid_account.live_phase = Some("requesting".to_string());
        runtime_invalid_account.t_total_ms = None;
        runtime_invalid_account.upstream_account_id = Some(TIMELINE_SAFE_ACCOUNT_ID_EXCLUSIVE);
        state
            .proxy_runtime_invocations
            .upsert(runtime_invalid_account);
        let mut runtime_missing_account =
            crate::api::slices::invocations_and_summary::summary_projection_test_invocation();
        runtime_missing_account.id = 99_004;
        runtime_missing_account.invoke_id = "runtime-account-fallback".to_string();
        runtime_missing_account.occurred_at = db_occurred_at_lower_bound(at(86_860));
        runtime_missing_account.source = SOURCE_PROXY.to_string();
        runtime_missing_account.status = Some("running".to_string());
        runtime_missing_account.live_phase = Some("requesting".to_string());
        runtime_missing_account.t_total_ms = None;
        runtime_missing_account.upstream_account_id = None;
        state
            .proxy_runtime_invocations
            .upsert(runtime_missing_account);
        for (invoke_id, occurred_at, id) in [
            ("runtime-nonpositive-fallback", at(86_870), 99_006),
            ("runtime-fractional-fallback", at(86_871), 99_007),
        ] {
            let mut runtime_malformed_fallback =
                crate::api::slices::invocations_and_summary::summary_projection_test_invocation();
            runtime_malformed_fallback.id = id;
            runtime_malformed_fallback.invoke_id = invoke_id.to_string();
            runtime_malformed_fallback.occurred_at = db_occurred_at_lower_bound(occurred_at);
            runtime_malformed_fallback.source = SOURCE_PROXY.to_string();
            runtime_malformed_fallback.status = Some("running".to_string());
            runtime_malformed_fallback.live_phase = Some("requesting".to_string());
            runtime_malformed_fallback.t_total_ms = None;
            runtime_malformed_fallback.upstream_account_id = None;
            state
                .proxy_runtime_invocations
                .upsert(runtime_malformed_fallback);
        }
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, payload, raw_response, detail_level) VALUES ('runtime-valid-account', ?1, 'proxy', 'running', '{bad-json', '', 'full')",
        )
        .bind(db_occurred_at_lower_bound(at(86_880)))
        .execute(&state.pool)
        .await
        .expect("insert malformed durable runtime account fixture");
        let mut runtime_valid_account =
            crate::api::slices::invocations_and_summary::summary_projection_test_invocation();
        runtime_valid_account.id = 99_005;
        runtime_valid_account.invoke_id = "runtime-valid-account".to_string();
        runtime_valid_account.occurred_at = db_occurred_at_lower_bound(at(86_880));
        runtime_valid_account.source = SOURCE_PROXY.to_string();
        runtime_valid_account.status = Some("running".to_string());
        runtime_valid_account.live_phase = Some("requesting".to_string());
        runtime_valid_account.t_total_ms = None;
        runtime_valid_account.upstream_account_id = Some(48);
        runtime_valid_account.upstream_account_name = Some("runtime-valid".to_string());
        state
            .proxy_runtime_invocations
            .upsert(runtime_valid_account);

        let Json(response) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(at(86_000)),
                to: format_utc_iso(at(87_000)),
                upstream_account_id: Some(42),
                include_live: None,
                limit: None,
                cursor: None,
                as_of: None,
            }),
        )
        .await
        .expect("fetch overflow timeline fixture");
        assert_eq!(response.total, 1);
        assert_eq!(response.records[0].invoke_id, "overflow");
        assert_eq!(response.records[0].upstream_account_id, Some(42));
        assert_eq!(response.records[0].upstream_account_name, None);
        let Json(runtime_valid_response) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(at(86_000)),
                to: format_utc_iso(at(87_000)),
                upstream_account_id: Some(48),
                include_live: Some(true),
                limit: None,
                cursor: None,
                as_of: None,
            }),
        )
        .await
        .expect("fetch valid runtime account fixture");
        assert_eq!(runtime_valid_response.total, 1);
        assert_eq!(
            runtime_valid_response.records[0].invoke_id,
            "runtime-valid-account"
        );
        assert_eq!(
            runtime_valid_response.records[0].upstream_account_id,
            Some(48)
        );
        assert_eq!(
            runtime_valid_response.records[0]
                .upstream_account_name
                .as_deref(),
            Some("runtime-valid")
        );
        let Json(integer_response) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(at(86_000)),
                to: format_utc_iso(at(87_000)),
                upstream_account_id: Some(43),
                include_live: None,
                limit: None,
                cursor: None,
                as_of: None,
            }),
        )
        .await
        .expect("fetch integer overflow timeline fixture");
        assert_eq!(integer_response.total, 1);
        assert_eq!(integer_response.records[0].invoke_id, "overflow-integer");
        assert_eq!(integer_response.records[0].upstream_account_id, Some(43));
        let Json(malformed_response) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(at(86_000)),
                to: format_utc_iso(at(87_000)),
                upstream_account_id: Some(44),
                include_live: None,
                limit: None,
                cursor: None,
                as_of: None,
            }),
        )
        .await
        .expect("fetch malformed timeline fixture");
        assert_eq!(malformed_response.total, 1);
        assert_eq!(malformed_response.records[0].invoke_id, "malformed");
        assert_eq!(malformed_response.records[0].upstream_account_id, Some(44));
        let Json(exact_max_response) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(at(86_000)),
                to: format_utc_iso(at(87_000)),
                upstream_account_id: Some(45),
                include_live: None,
                limit: None,
                cursor: None,
                as_of: None,
            }),
        )
        .await
        .expect("fetch exact max timeline fixture");
        assert_eq!(exact_max_response.total, 1);
        assert_eq!(exact_max_response.records[0].invoke_id, "exact-max");
        assert_eq!(exact_max_response.records[0].upstream_account_id, Some(45));
        let Json(live_overflow_response) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(at(86_000)),
                to: format_utc_iso(at(87_000)),
                upstream_account_id: Some(46),
                include_live: None,
                limit: None,
                cursor: None,
                as_of: None,
            }),
        )
        .await
        .expect("fetch live overflow timeline fixture");
        assert_eq!(live_overflow_response.total, 1);
        assert_eq!(live_overflow_response.records.len(), 1);
        assert_eq!(live_overflow_response.records[0].invoke_id, "live-overflow");
        let Json(runtime_account_response) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(at(86_000)),
                to: format_utc_iso(at(87_000)),
                upstream_account_id: None,
                include_live: Some(true),
                limit: None,
                cursor: None,
                as_of: None,
            }),
        )
        .await
        .expect("fetch runtime account safety fixture");
        let runtime_invalid = runtime_account_response
            .records
            .iter()
            .find(|record| record.invoke_id == "runtime-invalid-account")
            .expect("runtime invalid account record is present");
        assert_eq!(runtime_invalid.upstream_account_id, None);
        let Json(runtime_fallback_response) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(at(86_000)),
                to: format_utc_iso(at(87_000)),
                upstream_account_id: Some(47),
                include_live: Some(true),
                limit: None,
                cursor: None,
                as_of: None,
            }),
        )
        .await
        .expect("fetch runtime account fallback fixture");
        assert_eq!(runtime_fallback_response.total, 1);
        assert_eq!(
            runtime_fallback_response.records[0].invoke_id,
            "runtime-account-fallback"
        );
        assert_eq!(
            runtime_fallback_response.records[0].upstream_account_id,
            Some(47)
        );
        let Json(unfiltered_response) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(at(86_000)),
                to: format_utc_iso(at(87_000)),
                upstream_account_id: None,
                include_live: Some(false),
                limit: None,
                cursor: None,
                as_of: None,
            }),
        )
        .await
        .expect("fetch unfiltered invalid account fixture");
        let invalid_attempt = unfiltered_response
            .records
            .iter()
            .find(|record| record.invoke_id == "invalid-attempt-account")
            .expect("invalid attempt record is present");
        assert_eq!(invalid_attempt.upstream_account_id, None);
        let fractional_account = unfiltered_response
            .records
            .iter()
            .find(|record| record.invoke_id == "fractional-account-name")
            .expect("fractional account record is present");
        assert_eq!(fractional_account.upstream_account_id, None);
        assert_eq!(fractional_account.upstream_account_name, None);
        let Json(malformed_attempt_response) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(at(86_000)),
                to: format_utc_iso(at(87_000)),
                upstream_account_id: None,
                include_live: Some(true),
                limit: None,
                cursor: None,
                as_of: None,
            }),
        )
        .await
        .expect("fetch malformed attempt fallback fixtures");
        for invoke_id in [
            "runtime-nonpositive-fallback",
            "runtime-fractional-fallback",
        ] {
            let record = malformed_attempt_response
                .records
                .iter()
                .find(|record| record.invoke_id == invoke_id)
                .expect("malformed attempt runtime record is present");
            assert_eq!(record.upstream_account_id, None);
        }
        for invoke_id in [
            "runtime-nonpositive-fallback",
            "runtime-fractional-fallback",
        ] {
            let record = unfiltered_response
                .records
                .iter()
                .find(|record| record.invoke_id == invoke_id)
                .expect("malformed persisted account record is present");
            assert_eq!(record.upstream_account_id, None);
        }
        let Json(malformed_persisted_filtered_response) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(at(86_000)),
                to: format_utc_iso(at(87_000)),
                upstream_account_id: Some(49),
                include_live: Some(false),
                limit: None,
                cursor: None,
                as_of: None,
            }),
        )
        .await
        .expect("fetch filtered malformed persisted account fixtures");
        assert!(
            malformed_persisted_filtered_response
                .records
                .iter()
                .all(|record| {
                    record.invoke_id != "runtime-nonpositive-fallback"
                        && record.invoke_id != "runtime-fractional-fallback"
                })
        );
        state.pool.close().await;
    }

    #[tokio::test]
    async fn stable_cursor_paginates_logical_invocations_without_mixing_snapshots() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        for (invoke_id, occurred_at) in [
            ("paged-first", at(10_000)),
            ("paged-retry", at(10_001)),
            ("paged-second", at(10_002)),
            ("paged-third", at(10_003)),
        ] {
            sqlx::query(
                "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES (?1, ?2, 'proxy', 'success', 100, '{\"upstreamAccountId\":42}', '', 'full')",
            )
            .bind(invoke_id)
            .bind(db_occurred_at_lower_bound(occurred_at))
            .execute(&state.pool)
            .await
            .expect("insert pagination fixture");
        }

        let base_query = || InvocationTimelineQuery {
            from: format_utc_iso(at(9_999)),
            to: format_utc_iso(at(10_010)),
            upstream_account_id: Some(42),
            include_live: Some(false),
            limit: Some(2),
            cursor: None,
            as_of: None,
        };
        let Json(first) = fetch_timeline(State(state.clone()), Query(base_query()))
            .await
            .expect("fetch first pagination page");
        assert!(first.has_more);
        let first_cursor = first.next_cursor.clone().expect("next cursor");
        assert_eq!(first.total, 4);
        assert_eq!(first.records.len(), 2);

        let Json(second) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                cursor: Some(first_cursor.clone()),
                as_of: Some(first.as_of.clone()),
                ..base_query()
            }),
        )
        .await
        .expect("fetch second pagination page");
        assert!(!second.has_more);
        assert_eq!(second.next_cursor, None);
        assert_eq!(second.as_of, first.as_of);
        assert_eq!(second.total, 4);

        let mismatched_window = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(at(9_998)),
                to: format_utc_iso(at(10_010)),
                cursor: Some(first_cursor),
                as_of: Some(first.as_of),
                ..base_query()
            }),
        )
        .await;
        assert!(mismatched_window.is_err());

        let mut keys = first
            .records
            .iter()
            .chain(second.records.iter())
            .map(|record| (record.invoke_id.clone(), record.occurred_at.clone()))
            .collect::<Vec<_>>();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), 4);

        let mut live =
            crate::api::slices::invocations_and_summary::summary_projection_test_invocation();
        live.id = 99_001;
        live.invoke_id = "paged-live".to_string();
        live.occurred_at = db_occurred_at_lower_bound(at(10_001));
        live.source = SOURCE_PROXY.to_string();
        live.status = Some("running".to_string());
        live.live_phase = Some("requesting".to_string());
        live.t_total_ms = None;
        live.upstream_account_id = Some(42);
        state.proxy_runtime_invocations.upsert(live);

        let live_query = || InvocationTimelineQuery {
            from: format_utc_iso(at(9_999)),
            to: format_utc_iso(at(10_010)),
            upstream_account_id: Some(42),
            include_live: Some(true),
            limit: Some(2),
            cursor: None,
            as_of: None,
        };
        let Json(live_first) = fetch_timeline(State(state.clone()), Query(live_query()))
            .await
            .expect("fetch interleaved live first page");
        let live_first_cursor = live_first.next_cursor.clone().expect("live next cursor");
        assert_eq!(live_first.total, 5);
        assert_eq!(
            live_first
                .records
                .iter()
                .map(|record| record.invoke_id.as_str())
                .collect::<Vec<_>>(),
            vec!["paged-first", "paged-live"]
        );
        let Json(live_second) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                cursor: Some(live_first_cursor),
                as_of: Some(live_first.as_of.clone()),
                ..live_query()
            }),
        )
        .await
        .expect("fetch interleaved live second page");
        assert_eq!(
            live_second
                .records
                .iter()
                .map(|record| record.invoke_id.as_str())
                .collect::<Vec<_>>(),
            vec!["paged-retry", "paged-second"]
        );
        let Json(live_third) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                cursor: live_second.next_cursor.clone(),
                as_of: Some(live_first.as_of),
                ..live_query()
            }),
        )
        .await
        .expect("fetch interleaved live third page");
        assert_eq!(
            live_third
                .records
                .iter()
                .map(|record| record.invoke_id.as_str())
                .collect::<Vec<_>>(),
            vec!["paged-third"]
        );
        assert!(!live_third.has_more);
        state.pool.close().await;
    }

    #[tokio::test]
    async fn snapshot_pagination_does_not_hydrate_runtime_from_later_rows() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES ('snapshot-terminal', ?1, 'proxy', 'success', 100, '{\"upstreamAccountId\":42}', '', 'full')",
        )
        .bind(db_occurred_at_lower_bound(at(10_001)))
        .execute(&state.pool)
        .await
        .expect("insert snapshot pagination fixture");

        let mut live =
            crate::api::slices::invocations_and_summary::summary_projection_test_invocation();
        live.id = 99_101;
        live.invoke_id = "snapshot-live".to_string();
        live.occurred_at = db_occurred_at_lower_bound(at(10_000));
        live.source = SOURCE_PROXY.to_string();
        live.status = Some("running".to_string());
        live.live_phase = Some("requesting".to_string());
        live.t_total_ms = None;
        live.upstream_account_id = Some(42);
        state.proxy_runtime_invocations.upsert(live);

        let query = || InvocationTimelineQuery {
            from: format_utc_iso(at(9_999)),
            to: format_utc_iso(at(10_010)),
            upstream_account_id: Some(42),
            include_live: Some(true),
            limit: Some(1),
            cursor: None,
            as_of: None,
        };
        let Json(first) = fetch_timeline(State(state.clone()), Query(query()))
            .await
            .expect("fetch snapshot first page");
        assert_eq!(first.records[0].invoke_id, "snapshot-live");
        let cursor = first.next_cursor.clone().expect("snapshot next cursor");

        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES ('snapshot-live', ?1, 'proxy', 'success', 100, '{\"upstreamAccountId\":99}', '', 'full')",
        )
        .bind(db_occurred_at_lower_bound(at(10_000)))
        .execute(&state.pool)
        .await
        .expect("insert later terminal row");
        sqlx::query(
            "UPDATE codex_invocations SET status = 'failed', t_total_ms = 900, payload = '{\"upstreamAccountId\":77}' WHERE invoke_id = 'snapshot-terminal'",
        )
        .execute(&state.pool)
        .await
        .expect("mutate snapshot row after first page");

        let Json(second) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                cursor: Some(cursor),
                as_of: Some(first.as_of),
                ..query()
            }),
        )
        .await
        .expect("fetch snapshot second page");
        assert!(!second.has_more);
        assert_eq!(second.total, 2);
        assert_eq!(second.records.len(), 1);
        assert_eq!(second.records[0].invoke_id, "snapshot-terminal");
        assert_eq!(second.records[0].status.as_deref(), Some("success"));
        assert_eq!(second.records[0].t_total_ms, Some(100.0));
        assert_eq!(second.records[0].upstream_account_id, Some(42));
        state.pool.close().await;
    }
}
