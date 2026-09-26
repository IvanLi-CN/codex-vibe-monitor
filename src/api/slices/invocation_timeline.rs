use super::*;
use anyhow::{Context, anyhow};
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, QueryBuilder, Sqlite};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

const INVOCATION_TIMELINE_PAGE_SIZE: i64 = 500;
const INVOCATION_TIMELINE_MAX_PAGE_SIZE: i64 = 2_000;
const INVOCATION_TIMELINE_MAX_DURATION_MS: f64 = 30.0 * 24.0 * 60.0 * 60.0 * 1_000.0;
const INVOCATION_TIMELINE_SNAPSHOT_TTL: Duration = Duration::from_secs(30 * 60);
const INVOCATION_TIMELINE_SNAPSHOT_CACHE_LIMIT: usize = 256;
const TIMELINE_SAFE_ACCOUNT_ID_EXCLUSIVE: i64 = 9_007_199_254_740_992;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TimelineCursor {
    as_of: String,
    occurred_at: Option<String>,
    invoke_id: Option<String>,
}

#[derive(Debug, Clone)]
struct TimelineSnapshotRecord {
    record: ApiInvocation,
    is_runtime: bool,
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
    records: Vec<TimelineSnapshotRecord>,
    expires_at: Instant,
}

static INVOCATION_TIMELINE_SNAPSHOTS: once_cell::sync::Lazy<
    StdMutex<HashMap<String, TimelineSnapshot>>,
> = once_cell::sync::Lazy::new(|| StdMutex::new(HashMap::new()));

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

fn timeline_record_order(left: &ApiInvocation, right: &ApiInvocation) -> std::cmp::Ordering {
    left.occurred_at
        .cmp(&right.occurred_at)
        .then_with(|| left.invoke_id.cmp(&right.invoke_id))
        .then_with(|| left.id.cmp(&right.id))
}

fn timeline_record_is_after_cursor(record: &ApiInvocation, cursor: &TimelineCursor) -> bool {
    let (Some(occurred_at), Some(invoke_id)) = (&cursor.occurred_at, &cursor.invoke_id) else {
        return true;
    };
    record.occurred_at > *occurred_at
        || (record.occurred_at == *occurred_at && record.invoke_id > *invoke_id)
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
    runtime_records: Vec<ApiInvocation>,
) -> Result<String, ApiError> {
    create_timeline_snapshot_with_records(
        snapshot_id,
        attempt_snapshot_id,
        range_start,
        range_end,
        upstream_account_id,
        include_live,
        InvocationSourceScope::All,
        runtime_records
            .into_iter()
            .map(|record| TimelineSnapshotRecord {
                record,
                is_runtime: true,
            })
            .collect(),
    )
}

fn create_timeline_snapshot_with_records(
    snapshot_id: i64,
    attempt_snapshot_id: i64,
    range_start: String,
    range_end: String,
    upstream_account_id: Option<i64>,
    include_live: bool,
    source_scope: InvocationSourceScope,
    records: Vec<TimelineSnapshotRecord>,
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
            records,
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

#[derive(Debug, Serialize)]
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

async fn materialize_timeline_snapshot_records(
    state: &AppState,
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
) -> Result<Vec<TimelineSnapshotRecord>, ApiError> {
    // Pages read this frozen set so mutable terminal rows cannot drift between cursors.
    let mut persisted_filters = filters.clone();
    persisted_filters.upstream_account_id = None;
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
        None,
    );
    query
        .push(" AND NOT EXISTS (SELECT 1 FROM codex_invocations AS duplicate WHERE duplicate.invoke_id = codex_invocations.invoke_id AND duplicate.occurred_at = codex_invocations.occurred_at AND duplicate.id > codex_invocations.id AND duplicate.id <= ")
        .push_bind(snapshot_id)
        .push(") ORDER BY occurred_at ASC, invoke_id ASC, id ASC");
    let mut persisted_records = query
        .build_query_as::<ApiInvocation>()
        .fetch_all(&state.pool)
        .await?;
    for record in &mut persisted_records {
        hydrate_api_invocation_blocked_binding(record);
    }
    hydrate_timeline_accounts(
        &state.pool,
        &mut persisted_records,
        source_scope,
        snapshot_id,
        attempt_snapshot_id,
    )
    .await?;
    persisted_records
        .retain(|record| runtime_record_matches_filters(record, filters, source_scope));

    let terminal_keys = persisted_records
        .iter()
        .filter(|record| !runtime_record_is_in_flight(record))
        .map(|record| (record.invoke_id.clone(), record.occurred_at.clone()))
        .collect::<HashSet<_>>();
    let mut runtime_records = if include_live {
        runtime_overlay_snapshot(state)
    } else {
        Vec::new()
    };
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
            && runtime_record_matches_filters(record, filters, source_scope)
    });
    hydrate_timeline_accounts(
        &state.pool,
        &mut runtime_records,
        source_scope,
        snapshot_id,
        attempt_snapshot_id,
    )
    .await?;
    runtime_records.retain(|record| runtime_record_matches_filters(record, filters, source_scope));
    runtime_records.retain(|record| {
        !terminal_keys.contains(&(record.invoke_id.clone(), record.occurred_at.clone()))
    });

    let mut merged = persisted_records
        .into_iter()
        .map(|record| {
            (
                (record.invoke_id.clone(), record.occurred_at.clone()),
                TimelineSnapshotRecord {
                    record,
                    is_runtime: false,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    for record in runtime_records {
        let key = (record.invoke_id.clone(), record.occurred_at.clone());
        merged
            .entry(key)
            .and_modify(|existing| {
                if !existing.is_runtime || record.id > existing.record.id {
                    *existing = TimelineSnapshotRecord {
                        record: record.clone(),
                        is_runtime: true,
                    };
                }
            })
            .or_insert(TimelineSnapshotRecord {
                record,
                is_runtime: true,
            });
    }
    let mut records = merged.into_values().collect::<Vec<_>>();
    records.sort_by(|left, right| timeline_record_order(&left.record, &right.record));
    Ok(records)
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
    let (as_of, snapshot) = if let Some(as_of) = params.as_of.as_deref() {
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
        let source_scope = resolve_default_source_scope(&state.pool).await?;
        let (snapshot_id, attempt_snapshot_id) =
            resolve_timeline_snapshot_watermarks(&state.pool, source_scope).await?;
        let records = materialize_timeline_snapshot_records(
            state.as_ref(),
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
        .await?;
        let as_of = create_timeline_snapshot_with_records(
            snapshot_id,
            attempt_snapshot_id,
            canonical_range_start.clone(),
            canonical_range_end.clone(),
            params.upstream_account_id,
            include_live,
            source_scope,
            records,
        )?;
        (
            as_of.clone(),
            load_timeline_snapshot(
                &as_of,
                &canonical_range_start,
                &canonical_range_end,
                params.upstream_account_id,
                include_live,
            )?,
        )
    };
    let mut merged = snapshot
        .records
        .iter()
        .filter(|entry| {
            cursor
                .as_ref()
                .is_none_or(|cursor| timeline_record_is_after_cursor(&entry.record, cursor))
        })
        .cloned()
        .collect::<Vec<_>>();
    let total = snapshot.records.len() as i64;
    let has_more = merged.len() > page_size as usize;
    merged.truncate(page_size as usize);
    let next_cursor = if has_more {
        merged
            .last()
            .map(|entry| {
                encode_timeline_cursor(&TimelineCursor {
                    as_of: as_of.clone(),
                    occurred_at: Some(entry.record.occurred_at.clone()),
                    invoke_id: Some(entry.record.invoke_id.clone()),
                })
            })
            .transpose()?
    } else {
        None
    };
    let timeline_records = merged
        .into_iter()
        .map(|entry| {
            let record = entry.record;
            let is_runtime = entry.is_runtime;
            let is_in_flight = runtime_record_is_in_flight(&record);
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
            let status = invocation_display_status_value(&record).map(str::to_string);
            InvocationTimelineRecord {
                id: record.id,
                invoke_id: record.invoke_id.clone(),
                occurred_at: parse_to_utc_datetime(&occurred_at)
                    .map(format_utc_iso_precise)
                    .unwrap_or(occurred_at),
                end_at,
                is_in_flight,
                status,
                live_phase: if is_runtime {
                    runtime_record_live_phase(&record).map(str::to_string)
                } else {
                    record.live_phase.clone()
                },
                first_token_ms: if is_runtime {
                    runtime_record_first_token_ms(&record)
                } else {
                    record
                        .first_token_ms
                        .filter(|value| value.is_finite() && *value >= 0.0)
                },
                t_total_ms: total_ms,
                pool_attempt_count: record.pool_attempt_count,
                upstream_account_id: record.upstream_account_id,
                upstream_account_name: record.upstream_account_name,
                failure_class: record.failure_class,
            }
        })
        .collect();

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
    let mut invalid_account_keys = HashSet::new();
    for record in records.iter_mut() {
        if record.upstream_account_id.is_some_and(|account_id| {
            account_id <= 0 || account_id >= TIMELINE_SAFE_ACCOUNT_ID_EXCLUSIVE
        }) {
            invalid_account_keys.insert((record.invoke_id.clone(), record.occurred_at.clone()));
            record.upstream_account_id = None;
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
                .fetch_all(pool)
                .await?,
        );
    }
    let fallbacks = rows
        .into_iter()
        .map(|row| ((row.invoke_id.clone(), row.occurred_at.clone()), row))
        .collect::<std::collections::HashMap<_, _>>();
    for record in records {
        if let Some(row) = fallbacks.get(&(record.invoke_id.clone(), record.occurred_at.clone())) {
            record.upstream_account_id = row.upstream_account_id;
            record.upstream_account_name = row.upstream_account_name.clone();
        }
        if invalid_account_keys.contains(&(record.invoke_id.clone(), record.occurred_at.clone()))
            && record.upstream_account_id.is_none()
        {
            record.upstream_account_id = sqlx::query_scalar(
                "SELECT upstream_account_id FROM pool_upstream_request_attempts WHERE invoke_id = ?1 AND occurred_at = ?2 AND id <= ?3 AND upstream_account_id IS NOT NULL AND upstream_account_id > 0 AND upstream_account_id < 9007199254740992 ORDER BY attempt_index DESC, id DESC LIMIT 1",
            )
            .bind(&record.invoke_id)
            .bind(&record.occurred_at)
            .bind(attempt_snapshot_id)
            .fetch_optional(pool)
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
    let attempt_snapshot_predicate = attempt_snapshot_id
        .map(|snapshot_id| format!(" AND attempt.id <= {snapshot_id}"))
        .unwrap_or_default();
    format!(
        "COALESCE(CASE WHEN {payload_is_valid} AND {valid_payload_id} THEN CAST({value} AS INTEGER) END, (SELECT attempt.upstream_account_id FROM pool_upstream_request_attempts attempt WHERE attempt.invoke_id = {invocation_ref}.invoke_id AND attempt.occurred_at = {invocation_ref}.occurred_at AND attempt.id > 0{attempt_snapshot_predicate} AND attempt.upstream_account_id IS NOT NULL AND attempt.upstream_account_id > 0 AND attempt.upstream_account_id < 9007199254740992 ORDER BY attempt.attempt_index DESC, attempt.id DESC LIMIT 1))"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

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
        .bind(r#"{"upstreamAccountId":"9223372036854775808"}"#)
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
